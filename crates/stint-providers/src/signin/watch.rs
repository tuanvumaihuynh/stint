//! Runs a sign-in in the background with its input and output piped, for the window:
//! it reports the sign-in link and Claude Code's code prompt, forwards a pasted code,
//! and can be cancelled.

use std::io::{Read, Write as _};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use super::{Enrolled, SignIn};
use crate::error::ProviderError;

// How often the watcher checks whether the CLI has exited.
const POLL: Duration = Duration::from_millis(100);
// How long the outcome waits for the CLI's last output to be read. A process the CLI left
// behind can hold its output open, so the wait is bounded.
const DRAIN: Duration = Duration::from_secs(1);
// Keeps the scanned output bounded however much the CLI prints.
const MAX_SCANNED: usize = 64 * 1024;
const CODE_PROMPT: &str = "Paste code";

/// What a running sign-in reports, in order: maybe a link, maybe a code prompt, then
/// exactly one [`SignInEvent::Finished`].
#[derive(Debug)]
pub enum SignInEvent {
    /// The sign-in page, to offer opening it again.
    Link(String),
    /// Claude Code takes a code pasted from the sign-in page.
    CodePrompt,
    /// The new login, or why there is none.
    Finished(Result<Enrolled, ProviderError>),
}

type Listener = Arc<dyn Fn(SignInEvent) + Send + Sync>;

/// A sign-in running in the background. Dropping it cancels the sign-in.
pub struct Watched {
    child: Arc<Mutex<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    cancelled: Arc<AtomicBool>,
}

impl SignIn {
    /// Starts the CLI sign-in and reports its progress to `listener`, from background
    /// threads.
    pub fn watch(
        self,
        listener: impl Fn(SignInEvent) + Send + Sync + 'static,
    ) -> Result<Watched, ProviderError> {
        let command = self.command()?;
        self.watch_command(command, Arc::new(listener))
    }

    fn watch_command(
        self,
        mut command: Command,
        listener: Listener,
    ) -> Result<Watched, ProviderError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let program = command.get_program().to_owned();
        let mut child = command.spawn().map_err(ProviderError::io(program))?;
        let stdin = child.stdin.take();
        let scanner = Arc::new(Mutex::new(Scanner::default()));
        // Each reader holds a sender until its stream ends.
        let (drained, outputs_done) = mpsc::channel::<()>();
        if let Some(stdout) = child.stdout.take() {
            read_output(stdout, scanner.clone(), listener.clone(), drained.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            read_output(stderr, scanner, listener.clone(), drained);
        }

        let child = Arc::new(Mutex::new(child));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (waited, was_cancelled) = (child.clone(), cancelled.clone());
        thread::spawn(move || {
            let result = match wait(&waited) {
                _ if was_cancelled.load(Ordering::SeqCst) => Err(ProviderError::SignInCancelled),
                Ok(status) if status.success() => self.finish(),
                Ok(status) => Err(ProviderError::SignInFailed {
                    code: status.code(),
                }),
                Err(err) => Err(err),
            };
            // Report the link and prompt before the outcome.
            outputs_done.recv_timeout(DRAIN).ok();
            // Removes the attempt's files before reporting the outcome.
            drop(self);
            listener(SignInEvent::Finished(result));
        });

        Ok(Watched {
            child,
            stdin: Mutex::new(stdin),
            cancelled,
        })
    }
}

impl Watched {
    /// Sends a code pasted from the sign-in page to Claude Code.
    pub fn submit_code(&self, code: &str) -> Result<(), ProviderError> {
        let mut stdin = self.stdin.lock().unwrap_or_else(PoisonError::into_inner);
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| ProviderError::Unsupported("the sign-in takes no code".into()))?;
        let line = format!("{}\n", code.trim());
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(ProviderError::io("sign-in input"))
    }

    /// Stops the sign-in. The listener then gets [`ProviderError::SignInCancelled`].
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        // Fails only when the CLI already exited.
        child.kill().ok();
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let finished = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_wait()
            .is_ok_and(|status| status.is_some());
        if !finished {
            self.cancel();
        }
    }
}

fn wait(child: &Mutex<Child>) -> Result<std::process::ExitStatus, ProviderError> {
    loop {
        let status = child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_wait()
            .map_err(ProviderError::io("sign-in CLI"))?;
        if let Some(status) = status {
            return Ok(status);
        }
        thread::sleep(POLL);
    }
}

fn read_output(
    mut output: impl Read + Send + 'static,
    scanner: Arc<Mutex<Scanner>>,
    listener: Listener,
    drained: mpsc::Sender<()>,
) {
    thread::spawn(move || {
        // Dropped when the stream ends, which tells the watcher this output is read.
        let _drained = drained;
        let mut buffer = [0_u8; 4096];
        while let Ok(read) = output.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buffer[..read]);
            let events = scanner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .feed(&text);
            for event in events {
                listener(event);
            }
        }
    });
}

/// Finds the first sign-in link and the code prompt in the CLI's output, each once.
#[derive(Default)]
struct Scanner {
    seen: String,
    link_found: bool,
    prompt_found: bool,
}

impl Scanner {
    fn feed(&mut self, text: &str) -> Vec<SignInEvent> {
        if self.seen.len() < MAX_SCANNED {
            self.seen.push_str(text);
        }
        let mut events = Vec::new();
        if !self.link_found
            && let Some(start) = self.seen.find("https://")
            && let Some(length) = self.seen[start..].find(char::is_whitespace)
        {
            self.link_found = true;
            events.push(SignInEvent::Link(
                self.seen[start..start + length].to_owned(),
            ));
        }
        if !self.prompt_found && self.seen.contains(CODE_PROMPT) {
            self.prompt_found = true;
            events.push(SignInEvent::CodePrompt);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::super::CliProvider;

    use super::*;
    use crate::credential::Credential;

    const TIMEOUT: Duration = Duration::from_secs(10);

    fn names(events: &[SignInEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                SignInEvent::Link(link) => format!("link {link}"),
                SignInEvent::CodePrompt => "prompt".to_owned(),
                SignInEvent::Finished(Ok(_)) => "ok".to_owned(),
                SignInEvent::Finished(Err(err)) => format!("error {err}"),
            })
            .collect()
    }

    /// Runs `script` as the sign-in CLI, with `$HOME_DIR` set to the attempt's home.
    fn run(
        script: &str,
        drive: impl FnOnce(&Watched, &mpsc::Receiver<SignInEvent>) -> Vec<SignInEvent>,
    ) -> Vec<SignInEvent> {
        let data = tempfile::TempDir::new().unwrap();
        let attempt = SignIn::start(CliProvider::Codex, data.path()).unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).env("HOME_DIR", &attempt.home);
        let (sender, events) = mpsc::channel();
        let sender = Mutex::new(sender);
        let watched = attempt
            .watch_command(
                command,
                Arc::new(move |event| {
                    sender.lock().unwrap().send(event).ok();
                }),
            )
            .unwrap();
        let mut seen = drive(&watched, &events);
        let deadline = Instant::now() + TIMEOUT;
        while !matches!(seen.last(), Some(SignInEvent::Finished(_))) {
            let left = deadline.saturating_duration_since(Instant::now());
            seen.push(
                events
                    .recv_timeout(left)
                    .expect("the sign-in never finished"),
            );
        }
        seen
    }

    fn until_prompt(events: &mpsc::Receiver<SignInEvent>) -> Vec<SignInEvent> {
        let mut seen = Vec::new();
        while !matches!(seen.last(), Some(SignInEvent::CodePrompt)) {
            seen.push(events.recv_timeout(TIMEOUT).unwrap());
        }
        seen
    }

    #[test]
    fn reports_the_link_then_the_new_login() {
        let auth = r#"{"tokens":{"id_token":"h.e30.s","access_token":"a","refresh_token":"r","account_id":"acct"}}"#;
        let script = format!(
            "echo 'Starting local login server on http://localhost:1455.' >&2
             echo 'If your browser did not open, visit:' >&2
             echo 'https://auth.example/authorize?x=1' >&2
             printf '%s' '{auth}' > \"$HOME_DIR/auth.json\""
        );
        let events = run(&script, |_, _| Vec::new());
        assert_eq!(
            names(&events),
            ["link https://auth.example/authorize?x=1", "ok"]
        );
        let Some(SignInEvent::Finished(Ok(enrolled))) = events.last() else {
            unreachable!()
        };
        assert!(
            matches!(&enrolled.credential, Credential::Codex(login) if login.account_id == "acct")
        );
    }

    #[test]
    fn forwards_a_pasted_code() {
        // Echoes the code back as the exit status's reason.
        let script = "printf 'Visit https://claude.example/code\\nPaste code here if prompted > '
                      read code
                      [ \"$code\" = 'abc#state' ] && exit 3
                      exit 1";
        let events = run(script, |watched, events| {
            let seen = until_prompt(events);
            watched.submit_code("  abc#state \n").unwrap();
            seen
        });
        assert_eq!(
            names(&events),
            [
                "link https://claude.example/code",
                "prompt",
                "error the sign-in did not finish (exit code 3)",
            ]
        );
    }

    #[test]
    fn cancel_stops_the_cli() {
        let started = Instant::now();
        let events = run("echo 'Paste code'; sleep 30", |watched, events| {
            let seen = until_prompt(events);
            watched.cancel();
            seen
        });
        assert_eq!(
            names(&events),
            ["prompt", "error the sign-in was cancelled"]
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn scanner_waits_for_the_whole_link() {
        let mut scanner = Scanner::default();
        assert!(scanner.feed("open https://exa").is_empty());
        assert_eq!(
            names(&scanner.feed("mple.com/a b\n")),
            ["link https://example.com/a"]
        );
        assert!(scanner.feed("https://other.example\n").is_empty());
    }
}

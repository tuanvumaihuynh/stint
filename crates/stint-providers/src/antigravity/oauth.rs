//! Signs a Google account in for Antigravity in the browser: an OAuth installed-app flow
//! with PKCE, answered on a loopback port.

use std::fs::File;
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

use super::{CLIENT_ID, SCOPES};
use crate::error::ProviderError;
use crate::http;

const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
// The port and path registered for Antigravity's client; Google only redirects there.
pub(super) const PORT: u16 = 51121;
const CALLBACK_PATH: &str = "/oauth-callback";
// How often the listener checks for a browser or a cancel.
const POLL: Duration = Duration::from_millis(100);
// A browser that connects but never sends its request is dropped after this.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST: usize = 16 * 1024;

const SIGNED_IN_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Stint</title>\
    <body style=\"font:15px -apple-system,sans-serif;text-align:center;padding-top:20vh\">\
    <h2>Signed in to Stint</h2><p>You can close this tab and return to Stint.</p>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Stint</title>\
    <body style=\"font:15px -apple-system,sans-serif;text-align:center;padding-top:20vh\">\
    <h2>Sign-in did not finish</h2><p>Return to Stint and try again.</p>";

/// A browser sign-in waiting for Google to send the user back.
pub struct Authorization {
    listeners: Vec<TcpListener>,
    state: String,
    pub(super) verifier: String,
    url: String,
}

impl Authorization {
    /// Starts listening for the callback and builds the sign-in page's address. Open it in
    /// the browser, then [`Authorization::wait`].
    pub fn start() -> Result<Self, ProviderError> {
        // `localhost` can resolve to either address; IPv6 is a bonus where available.
        let v4 = TcpListener::bind(("127.0.0.1", PORT)).map_err(|err| match err.kind() {
            ErrorKind::AddrInUse => ProviderError::CallbackPortBusy(PORT),
            _ => ProviderError::io(format!("127.0.0.1:{PORT}"))(err),
        })?;
        let mut listeners = vec![v4];
        if let Ok(v6) = TcpListener::bind(("::1", PORT)) {
            listeners.push(v6);
        }
        for listener in &listeners {
            listener
                .set_nonblocking(true)
                .map_err(ProviderError::io("sign-in listener"))?;
        }
        let verifier = random_token()?;
        let state = random_token()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let query = http::form(&[
            ("client_id", CLIENT_ID),
            ("redirect_uri", &redirect_uri()),
            ("response_type", "code"),
            ("scope", &SCOPES.join(" ")),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ]);
        Ok(Self {
            listeners,
            state,
            verifier,
            url: format!("{AUTHORIZE_URL}?{query}"),
        })
    }

    /// The Google sign-in page to open.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits until the browser comes back with a code, `cancelled` is set, or `limit`
    /// passes. Requests to other paths, or with someone else's state, are turned away.
    pub fn wait(&self, cancelled: &AtomicBool, limit: Duration) -> Result<String, ProviderError> {
        let deadline = Instant::now() + limit;
        loop {
            if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Err(ProviderError::SignInCancelled);
            }
            for listener in &self.listeners {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(outcome) = self.answer(stream) {
                            return outcome;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {}
                    Err(err) => return Err(ProviderError::io("sign-in listener")(err)),
                }
            }
            thread::sleep(POLL);
        }
    }

    /// Reads one request. `None` when it was not the callback, so waiting goes on.
    fn answer(&self, mut stream: TcpStream) -> Option<Result<String, ProviderError>> {
        let request = read_request(&mut stream)?;
        let Some(query) = callback_query(&request) else {
            respond(&mut stream, "404 Not Found", "");
            return None;
        };
        let field = |name: &str| {
            query.split('&').find_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                (key == name).then(|| http::decode(value)).flatten()
            })
        };
        if field("state").as_deref() != Some(self.state.as_str()) {
            respond(&mut stream, "400 Bad Request", FAILED_PAGE);
            return None;
        }
        if let Some(code) = field("code").filter(|code| !code.is_empty()) {
            respond(&mut stream, "200 OK", SIGNED_IN_PAGE);
            return Some(Ok(code));
        }
        respond(&mut stream, "200 OK", FAILED_PAGE);
        Some(Err(match field("error").as_deref() {
            Some("access_denied") => ProviderError::SignInCancelled,
            _ => ProviderError::SignInRefused("Google did not return a sign-in code"),
        }))
    }
}

pub(super) fn redirect_uri() -> String {
    format!("http://localhost:{PORT}{CALLBACK_PATH}")
}

/// The query of `GET /oauth-callback?…`, or `None` for any other request.
fn callback_query(request: &str) -> Option<&str> {
    let target = request.strip_prefix("GET ")?.split(' ').next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (path == CALLBACK_PATH).then_some(query)
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    while !request.windows(4).any(|end| end == b"\r\n\r\n") && request.len() < MAX_REQUEST {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let line_end = request.iter().position(|&byte| byte == b'\r')?;
    String::from_utf8(request[..line_end].to_vec()).ok()
}

fn respond(stream: &mut TcpStream, status: &str, page: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    // The browser may already be gone; the outcome stands either way.
    stream.write_all(response.as_bytes()).ok();
}

/// 32 random bytes from the system, URL-safe.
fn random_token() -> Result<String, ProviderError> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(ProviderError::io("/dev/urandom"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_query_only_matches_the_callback() {
        assert_eq!(
            callback_query("GET /oauth-callback?code=a&state=b HTTP/1.1"),
            Some("code=a&state=b")
        );
        assert_eq!(callback_query("GET /oauth-callback HTTP/1.1"), Some(""));
        assert_eq!(callback_query("GET /favicon.ico HTTP/1.1"), None);
        assert_eq!(callback_query("POST /oauth-callback?code=a HTTP/1.1"), None);
    }

    #[test]
    fn tokens_are_random_and_url_safe() {
        let (a, b) = (random_token().unwrap(), random_token().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
    }

    // One test owns the fixed port, so the listener cases run in order.
    #[test]
    fn the_callback_returns_the_code_and_turns_others_away() {
        let attempt = Authorization::start().unwrap();
        assert!(attempt.url().contains("code_challenge_method=S256"));
        assert!(matches!(
            Authorization::start(),
            Err(ProviderError::CallbackPortBusy(PORT))
        ));

        let state = attempt.state.clone();
        let browser = thread::spawn(move || {
            let send = |path: String| {
                let mut stream = TcpStream::connect(("127.0.0.1", PORT)).unwrap();
                write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut answer = String::new();
                stream.read_to_string(&mut answer).ok();
                answer
            };
            assert!(send("/favicon.ico".into()).starts_with("HTTP/1.1 404"));
            assert!(send("/oauth-callback?code=x&state=forged".into()).starts_with("HTTP/1.1 400"));
            send(format!("/oauth-callback?code=4%2F0Ab&state={state}"))
        });
        let code = attempt
            .wait(&AtomicBool::new(false), Duration::from_secs(10))
            .unwrap();
        assert_eq!(code, "4/0Ab");
        assert!(browser.join().unwrap().contains("Signed in to Stint"));

        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            attempt.wait(&cancelled, Duration::from_secs(10)),
            Err(ProviderError::SignInCancelled)
        ));
    }
}

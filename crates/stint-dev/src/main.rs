//! Adds and removes real accounts from the terminal until the desktop window can:
//! `just add-account claude Personal`. A development tool; it is not shipped.
//!
//! Adding runs the provider's CLI sign-in in a private directory, with this terminal as
//! its input and output, so its browser link and code prompt work as usual. Antigravity
//! has no CLI sign-in Stint can isolate, so it signs in through the browser directly.
//! `usage` prints an account's live quota windows, never its login.

use std::process::{Command, ExitCode};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Context as _, bail};
use jiff::Timestamp;
use stint_core::{NewAccount, ProviderId, QuotaStore, SqliteStore};
use stint_providers::{
    Accounts, CliProvider, Enrolled, SignIn, Vault, antigravity, new_credential_key,
};

const USAGE: &str = "usage: stint-dev add-account <claude|codex|antigravity> <name>\n       \
    stint-dev remove-account <claude|codex|antigravity> <name>\n       \
    stint-dev usage <claude|codex|antigravity> <name>";
// How long a browser sign-in may take.
const SIGN_IN_LIMIT: Duration = Duration::from_mins(5);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("stint: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> anyhow::Result<()> {
    let [command, provider, name] = args else {
        bail!("{USAGE}");
    };
    let provider: ProviderId = provider.parse().context(USAGE)?;
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "{USAGE}");
    match command.as_str() {
        "add-account" => add(provider, name),
        "remove-account" => remove(provider, name),
        "usage" => usage(provider, name),
        _ => bail!("{USAGE}"),
    }
}

fn open_store() -> anyhow::Result<SqliteStore> {
    let path = stint_core::database_path().context("find the home directory")?;
    SqliteStore::open(&path).with_context(|| format!("open database {}", path.display()))
}

fn add(provider: ProviderId, name: &str) -> anyhow::Result<()> {
    let mut store = open_store()?;
    let taken = store
        .accounts()?
        .iter()
        .any(|account| account.provider == provider && account.label == name);
    anyhow::ensure!(!taken, "a {provider} account named {name:?} already exists");

    let enrolled = match CliProvider::of(provider) {
        Some(cli) => sign_in_with_cli(cli, provider)?,
        None => sign_in_in_browser()?,
    };

    let key = new_credential_key(provider);
    let vault = Vault::new();
    vault
        .save(&key, &enrolled.credential)
        .context("save the login to the Keychain")?;
    let account = NewAccount {
        provider,
        label: name.to_owned(),
        email: enrolled.email,
        plan: enrolled.plan,
        credential_key: key.clone(),
    };
    if let Err(err) = store.insert_account(&account, Timestamp::now()) {
        vault.delete(&key).ok();
        return Err(err).context("save the account");
    }
    let details: Vec<&str> = [account.email.as_deref(), account.plan.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    println!(
        "Added {provider} account {name:?} ({}).",
        details.join(", ")
    );
    Ok(())
}

fn sign_in_with_cli(cli: CliProvider, provider: ProviderId) -> anyhow::Result<Enrolled> {
    let data_dir = stint_core::data_dir().context("find the home directory")?;
    let attempt = SignIn::start(cli, &data_dir)?;
    println!("Signing in to {provider} in your browser. Finish there, then come back.");
    let status = attempt
        .command()?
        .status()
        .with_context(|| format!("run the {provider} sign-in"))?;
    anyhow::ensure!(
        status.success(),
        "the {provider} sign-in did not finish ({status})"
    );
    attempt.finish().context("read the new login")
}

fn sign_in_in_browser() -> anyhow::Result<Enrolled> {
    let authorization = antigravity::Authorization::start()?;
    println!("Signing in to Antigravity in your browser. Finish there, then come back.");
    let opened = Command::new("/usr/bin/open")
        .arg(authorization.url())
        .status()
        .is_ok_and(|status| status.success());
    if !opened {
        println!("Open this page to sign in:\n{}", authorization.url());
    }
    let code = authorization.wait(&AtomicBool::new(false), SIGN_IN_LIMIT)?;
    let client = stint_providers::http_client()?;
    runtime()?
        .block_on(antigravity::enroll(&client, &authorization, &code))
        .context("finish the Antigravity sign-in")
}

fn usage(provider: ProviderId, name: &str) -> anyhow::Result<()> {
    let store = open_store()?;
    let account = store
        .accounts()?
        .into_iter()
        .find(|account| account.provider == provider && account.label == name)
        .with_context(|| format!("no {provider} account named {name:?}"))?;
    let data_dir = stint_core::data_dir().context("find the home directory")?;
    let accounts = Accounts::new(stint_providers::http_client()?, Vault::new(), &data_dir);
    let windows = runtime()?
        .block_on(accounts.fetch(&account.credential_key))
        .with_context(|| format!("read the quota of {provider} account {name:?}"))?;
    if windows.is_empty() {
        println!("No quota windows reported.");
    }
    for window in windows {
        let scope = window.scope.as_deref().unwrap_or("plan");
        let resets = window
            .resets_at
            .map_or_else(String::new, |at| format!(", resets {at}"));
        println!("{scope} {}: {} used{resets}", window.kind, window.used);
    }
    Ok(())
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start the async runtime")
}

fn remove(provider: ProviderId, name: &str) -> anyhow::Result<()> {
    let mut store = open_store()?;
    let account = store
        .accounts()?
        .into_iter()
        .find(|account| account.provider == provider && account.label == name)
        .with_context(|| format!("no {provider} account named {name:?}"))?;
    Vault::new()
        .delete(&account.credential_key)
        .context("remove the login from the Keychain")?;
    store.remove_account(account.id)?;
    println!("Removed {provider} account {name:?}.");
    Ok(())
}

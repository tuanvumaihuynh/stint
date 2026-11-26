use std::io;
use std::path::PathBuf;

use jiff::SignedDuration;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("not signed in: no credentials found")]
    AuthMissing,
    #[error("the provider rejected the access token")]
    TokenExpired,
    #[error("the sign-in expired; sign in to the account again")]
    SignInExpired,
    #[error("rate limited by the provider")]
    RateLimited { retry_after: Option<SignedDuration> },
    #[error("the macOS keychain is locked")]
    KeychainLocked,
    #[error("keychain: {0}")]
    Keychain(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{0} is not installed or not on PATH")]
    CliMissing(&'static str),
    #[error("the sign-in was cancelled")]
    SignInCancelled,
    #[error("the sign-in did not finish ({})", exit_status(*.code))]
    SignInFailed { code: Option<i32> },
    #[error("sign-in is unsupported: {0}")]
    Unsupported(String),
    #[error("the account is not set up for this: {0}")]
    AccountSetup(String),
    #[error("port {0} is in use by another app's sign-in; close it and try again")]
    CallbackPortBusy(u16),
    #[error("the sign-in was refused: {0}")]
    SignInRefused(&'static str),
    #[error("usage answer is for another account than the signed-in one")]
    AccountMismatch,
    #[error("provider answered HTTP {status}")]
    Status { status: u16 },
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("decode response: {0}")]
    Decode(#[from] serde_json::Error),
}

impl ProviderError {
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}

fn exit_status(code: Option<i32>) -> String {
    code.map_or_else(
        || "stopped by a signal".to_owned(),
        |code| format!("exit code {code}"),
    )
}

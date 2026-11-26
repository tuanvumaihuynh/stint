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
    #[error("sign-in is unsupported: {0}")]
    Unsupported(String),
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

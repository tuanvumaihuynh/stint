use std::io;
use std::path::PathBuf;

use jiff::SignedDuration;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("not signed in: no credentials found")]
    AuthMissing,
    #[error("access token expired; run the provider's CLI to renew it")]
    TokenExpired,
    #[error("rate limited by the provider")]
    RateLimited { retry_after: Option<SignedDuration> },
    #[error("the macOS keychain is locked")]
    KeychainLocked,
    #[error("read keychain: {0}")]
    Keychain(String),
    #[error("read credentials {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("provider answered HTTP {status}")]
    Status { status: u16 },
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("decode response: {0}")]
    Decode(#[from] serde_json::Error),
}

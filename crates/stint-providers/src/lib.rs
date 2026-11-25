mod claude;
mod error;
mod http;
mod keychain;

use std::future::Future;

use stint_core::{ProviderId, QuotaWindow};

pub use claude::ClaudeProvider;
pub use error::ProviderError;
pub use http::client as http_client;

/// Reads the live subscription quota of the account a provider's CLI is signed in to.
///
/// Implementations only read CLI credentials. They never refresh or rewrite them, so they
/// cannot sign the CLI out.
pub trait UsageProvider {
    fn id(&self) -> ProviderId;

    fn fetch(&self) -> impl Future<Output = Result<Vec<QuotaWindow>, ProviderError>> + Send;
}

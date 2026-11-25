mod credentials;
mod usage;

use std::path::PathBuf;

use stint_core::{ProviderId, QuotaWindow};

use crate::UsageProvider;
use crate::error::ProviderError;
use crate::http;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// Reads the quota of the ChatGPT account Codex is signed in to.
///
/// The login is never refreshed: OpenAI rotates refresh tokens, so renewing Codex's token
/// here would sign Codex out. The endpoint answers 401 once the access token has expired,
/// which surfaces as [`ProviderError::TokenExpired`] until Codex renews it.
pub struct CodexProvider {
    client: reqwest::Client,
    codex_home: Option<PathBuf>,
}

impl CodexProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            codex_home: credentials::home_dir(),
        }
    }
}

impl UsageProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    async fn fetch(&self) -> Result<Vec<QuotaWindow>, ProviderError> {
        let codex_home = self
            .codex_home
            .as_deref()
            .ok_or(ProviderError::AuthMissing)?;
        let credentials = credentials::load(codex_home)?;

        let request = self
            .client
            .get(USAGE_URL)
            .bearer_auth(&credentials.access_token)
            .header("chatgpt-account-id", &credentials.account_id);
        let body = http::send(request).await?;

        usage::parse(&body, &credentials.account_id)
    }
}

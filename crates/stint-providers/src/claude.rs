mod credentials;
mod usage;

use std::path::PathBuf;

use jiff::Timestamp;
use stint_core::{ProviderId, QuotaWindow};

use crate::UsageProvider;
use crate::error::ProviderError;
use crate::http;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
// The usage endpoint only answers OAuth tokens with this beta header.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Reads the quota of the account Claude Code is signed in to.
///
/// The login is never refreshed: Anthropic rotates refresh tokens, so renewing Claude
/// Code's token here would sign Claude Code out. An expired token is reported as
/// [`ProviderError::TokenExpired`] until Claude Code renews it.
pub struct ClaudeProvider {
    client: reqwest::Client,
    config_dir: Option<PathBuf>,
}

impl ClaudeProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            config_dir: credentials::config_dir(),
        }
    }
}

impl UsageProvider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    async fn fetch(&self) -> Result<Vec<QuotaWindow>, ProviderError> {
        let config_dir = self
            .config_dir
            .as_deref()
            .ok_or(ProviderError::AuthMissing)?;
        let credentials = credentials::load(config_dir)?;
        if credentials.is_expired(Timestamp::now()) {
            return Err(ProviderError::TokenExpired);
        }

        let request = self
            .client
            .get(USAGE_URL)
            .bearer_auth(&credentials.access_token)
            .header("anthropic-beta", OAUTH_BETA);
        let body = http::send(request).await?;

        usage::parse(&body)
    }
}

mod credentials;
mod usage;

use jiff::{SignedDuration, Timestamp};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use stint_core::QuotaWindow;

pub(crate) use credentials::{Slot, import, keychain_user};

use crate::credential::ClaudeLogin;
use crate::error::ProviderError;
use crate::http;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
// The usage endpoint only answers OAuth tokens with this beta header.
const OAUTH_BETA: &str = "oauth-2025-04-20";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
// Claude Code's OAuth client, for logins that do not record their own.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
// Renew this long before the access token expires, to absorb clock skew.
const RENEW_EARLY: SignedDuration = SignedDuration::from_mins(5);
// A refresh refused this way keeps the login: the account is paused, not signed out.
const ACCOUNT_ON_HOLD: &str = "account_on_hold";

pub(crate) async fn usage(
    client: &reqwest::Client,
    login: &ClaudeLogin,
) -> Result<Vec<QuotaWindow>, ProviderError> {
    let request = client
        .get(USAGE_URL)
        .bearer_auth(&login.access_token)
        .header("anthropic-beta", OAUTH_BETA);
    let body = http::send(request).await?;
    usage::parse(&body)
}

pub(crate) fn renewal_due(login: &ClaudeLogin, now: Timestamp) -> bool {
    login
        .expires_at_ms
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .is_some_and(|expires_at| expires_at <= now + RENEW_EARLY)
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    grant_type: &'static str,
    refresh_token: &'a str,
    client_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OAuthError {
    error: Option<String>,
    error_description: Option<String>,
}

/// Trades the refresh token for new tokens. Anthropic rotates refresh tokens, so the
/// returned login replaces the old one at once.
pub(crate) async fn refresh(
    client: &reqwest::Client,
    login: &ClaudeLogin,
    now: Timestamp,
) -> Result<ClaudeLogin, ProviderError> {
    let body = RefreshRequest {
        grant_type: "refresh_token",
        refresh_token: &login.refresh_token,
        client_id: login.client_id.as_deref().unwrap_or(CLIENT_ID),
        scope: (!login.scopes.is_empty()).then(|| login.scopes.join(" ")),
    };
    let answer = http::post_json(client, TOKEN_URL, &body).await?;
    if !answer.status.is_success() {
        return Err(refresh_error(answer.status, &answer.headers, &answer.body));
    }
    let answer: RefreshResponse = serde_json::from_slice(&answer.body)?;
    Ok(renewed(login, answer, now))
}

fn renewed(login: &ClaudeLogin, answer: RefreshResponse, now: Timestamp) -> ClaudeLogin {
    ClaudeLogin {
        access_token: answer.access_token,
        refresh_token: answer
            .refresh_token
            .filter(|token| !token.is_empty())
            .unwrap_or_else(|| login.refresh_token.clone()),
        expires_at_ms: answer
            .expires_in
            .map(|seconds| (now + SignedDuration::from_secs(seconds)).as_millisecond()),
        scopes: answer.scope.map_or_else(
            || login.scopes.clone(),
            |scope| scope.split_whitespace().map(str::to_owned).collect(),
        ),
        client_id: login.client_id.clone(),
    }
}

fn refresh_error(
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
) -> ProviderError {
    let error: OAuthError = serde_json::from_slice(body).unwrap_or_default();
    let spent = matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED)
        && error.error.as_deref() == Some("invalid_grant")
        && error.error_description.as_deref() != Some(ACCOUNT_ON_HOLD);
    if spent {
        return ProviderError::SignInExpired;
    }
    match http::check_status(status, headers) {
        // A rejected refresh that is not a spent grant says nothing about the login.
        Err(ProviderError::TokenExpired) | Ok(()) => ProviderError::Status {
            status: status.as_u16(),
        },
        Err(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderMap;

    use super::*;

    fn login() -> ClaudeLogin {
        ClaudeLogin {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at_ms: Some(1_790_000_000_000),
            scopes: vec!["user:inference".into(), "user:profile".into()],
            client_id: None,
        }
    }

    fn now() -> Timestamp {
        Timestamp::from_millisecond(1_790_000_000_000).unwrap()
    }

    #[test]
    fn renewal_is_due_five_minutes_early() {
        let login = login();
        assert!(renewal_due(&login, now()));
        assert!(renewal_due(&login, now() - SignedDuration::from_mins(4)));
        assert!(!renewal_due(&login, now() - SignedDuration::from_mins(6)));
        let unknown = ClaudeLogin {
            expires_at_ms: None,
            ..login
        };
        assert!(!renewal_due(&unknown, now()));
    }

    #[test]
    fn request_sends_granted_scopes_and_default_client() {
        let login = login();
        let body = RefreshRequest {
            grant_type: "refresh_token",
            refresh_token: &login.refresh_token,
            client_id: login.client_id.as_deref().unwrap_or(CLIENT_ID),
            scope: Some(login.scopes.join(" ")),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["client_id"], CLIENT_ID);
        assert_eq!(json["scope"], "user:inference user:profile");
        assert_eq!(json["grant_type"], "refresh_token");
    }

    #[test]
    fn renewal_rotates_tokens_and_resets_expiry() {
        let answer: RefreshResponse = serde_json::from_str(
            r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":28800,"scope":"user:inference"}"#,
        )
        .unwrap();
        let renewed = renewed(&login(), answer, now());
        assert_eq!(renewed.access_token, "new-access");
        assert_eq!(renewed.refresh_token, "new-refresh");
        assert_eq!(
            renewed.expires_at_ms,
            Some((now() + SignedDuration::from_hours(8)).as_millisecond())
        );
        assert_eq!(renewed.scopes, ["user:inference"]);
    }

    #[test]
    fn renewal_without_rotation_keeps_the_refresh_token() {
        let answer: RefreshResponse =
            serde_json::from_str(r#"{"access_token":"new-access"}"#).unwrap();
        let renewed = renewed(&login(), answer, now());
        assert_eq!(renewed.refresh_token, "old-refresh");
        assert_eq!(renewed.scopes, login().scopes);
        assert_eq!(renewed.expires_at_ms, None);
    }

    #[test]
    fn spent_grant_means_sign_in_again() {
        let headers = HeaderMap::new();
        let spent = br#"{"error":"invalid_grant","error_description":"Refresh token not found"}"#;
        assert!(matches!(
            refresh_error(StatusCode::BAD_REQUEST, &headers, spent),
            ProviderError::SignInExpired
        ));
        let hold = br#"{"error":"invalid_grant","error_description":"account_on_hold"}"#;
        assert!(matches!(
            refresh_error(StatusCode::BAD_REQUEST, &headers, hold),
            ProviderError::Status { status: 400 }
        ));
        assert!(matches!(
            refresh_error(StatusCode::UNAUTHORIZED, &headers, b"{}"),
            ProviderError::Status { status: 401 }
        ));
        assert!(matches!(
            refresh_error(StatusCode::TOO_MANY_REQUESTS, &headers, b""),
            ProviderError::RateLimited { .. }
        ));
    }
}

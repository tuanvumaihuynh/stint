mod credentials;
mod usage;

use jiff::{SignedDuration, Timestamp};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use stint_core::QuotaWindow;

pub(crate) use credentials::import;

use crate::credential::CodexLogin;
use crate::error::ProviderError;
use crate::http;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
// Codex CLI's OAuth client.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
// Renew this long before the access token expires, as Codex does.
const RENEW_EARLY: SignedDuration = SignedDuration::from_mins(5);
// Codex's fallback when the access token's expiry cannot be read.
const RENEW_EVERY: SignedDuration = SignedDuration::from_hours(8 * 24);
// Error codes Codex treats as a spent refresh token.
const SPENT_CODES: [&str; 4] = [
    "invalid_grant",
    "refresh_token_expired",
    "refresh_token_reused",
    "refresh_token_invalidated",
];

pub(crate) async fn usage(
    client: &reqwest::Client,
    login: &CodexLogin,
) -> Result<Vec<QuotaWindow>, ProviderError> {
    let request = client
        .get(USAGE_URL)
        .bearer_auth(&login.access_token)
        .header("chatgpt-account-id", &login.account_id);
    let body = http::send(request).await?;
    usage::parse(&body, &login.account_id)
}

pub(crate) fn renewal_due(login: &CodexLogin, now: Timestamp) -> bool {
    if let Some(expires_at) = credentials::expiry(&login.access_token) {
        return expires_at <= now + RENEW_EARLY;
    }
    login
        .last_refresh_ms
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .is_some_and(|at| at < now - RENEW_EVERY)
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: &'a str,
}

#[derive(Deserialize)]
struct RefreshResponse {
    #[serde(rename = "id_token")]
    id: Option<String>,
    #[serde(rename = "access_token")]
    access: Option<String>,
    #[serde(rename = "refresh_token")]
    refresh: Option<String>,
}

/// Trades the refresh token for new tokens. OpenAI rotates refresh tokens, so the
/// returned login replaces the old one at once.
pub(crate) async fn refresh(
    client: &reqwest::Client,
    login: &CodexLogin,
    now: Timestamp,
) -> Result<CodexLogin, ProviderError> {
    let body = RefreshRequest {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token: &login.refresh_token,
    };
    let url = format!("{}/oauth/token", credentials::ISSUER);
    let answer = http::post_json(client, &url, &body).await?;
    if !answer.status.is_success() {
        return Err(refresh_error(answer.status, &answer.headers, &answer.body));
    }
    let answer: RefreshResponse = serde_json::from_slice(&answer.body)?;
    Ok(renewed(login, answer, now))
}

fn renewed(login: &CodexLogin, answer: RefreshResponse, now: Timestamp) -> CodexLogin {
    let or_old = |new: Option<String>, old: &String| {
        new.filter(|token| !token.is_empty())
            .unwrap_or_else(|| old.clone())
    };
    CodexLogin {
        id_token: or_old(answer.id, &login.id_token),
        access_token: or_old(answer.access, &login.access_token),
        refresh_token: or_old(answer.refresh, &login.refresh_token),
        account_id: login.account_id.clone(),
        last_refresh_ms: Some(now.as_millisecond()),
    }
}

/// The error code of an OAuth failure: `{"error":"code"}` or `{"error":{"code":"code"}}`.
fn error_code(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let error = value.get("error")?;
    error
        .as_str()
        .or_else(|| error.get("code")?.as_str())
        .map(str::to_owned)
}

fn refresh_error(
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
) -> ProviderError {
    let code = error_code(body);
    let spent = status == StatusCode::UNAUTHORIZED
        || code
            .as_deref()
            .is_some_and(|code| SPENT_CODES.contains(&code));
    if spent {
        return ProviderError::SignInExpired;
    }
    match http::check_status(status, headers) {
        Err(ProviderError::TokenExpired) | Ok(()) => ProviderError::Status {
            status: status.as_u16(),
        },
        Err(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use reqwest::header::HeaderMap;

    use super::*;

    fn jwt(claims: &str) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims))
    }

    fn now() -> Timestamp {
        Timestamp::from_second(1_790_000_000).unwrap()
    }

    fn login(access_token: String, last_refresh: Option<Timestamp>) -> CodexLogin {
        CodexLogin {
            id_token: "old-id".into(),
            access_token,
            refresh_token: "old-refresh".into(),
            account_id: "acct".into(),
            last_refresh_ms: last_refresh.map(Timestamp::as_millisecond),
        }
    }

    #[test]
    fn renewal_follows_access_token_expiry() {
        let expiring = login(jwt(r#"{"exp":1790000200}"#), None);
        assert!(renewal_due(&expiring, now()));
        let fresh = login(jwt(r#"{"exp":1790001000}"#), Some(now() - RENEW_EVERY * 2));
        assert!(!renewal_due(&fresh, now()));
    }

    #[test]
    fn unreadable_expiry_renews_after_eight_days() {
        let old = login(
            "opaque".into(),
            Some(now() - RENEW_EVERY - SignedDuration::from_secs(1)),
        );
        assert!(renewal_due(&old, now()));
        let recent = login(
            "opaque".into(),
            Some(now() - SignedDuration::from_hours(24)),
        );
        assert!(!renewal_due(&recent, now()));
        assert!(!renewal_due(&login("opaque".into(), None), now()));
    }

    #[test]
    fn renewal_replaces_given_tokens_and_stamps_the_time() {
        let answer: RefreshResponse =
            serde_json::from_str(r#"{"access_token":"new-access","refresh_token":"new-refresh"}"#)
                .unwrap();
        let renewed = renewed(&login("old-access".into(), None), answer, now());
        assert_eq!(renewed.access_token, "new-access");
        assert_eq!(renewed.refresh_token, "new-refresh");
        assert_eq!(renewed.id_token, "old-id");
        assert_eq!(renewed.account_id, "acct");
        assert_eq!(renewed.last_refresh_ms, Some(now().as_millisecond()));
    }

    #[test]
    fn spent_refresh_tokens_mean_sign_in_again() {
        let headers = HeaderMap::new();
        for (status, body) in [
            (StatusCode::UNAUTHORIZED, &b"{}"[..]),
            (StatusCode::BAD_REQUEST, br#"{"error":"invalid_grant"}"#),
            (
                StatusCode::BAD_REQUEST,
                br#"{"error":{"code":"refresh_token_reused","message":"x"}}"#,
            ),
        ] {
            assert!(matches!(
                refresh_error(status, &headers, body),
                ProviderError::SignInExpired
            ));
        }
        assert!(matches!(
            refresh_error(
                StatusCode::BAD_REQUEST,
                &headers,
                br#"{"error":"invalid_client"}"#
            ),
            ProviderError::Status { status: 400 }
        ));
        assert!(matches!(
            refresh_error(StatusCode::BAD_GATEWAY, &headers, b""),
            ProviderError::Status { status: 502 }
        ));
    }
}

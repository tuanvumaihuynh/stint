//! Google Antigravity accounts. Stint signs them in itself with Antigravity's OAuth client
//! (see [`Authorization`]) and reads quota from the Cloud Code API Antigravity uses.

mod oauth;
mod quota;

use jiff::{SignedDuration, Timestamp};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde_json::json;
use stint_core::QuotaWindow;

pub use oauth::Authorization;

use crate::credential::{AntigravityLogin, Credential};
use crate::error::ProviderError;
use crate::http;
use crate::signin::Enrolled;

// Antigravity's consumer OAuth client. Installed-app secrets are public by design.
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const SCOPES: [&str; 6] = [
    "openid",
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const CLOUD_CODE: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal";
// Which model groups Google reports depends on the client it believes is asking.
const CLIENT_VERSION: &str = "antigravity/hub/2.24.0";
const CLIENT_BUILD: &str = "cl=963137146";
// Renew this long before the access token expires, to absorb clock skew.
const RENEW_EARLY: SignedDuration = SignedDuration::from_mins(5);

/// Trades the code from a finished browser sign-in for a login, and reads who the account
/// is and its plan.
pub async fn enroll(
    client: &reqwest::Client,
    authorization: &Authorization,
    code: &str,
) -> Result<Enrolled, ProviderError> {
    let redirect_uri = oauth::redirect_uri();
    let answer = http::post_form(
        client,
        TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", &authorization.verifier),
            ("redirect_uri", &redirect_uri),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ],
    )
    .await?;
    if !answer.status.is_success() {
        return Err(token_error(answer.status, &answer.headers, &answer.body));
    }
    let tokens = tokens(&answer.body)?;
    let refresh_token = tokens
        .refresh_token
        .filter(|token| !token.is_empty())
        .ok_or(ProviderError::SignInRefused(
            "Google returned no refresh token",
        ))?;
    let now = Timestamp::now();
    let mut login = AntigravityLogin {
        access_token: tokens.access_token,
        refresh_token,
        expires_at_ms: expiry(now, tokens.expires_in),
        subject: None,
    };

    let identity: UserInfo = serde_json::from_slice(
        &http::send(client.get(USERINFO_URL).bearer_auth(&login.access_token)).await?,
    )?;
    login.subject = identity.sub;
    let setup = setup(client, &login).await?;
    Ok(Enrolled {
        credential: Credential::Antigravity(login),
        email: identity.email,
        plan: setup.plan,
    })
}

pub(crate) async fn usage(
    client: &reqwest::Client,
    login: &AntigravityLogin,
) -> Result<Vec<QuotaWindow>, ProviderError> {
    let setup = setup(client, login).await?;
    let body = call(
        client,
        login,
        "retrieveUserQuotaSummary",
        &json!({ "project": setup.project }),
    )
    .await?;
    quota::summary(&body)
}

pub(crate) fn renewal_due(login: &AntigravityLogin, now: Timestamp) -> bool {
    login
        .expires_at_ms
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .is_some_and(|expires_at| expires_at <= now + RENEW_EARLY)
}

/// Trades the refresh token for a new access token. Google usually keeps the refresh
/// token, and sends a new one only when it rotates.
pub(crate) async fn refresh(
    client: &reqwest::Client,
    login: &AntigravityLogin,
    now: Timestamp,
) -> Result<AntigravityLogin, ProviderError> {
    let answer = http::post_form(
        client,
        TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &login.refresh_token),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ],
    )
    .await?;
    if !answer.status.is_success() {
        return Err(token_error(answer.status, &answer.headers, &answer.body));
    }
    let tokens = tokens(&answer.body)?;
    Ok(AntigravityLogin {
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .filter(|token| !token.is_empty())
            .unwrap_or_else(|| login.refresh_token.clone()),
        expires_at_ms: expiry(now, tokens.expires_in),
        subject: login.subject.clone(),
    })
}

async fn setup(
    client: &reqwest::Client,
    login: &AntigravityLogin,
) -> Result<quota::Setup, ProviderError> {
    let metadata = json!({
        "metadata": {
            "ideType": "ANTIGRAVITY",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        }
    });
    quota::setup(&call(client, login, "loadCodeAssist", &metadata).await?)
}

/// Posts to a Cloud Code method as Antigravity does.
async fn call(
    client: &reqwest::Client,
    login: &AntigravityLogin,
    method: &str,
    body: &serde_json::Value,
) -> Result<Vec<u8>, ProviderError> {
    let request = client
        .post(format!("{CLOUD_CODE}:{method}"))
        .bearer_auth(&login.access_token)
        .header(USER_AGENT, user_agent())
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(body)?);
    let response = request.send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?.to_vec();
    if status == StatusCode::FORBIDDEN {
        return Err(ProviderError::AccountSetup(
            forbidden_reason(&body)
                .unwrap_or_else(|| "Google refused to share the quota".to_owned()),
        ));
    }
    http::check_status(status, &headers)?;
    Ok(body)
}

fn user_agent() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    format!("{CLIENT_VERSION} (aidev_client; os_type=darwin; arch={arch}; {CLIENT_BUILD})")
}

fn expiry(now: Timestamp, expires_in: Option<i64>) -> Option<i64> {
    let seconds = expires_in.filter(|seconds| *seconds > 0)?;
    Some((now + SignedDuration::from_secs(seconds)).as_millisecond())
}

/// Reads a token answer. A parse error could quote the tokens, so it says nothing more.
fn tokens(body: &[u8]) -> Result<TokenResponse, ProviderError> {
    serde_json::from_slice(body)
        .map_err(|_| ProviderError::SignInRefused("Google sent a token answer Stint cannot read"))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct UserInfo {
    sub: Option<String>,
    email: Option<String>,
}

/// A refused grant means signing in again; anything else keeps the login.
fn token_error(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> ProviderError {
    let code = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_owned));
    match code.as_deref() {
        Some("invalid_grant") => ProviderError::SignInExpired,
        Some("invalid_client" | "unauthorized_client" | "redirect_uri_mismatch") => {
            ProviderError::SignInRefused("Google no longer accepts this sign-in client")
        }
        _ => match http::check_status(status, headers) {
            Err(ProviderError::TokenExpired) | Ok(()) => ProviderError::Status {
                status: status.as_u16(),
            },
            Err(err) => err,
        },
    }
}

/// The reason in a Google API error, such as `VALIDATION_REQUIRED`, with its message.
fn forbidden_reason(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let error = value.get("error")?;
    let reason = error
        .get("details")?
        .as_array()?
        .iter()
        .find_map(|detail| detail.get("reason")?.as_str());
    let message = error.get("message").and_then(serde_json::Value::as_str);
    match (reason, message) {
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        Timestamp::from_second(1_790_000_000).unwrap()
    }

    fn login(expires_in_secs: i64) -> AntigravityLogin {
        AntigravityLogin {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at_ms: Some(
                (now() + SignedDuration::from_secs(expires_in_secs)).as_millisecond(),
            ),
            subject: Some("sub-1".into()),
        }
    }

    #[test]
    fn renewal_follows_expiry() {
        assert!(renewal_due(&login(60), now()));
        assert!(!renewal_due(&login(3600), now()));
        let unknown = AntigravityLogin {
            expires_at_ms: None,
            ..login(0)
        };
        assert!(!renewal_due(&unknown, now()));
    }

    #[test]
    fn refused_grants_mean_sign_in_again() {
        let headers = HeaderMap::new();
        assert!(matches!(
            token_error(
                StatusCode::BAD_REQUEST,
                &headers,
                br#"{"error":"invalid_grant"}"#
            ),
            ProviderError::SignInExpired
        ));
        assert!(matches!(
            token_error(
                StatusCode::UNAUTHORIZED,
                &headers,
                br#"{"error":"invalid_client"}"#
            ),
            ProviderError::SignInRefused(_)
        ));
        assert!(matches!(
            token_error(StatusCode::BAD_GATEWAY, &headers, b""),
            ProviderError::Status { status: 502 }
        ));
    }

    #[test]
    fn unreadable_token_answers_never_quote_them() {
        let err = tokens(br#"{"access_token":12345678}"#).err().unwrap();
        assert!(!err.to_string().contains("12345678"));
    }

    #[test]
    fn forbidden_answers_keep_their_reason() {
        let body = br#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"Verify your account.",
            "details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"VALIDATION_REQUIRED"}]}}"#;
        assert_eq!(
            forbidden_reason(body).as_deref(),
            Some("VALIDATION_REQUIRED: Verify your account.")
        );
        assert_eq!(forbidden_reason(b"nope"), None);
    }

    #[test]
    fn user_agent_names_the_mac() {
        assert!(user_agent().starts_with("antigravity/hub/"));
        assert!(user_agent().contains("os_type=darwin"));
    }
}

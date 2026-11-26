//! Reads the login `codex login` leaves in a private `CODEX_HOME`.

use std::fs;
use std::io;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use serde::Deserialize;

use crate::credential::{CodexLogin, Credential};
use crate::error::ProviderError;
use crate::plan;
use crate::signin::Enrolled;

const AUTH_FILE: &str = "auth.json";
/// The only sign-in authority Stint renews logins with.
pub(crate) const ISSUER: &str = "https://auth.openai.com";

#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthFile {
    tokens: Option<Tokens>,
    last_refresh: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Tokens {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    oauth_provenance: Option<Provenance>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Provenance {
    issuer: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Claims {
    email: Option<String>,
    exp: Option<i64>,
    #[serde(rename = "https://api.openai.com/profile")]
    profile: Option<ProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaims>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ProfileClaims {
    email: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthClaims {
    chatgpt_account_id: Option<String>,
    chatgpt_plan_type: Option<String>,
}

pub(crate) fn import(codex_home: &Path) -> Result<Enrolled, ProviderError> {
    let path = codex_home.join(AUTH_FILE);
    match fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(ProviderError::AuthMissing),
        Err(err) => Err(ProviderError::io(path)(err)),
    }
}

/// An API-key login has no `tokens` and no subscription quota, so it counts as missing.
pub(crate) fn parse(bytes: &[u8]) -> Result<Enrolled, ProviderError> {
    let file: AuthFile = serde_json::from_slice(bytes)?;
    let tokens = file.tokens.ok_or(ProviderError::AuthMissing)?;
    if let Some(issuer) = tokens.oauth_provenance.and_then(|p| p.issuer)
        && issuer.trim_end_matches('/') != ISSUER
    {
        return Err(ProviderError::Unsupported(format!("logins from {issuer}")));
    }

    let id_token = non_empty(tokens.id_token).ok_or(ProviderError::AuthMissing)?;
    let access_token = non_empty(tokens.access_token).ok_or(ProviderError::AuthMissing)?;
    let refresh_token = non_empty(tokens.refresh_token).ok_or(ProviderError::AuthMissing)?;
    let claims = claims(&id_token).unwrap_or_default();
    let auth = claims.auth.unwrap_or_default();
    let account_id = non_empty(tokens.account_id)
        .or(non_empty(auth.chatgpt_account_id))
        .ok_or(ProviderError::AuthMissing)?;
    let last_refresh_ms = file
        .last_refresh
        .and_then(|at| at.parse::<Timestamp>().ok())
        .map(Timestamp::as_millisecond);

    Ok(Enrolled {
        credential: Credential::Codex(CodexLogin {
            id_token,
            access_token,
            refresh_token,
            account_id,
            last_refresh_ms,
        }),
        email: non_empty(claims.email).or(non_empty(claims.profile.and_then(|p| p.email))),
        plan: non_empty(auth.chatgpt_plan_type).map(|code| plan::codex(&code)),
    })
}

/// When a JWT expires, from its unverified `exp` claim.
pub(crate) fn expiry(token: &str) -> Option<Timestamp> {
    Timestamp::from_second(claims(token)?.exp?).ok()
}

/// A JWT's payload. The signature is not checked: the claims only label the account and
/// schedule renewal, and the servers check the tokens themselves.
fn claims(token: &str) -> Option<Claims> {
    let payload = token.split('.').nth(1)?;
    let json = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&json).ok()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn jwt(claims: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims);
        format!("eyJhbGciOiJub25lIn0.{payload}.c2ln")
    }

    fn auth(tokens: &str) -> String {
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{tokens},"last_refresh":"2026-09-20T12:00:00Z"}}"#
        )
    }

    fn login(enrolled: &Enrolled) -> &CodexLogin {
        match &enrolled.credential {
            Credential::Codex(login) => login,
            Credential::Claude(_) => panic!("not a Codex login"),
        }
    }

    #[test]
    fn reads_tokens_identity_and_plan() {
        let id = jwt(
            r#"{"email":"me@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"acct_jwt","chatgpt_plan_type":"plus"}}"#,
        );
        let body = auth(&format!(
            r#"{{"id_token":"{id}","access_token":"a","refresh_token":"r","account_id":"acct_fake_123"}}"#
        ));
        let enrolled = parse(body.as_bytes()).unwrap();
        let login = login(&enrolled);
        assert_eq!(login.access_token, "a");
        assert_eq!(login.refresh_token, "r");
        assert_eq!(login.account_id, "acct_fake_123");
        assert_eq!(
            login.last_refresh_ms,
            Some(
                "2026-09-20T12:00:00Z"
                    .parse::<Timestamp>()
                    .unwrap()
                    .as_millisecond()
            )
        );
        assert_eq!(enrolled.email.as_deref(), Some("me@example.com"));
        assert_eq!(enrolled.plan.as_deref(), Some("Plus"));
    }

    #[test]
    fn account_id_and_email_fall_back_to_claims() {
        let id = jwt(
            r#"{"https://api.openai.com/profile":{"email":"p@example.com"},"https://api.openai.com/auth":{"chatgpt_account_id":"acct_jwt"}}"#,
        );
        let body = auth(&format!(
            r#"{{"id_token":"{id}","access_token":"a","refresh_token":"r"}}"#
        ));
        let enrolled = parse(body.as_bytes()).unwrap();
        assert_eq!(login(&enrolled).account_id, "acct_jwt");
        assert_eq!(enrolled.email.as_deref(), Some("p@example.com"));
        assert_eq!(enrolled.plan, None);
    }

    #[test]
    fn api_key_or_incomplete_login_is_auth_missing() {
        for body in [
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-FAKE","tokens":null}"#.to_owned(),
            auth(r#"{"id_token":"x.e30.y","access_token":"a","account_id":"acct"}"#),
            auth(r#"{"id_token":"x.e30.y","access_token":"a","refresh_token":"r"}"#),
        ] {
            assert!(matches!(
                parse(body.as_bytes()),
                Err(ProviderError::AuthMissing)
            ));
        }
    }

    #[test]
    fn other_issuers_are_unsupported() {
        let body = auth(
            r#"{"id_token":"x.e30.y","access_token":"a","refresh_token":"r","account_id":"acct","oauth_provenance":{"issuer":"https://example.com"}}"#,
        );
        assert!(matches!(
            parse(body.as_bytes()),
            Err(ProviderError::Unsupported(_))
        ));
        let body = auth(
            r#"{"id_token":"x.e30.y","access_token":"a","refresh_token":"r","account_id":"acct","oauth_provenance":{"issuer":"https://auth.openai.com/"}}"#,
        );
        assert!(parse(body.as_bytes()).is_ok());
    }

    #[test]
    fn expiry_reads_exp_seconds() {
        assert_eq!(
            expiry(&jwt(r#"{"exp":1790000000}"#)),
            Some(Timestamp::from_second(1_790_000_000).unwrap())
        );
        assert_eq!(expiry("not-a-jwt"), None);
        assert_eq!(expiry(&jwt("{}")), None);
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(parse(b"{"), Err(ProviderError::Decode(_))));
    }
}

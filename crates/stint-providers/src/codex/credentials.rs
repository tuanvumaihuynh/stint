use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

use crate::error::ProviderError;

const AUTH_FILE: &str = "auth.json";

/// The ChatGPT login Codex stores, reduced to what a usage read needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub(crate) access_token: String,
    pub(crate) account_id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthFile {
    tokens: Option<Tokens>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Tokens {
    id_token: Option<String>,
    access_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct IdClaims {
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<AuthClaims>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthClaims {
    chatgpt_account_id: Option<String>,
}

/// Codex's home directory, honoring `CODEX_HOME`.
pub(crate) fn home_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::home_dir().map(|home| home.join(".codex"))
}

/// Loads the signed-in login from `auth.json`, Codex's default store.
///
/// Codex can be configured to keep its login in the keychain instead; that store is not
/// read yet, so such setups report [`ProviderError::AuthMissing`].
pub(crate) fn load(codex_home: &Path) -> Result<Credentials, ProviderError> {
    let path = codex_home.join(AUTH_FILE);
    match fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(ProviderError::AuthMissing),
        Err(source) => Err(ProviderError::Io { path, source }),
    }
}

/// An API-key login has no `tokens` and no subscription quota, so it counts as missing.
pub(crate) fn parse(bytes: &[u8]) -> Result<Credentials, ProviderError> {
    let file: AuthFile = serde_json::from_slice(bytes)?;
    let tokens = file.tokens.ok_or(ProviderError::AuthMissing)?;

    let access_token = non_empty(tokens.access_token).ok_or(ProviderError::AuthMissing)?;
    let account_id = non_empty(tokens.account_id)
        .or_else(|| tokens.id_token.as_deref().and_then(account_id_claim))
        .ok_or(ProviderError::AuthMissing)?;

    Ok(Credentials {
        access_token,
        account_id,
    })
}

/// Reads the ChatGPT account id from an ID token's payload. The signature is not checked:
/// the token only names which account to ask about, and the server checks the access token.
fn account_id_claim(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let json = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: IdClaims = serde_json::from_slice(&json).ok()?;
    non_empty(claims.auth?.chatgpt_account_id)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH: &str = include_str!("testdata/auth.json");

    fn id_token(claims: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims);
        format!("eyJhbGciOiJub25lIn0.{payload}.c2ln")
    }

    #[test]
    fn reads_access_token_and_account_id() {
        let credentials = parse(AUTH.as_bytes()).unwrap();
        assert_eq!(credentials.access_token, "eyJFAKE.access.token");
        assert_eq!(credentials.account_id, "acct_fake_123");
    }

    #[test]
    fn falls_back_to_the_id_token_claim() {
        let token =
            id_token(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct_from_jwt"}}"#);
        let body = format!(r#"{{"tokens":{{"access_token":"a","id_token":"{token}"}}}}"#);
        let credentials = parse(body.as_bytes()).unwrap();
        assert_eq!(credentials.account_id, "acct_from_jwt");
    }

    #[test]
    fn api_key_login_is_auth_missing() {
        let body = r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-FAKE","tokens":null}"#;
        assert!(matches!(
            parse(body.as_bytes()),
            Err(ProviderError::AuthMissing)
        ));
    }

    #[test]
    fn login_without_account_id_is_auth_missing() {
        for id_token in ["not-a-jwt", "a.%%%.c", &id_token(r#"{"email":"x"}"#)] {
            let body = format!(r#"{{"tokens":{{"access_token":"a","id_token":"{id_token}"}}}}"#);
            assert!(matches!(
                parse(body.as_bytes()),
                Err(ProviderError::AuthMissing)
            ));
        }
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(parse(b"{"), Err(ProviderError::Decode(_))));
    }
}

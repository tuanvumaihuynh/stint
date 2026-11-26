//! Signs accounts in through the providers' own CLIs, keeps their logins in the Keychain,
//! and reads their live subscription quota.

mod accounts;
mod claude;
mod codex;
mod credential;
mod error;
mod http;
mod keychain;
mod plan;
mod signin;

pub use accounts::Accounts;
pub use credential::{ClaudeLogin, CodexLogin, Credential, Vault};
pub use error::ProviderError;
pub use http::client as http_client;
pub use signin::{Enrolled, SignIn, new_credential_key};

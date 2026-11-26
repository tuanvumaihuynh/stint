//! Signs accounts in, through the providers' own CLIs or in the browser, keeps their logins in the Keychain,
//! and reads their live subscription quota.

mod accounts;
pub mod antigravity;
mod claude;
mod codex;
mod credential;
mod error;
mod http;
mod keychain;
mod plan;
mod signin;

pub use accounts::Accounts;
pub use credential::{AntigravityLogin, ClaudeLogin, CodexLogin, Credential, Vault};
pub use error::ProviderError;
pub use http::client as http_client;
pub use signin::{CliProvider, Enrolled, SignIn, SignInEvent, Watched, new_credential_key};

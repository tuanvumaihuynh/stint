//! Adding an account, or signing one in again, from the window.

use std::sync::Arc;

use stint_core::{AccountId, NewAccount, ProviderId, QuotaStore, StoreError};
use stint_providers::{Enrolled, ProviderError, SignIn, SignInEvent, Vault, Watched};

use crate::{Stint, StintError, menu};

/// Told, from a background thread, how a sign-in is going: maybe a link, maybe a code
/// prompt, then exactly one of `succeeded` or `failed`.
#[uniffi::export(with_foreign)]
pub trait SignInListener: Send + Sync {
    /// The sign-in page, to offer opening it again.
    fn link(&self, url: String);
    /// The CLI takes a code pasted from the sign-in page.
    fn code_prompt(&self);
    /// The account is saved. `email` and `plan` are what the login says.
    fn succeeded(&self, email: Option<String>, plan: Option<String>);
    /// Why the sign-in did not add the account, in plain words.
    fn failed(&self, message: String, cancelled: bool);
}

/// A sign-in in progress. Dropping it cancels the sign-in.
#[derive(uniffi::Object)]
pub struct SignInSession {
    watched: Watched,
}

#[uniffi::export]
impl SignInSession {
    /// Sends a code pasted from the sign-in page.
    pub fn submit_code(&self, code: &str) -> Result<(), StintError> {
        self.watched
            .submit_code(code)
            .map_err(|_| StintError::Message("The sign-in no longer takes a code.".to_owned()))
    }

    /// Stops the sign-in; the listener then hears `failed` with `cancelled` set.
    pub fn cancel(&self) {
        self.watched.cancel();
    }
}

/// Which account a finished sign-in saves to.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    New { provider: ProviderId, name: String },
    Existing { id: AccountId },
}

impl Stint {
    pub(crate) fn start_sign_in(
        self: &Arc<Self>,
        target: Target,
        listener: Arc<dyn SignInListener>,
    ) -> Result<Arc<SignInSession>, StintError> {
        let Some(data_dir) = &self.data_dir else {
            return Err(StintError::Message(
                "Sign-in is off for made-up accounts.".to_owned(),
            ));
        };
        let provider = self.check_target(&target)?;
        let attempt = SignIn::start(provider, data_dir)
            .map_err(|err| StintError::Message(failure(&err, provider)))?;
        let stint = self.clone();
        let watched = attempt
            .watch(move |event| match event {
                SignInEvent::Link(url) => listener.link(url),
                SignInEvent::CodePrompt => listener.code_prompt(),
                SignInEvent::Finished(Ok(enrolled)) => {
                    let (email, plan) = (enrolled.email.clone(), enrolled.plan.clone());
                    match stint.save_login(&target, enrolled) {
                        Ok(()) => {
                            listener.succeeded(email, plan);
                            stint.clone().refresh();
                        }
                        Err(message) => listener.failed(message, false),
                    }
                }
                SignInEvent::Finished(Err(err)) => listener.failed(
                    failure(&err, provider),
                    matches!(err, ProviderError::SignInCancelled),
                ),
            })
            .map_err(|err| StintError::Message(failure(&err, provider)))?;
        Ok(Arc::new(SignInSession { watched }))
    }

    /// The provider to sign in to, once the nickname or account is known to be usable.
    fn check_target(&self, target: &Target) -> Result<ProviderId, StintError> {
        let mut state = self.state();
        let store = state.store_mut().ok_or_else(|| {
            StintError::Message("Sign-in is off for made-up accounts.".to_owned())
        })?;
        let accounts = store.accounts().map_err(|err| store_failure(&err))?;
        match target {
            Target::New { provider, name } => {
                if *provider == ProviderId::Gemini {
                    return Err(StintError::Message(
                        "Gemini accounts are coming later.".to_owned(),
                    ));
                }
                if accounts
                    .iter()
                    .any(|a| a.provider == *provider && a.label == *name)
                {
                    return Err(duplicate(*provider, name));
                }
                Ok(*provider)
            }
            Target::Existing { id } => accounts
                .iter()
                .find(|a| a.id == *id)
                .map(|a| a.provider)
                .ok_or_else(|| StintError::Message("That account was removed.".to_owned())),
        }
    }

    /// Saves a finished sign-in's login to the Keychain and the account to the database.
    fn save_login(&self, target: &Target, enrolled: Enrolled) -> Result<(), String> {
        let vault = Vault::new();
        match target {
            Target::New { provider, name } => {
                let key = stint_providers::new_credential_key(*provider);
                vault
                    .save(&key, &enrolled.credential)
                    .map_err(|err| failure(&err, *provider))?;
                let account = NewAccount {
                    provider: *provider,
                    label: name.clone(),
                    email: enrolled.email,
                    plan: enrolled.plan,
                    credential_key: key.clone(),
                };
                let mut state = self.state();
                let store = state
                    .store_mut()
                    .ok_or("Sign-in is off for made-up accounts.")?;
                if let Err(err) = store.insert_account(&account, jiff::Timestamp::now()) {
                    drop(state);
                    vault.delete(&key).ok();
                    return Err(match err {
                        StoreError::DuplicateAccount { provider, label } => {
                            duplicate(provider, &label).to_string()
                        }
                        err => store_failure(&err).to_string(),
                    });
                }
                state.reload();
            }
            Target::Existing { id } => {
                let profile = {
                    let mut state = self.state();
                    let store = state
                        .store_mut()
                        .ok_or("Sign-in is off for made-up accounts.")?;
                    let accounts = store
                        .accounts()
                        .map_err(|err| store_failure(&err).to_string())?;
                    accounts
                        .into_iter()
                        .find(|a| a.id == *id)
                        .ok_or("That account was removed while signing in.")?
                };
                if let (Some(old), Some(new)) = (&profile.email, &enrolled.email)
                    && !old.eq_ignore_ascii_case(new)
                {
                    return Err(format!(
                        "You signed in as {new}, but “{}” is {old}. Sign in with {old} instead.",
                        profile.label
                    ));
                }
                vault
                    .save(&profile.credential_key, &enrolled.credential)
                    .map_err(|err| failure(&err, profile.provider))?;
                let email = enrolled.email.or(profile.email);
                let plan = enrolled.plan.or(profile.plan);
                let mut state = self.state();
                state.forget_error(*id);
                if let Some(store) = state.store_mut()
                    && let Err(err) =
                        store.set_account_login(*id, email.as_deref(), plan.as_deref())
                {
                    eprintln!("stint: update account {id}: {err}");
                }
                state.reload();
            }
        }
        self.listener.changed();
        Ok(())
    }
}

pub(crate) fn duplicate(provider: ProviderId, name: &str) -> StintError {
    StintError::Message(format!(
        "You already have a {} account named “{name}”.",
        menu::provider_name(provider)
    ))
}

pub(crate) fn store_failure(err: &StoreError) -> StintError {
    eprintln!("stint: database: {err}");
    StintError::Message("Stint couldn't update its database.".to_owned())
}

/// Why a sign-in failed, in plain words. Never includes the CLI's output or a login.
fn failure(err: &ProviderError, provider: ProviderId) -> String {
    let cli = match provider {
        ProviderId::Claude => "Claude Code",
        ProviderId::Codex => "The Codex CLI",
        ProviderId::Gemini => "The Gemini CLI",
    };
    match err {
        ProviderError::CliMissing(_) => {
            format!("{cli} isn't installed. Install it, then try again.")
        }
        ProviderError::SignInCancelled => "The sign-in was cancelled.".to_owned(),
        ProviderError::SignInFailed { .. } => format!(
            "The sign-in page expired or was closed before {} returned a session.",
            menu::provider_name(provider)
        ),
        ProviderError::AuthMissing => {
            "The sign-in finished, but no login was saved. Try again.".to_owned()
        }
        ProviderError::KeychainLocked => {
            "The Keychain is locked. Unlock it, then try again.".to_owned()
        }
        ProviderError::Unsupported(_) => {
            format!(
                "{} accounts are coming later.",
                menu::provider_name(provider)
            )
        }
        _ => "Stint couldn't save the new login. Try again.".to_owned(),
    }
}

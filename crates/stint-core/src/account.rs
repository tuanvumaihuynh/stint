use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(i64);

impl AccountId {
    #[must_use]
    pub const fn new(raw: i64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    Claude,
    Codex,
    Gemini,
}

impl ProviderId {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Gemini];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| CoreError::UnknownProvider(s.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub provider: ProviderId,
    pub label: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountProfile {
    pub id: AccountId,
    pub provider: ProviderId,
    pub label: String,
    pub email: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_round_trips_through_str() {
        for provider in ProviderId::ALL {
            assert_eq!(provider.as_str().parse::<ProviderId>(), Ok(provider));
        }
    }

    #[test]
    fn provider_id_rejects_unknown_names() {
        assert_eq!(
            "openai".parse::<ProviderId>(),
            Err(CoreError::UnknownProvider("openai".into()))
        );
    }
}

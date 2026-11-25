use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, TimeDelta, Utc};

use crate::account::AccountId;
use crate::error::CoreError;

/// A usage percentage clamped to `0.0..=100.0`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Percentage(f64);

impl Percentage {
    pub const ZERO: Self = Self(0.0);
    pub const FULL: Self = Self(100.0);

    /// Clamps finite values into range and rejects `NaN` and infinities.
    pub fn new(value: f64) -> Result<Self, CoreError> {
        if value.is_finite() {
            Ok(Self(value.clamp(0.0, 100.0)))
        } else {
            Err(CoreError::InvalidPercentage(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn remaining(self) -> Self {
        Self(100.0 - self.0)
    }
}

impl TryFrom<f64> for Percentage {
    type Error = CoreError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.0}%", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    FiveHour,
    SevenDay,
    Daily,
}

impl WindowKind {
    pub const ALL: [Self; 3] = [Self::FiveHour, Self::SevenDay, Self::Daily];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FiveHour => "five_hour",
            Self::SevenDay => "seven_day",
            Self::Daily => "daily",
        }
    }

    #[must_use]
    pub const fn duration(self) -> TimeDelta {
        match self {
            Self::FiveHour => TimeDelta::hours(5),
            Self::SevenDay => TimeDelta::days(7),
            Self::Daily => TimeDelta::days(1),
        }
    }
}

impl fmt::Display for WindowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WindowKind {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| CoreError::UnknownWindowKind(s.to_owned()))
    }
}

/// One rolling quota window as reported by a provider.
///
/// `scope` narrows the window to a model family (for example `opus` or a
/// Gemini model id); `None` means the window covers the whole plan.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaWindow {
    pub kind: WindowKind,
    pub scope: Option<String>,
    pub used: Percentage,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Every window fetched for one account in a single poll.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSnapshot {
    pub account_id: AccountId,
    pub fetched_at: DateTime<Utc>,
    pub windows: Vec<QuotaWindow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_clamps_out_of_range_values() -> Result<(), CoreError> {
        assert_eq!(Percentage::new(-5.0)?, Percentage::ZERO);
        assert_eq!(Percentage::new(120.0)?, Percentage::FULL);
        assert!((Percentage::new(42.5)?.get() - 42.5).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn percentage_rejects_non_finite_values() {
        assert!(Percentage::new(f64::NAN).is_err());
        assert!(Percentage::try_from(f64::INFINITY).is_err());
    }

    #[test]
    fn percentage_remaining_is_complement() -> Result<(), CoreError> {
        let remaining = Percentage::new(30.0)?.remaining();
        assert!((remaining.get() - 70.0).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn window_kind_round_trips_through_str() {
        for kind in WindowKind::ALL {
            assert_eq!(kind.as_str().parse::<WindowKind>(), Ok(kind));
        }
        assert!("monthly".parse::<WindowKind>().is_err());
    }
}

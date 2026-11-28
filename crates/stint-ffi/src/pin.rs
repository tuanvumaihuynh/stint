use stint_core::{AccountId, WindowKind};

use crate::menu;
use crate::quota::{AccountQuota, FetchError};

pub(crate) const PIN_SETTING: &str = "menu_bar_account";
pub(crate) const STYLE_SETTING: &str = "menu_bar_style";
// What the menu bar shows for an account without a current reading.
const NO_READING: &str = "—";

/// What the status item shows next to the Stint icon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum MenuBarStyle {
    /// The icon alone.
    Icon,
    /// The icon and the pinned account's 5-hour percent used, such as "48%".
    #[default]
    IconPercent,
}

impl MenuBarStyle {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::IconPercent => "icon_percent",
        }
    }

    /// The stored value read back; anything unknown is the default.
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "icon" => Self::Icon,
            _ => Self::IconPercent,
        }
    }
}

/// The stored pin read back; a cleared or unreadable value pins nothing.
pub(crate) fn parse_pin(value: &str) -> Option<AccountId> {
    value.parse().ok().map(AccountId::new)
}

/// The pinned account: the stored one while it exists, else the first in provider order.
pub(crate) fn effective(accounts: &[AccountQuota], stored: Option<AccountId>) -> Option<AccountId> {
    if let Some(id) = stored
        && accounts.iter().any(|a| a.id == id)
    {
        return Some(id);
    }
    menu::PROVIDERS
        .iter()
        .find_map(|provider| accounts.iter().find(|a| a.provider == *provider))
        .map(|a| a.id)
}

/// The status item text for `account` in `style`, or `None` for the icon alone.
pub(crate) fn status_text(
    account: Option<&AccountQuota>,
    style: MenuBarStyle,
    offline: bool,
) -> Option<String> {
    if style == MenuBarStyle::Icon {
        return None;
    }
    let Some(account) = account else {
        return Some(NO_READING.to_owned());
    };
    // Offline as a whole keeps the last numbers, as the menu does; the icon is dimmed.
    let current = match &account.error {
        None => true,
        Some(FetchError::Offline) => offline,
        Some(_) => false,
    };
    let windows = &account.windows;
    let shown = windows
        .iter()
        .find(|w| w.scope.is_none() && w.kind == WindowKind::FiveHour)
        .or_else(|| windows.iter().find(|w| w.scope.is_none()))
        .or_else(|| windows.iter().find(|w| w.kind == WindowKind::FiveHour))
        .or_else(|| windows.first());
    match shown {
        Some(window) if current => Some(format!("{:.0}%", window.used.get())),
        _ => Some(NO_READING.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use stint_core::ProviderId;

    use super::*;
    use crate::mock;

    fn accounts(scenario: &str) -> Vec<AccountQuota> {
        mock::scenario(
            scenario,
            "2026-11-25T13:00:00Z".parse::<Timestamp>().unwrap(),
        )
        .unwrap()
    }

    fn text(account: &AccountQuota) -> Option<String> {
        status_text(Some(account), MenuBarStyle::IconPercent, false)
    }

    #[test]
    fn pin_falls_back_to_the_first_account_in_provider_order() {
        let mut list = accounts("default");
        // Listed out of provider order, as the database may return them.
        list.reverse();
        let first = list
            .iter()
            .find(|a| a.provider == ProviderId::Claude)
            .unwrap()
            .id;
        assert_eq!(effective(&list, None), Some(first));
        assert_eq!(effective(&[], None), None);
    }

    #[test]
    fn stored_pin_is_honored_while_the_account_exists() {
        let mut list = accounts("default");
        let pinned = list[3].id;
        assert_eq!(effective(&list, Some(pinned)), Some(pinned));

        list.retain(|a| a.id != pinned);
        assert_eq!(effective(&list, Some(pinned)), Some(list[0].id));
        assert_eq!(parse_pin(""), None);
        assert_eq!(parse_pin("4"), Some(AccountId::new(4)));
    }

    #[test]
    fn style_defaults_to_percent_and_round_trips() {
        assert_eq!(MenuBarStyle::default(), MenuBarStyle::IconPercent);
        assert_eq!(MenuBarStyle::parse("nonsense"), MenuBarStyle::IconPercent);
        for style in [MenuBarStyle::Icon, MenuBarStyle::IconPercent] {
            assert_eq!(MenuBarStyle::parse(style.as_str()), style);
        }
    }

    #[test]
    fn status_text_shows_the_five_hour_percent() {
        let list = accounts("default");
        assert_eq!(text(&list[0]).as_deref(), Some("77%"));
        assert_eq!(status_text(Some(&list[0]), MenuBarStyle::Icon, false), None);
        // Only a 30-day window.
        assert_eq!(text(&list[5]).as_deref(), Some("100%"));
        // Only model groups: the first group's 5-hour window.
        let groups = accounts("antigravity");
        assert_eq!(text(&groups[0]).as_deref(), Some("20%"));
    }

    #[test]
    fn status_text_is_a_dash_without_a_current_reading() {
        let list = accounts("default");
        // Session expired, and rate limited.
        assert_eq!(text(&list[1]).as_deref(), Some("—"));
        assert_eq!(text(&list[4]).as_deref(), Some("—"));
        let mut fresh = list[0].clone();
        fresh.windows.clear();
        assert_eq!(text(&fresh).as_deref(), Some("—"));
        assert_eq!(
            status_text(None, MenuBarStyle::IconPercent, false).as_deref(),
            Some("—")
        );

        let cached = accounts("offline");
        assert_eq!(
            status_text(Some(&cached[0]), MenuBarStyle::IconPercent, true).as_deref(),
            Some("77%")
        );
    }
}

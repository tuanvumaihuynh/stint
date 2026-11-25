use std::sync::Arc;

use gpui_kit::base::{StyledExt, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Colorize, Icon, IconName, Sizable};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, Div, Entity, FontFeatures,
    Global, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions, div, px, relative, rgb, size,
};
use jiff::Timestamp;
use stint_core::{Pace, ProviderId, QuotaWindow};

use crate::app::{Quit, Refresh};
use crate::format;
use crate::quota::{FetchError, ProviderQuota, QuotaState};
use crate::screen;

/// The brand accent, the same in light and dark mode.
pub(crate) const ACCENT_EMERALD: u32 = 0x16_C4_7F;

const WIDTH: f64 = 340.0;
// The provider list scrolls past this height.
const MAX_LIST_HEIGHT: u32 = 640;
// Heights in points, used to size the window before its content is laid out.
const CHROME_HEIGHT: u32 = 78;
const SECTION_HEIGHT: u32 = 43;
const NOTICE_HEIGHT: u32 = 54;
const WINDOW_ROW_HEIGHT: u32 = 59;
const EMPTY_HEIGHT: u32 = 25;

// A provider not refreshed for this long is shown as stale.
const STALE_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(15);
// Usage at or above these percentages turns the bar amber, then red.
const WARN_AT: f64 = 75.0;
const DANGER_AT: f64 = 90.0;
// Burning this many points faster than an even pace also turns the bar amber.
const WARN_AHEAD_POINTS: f64 = 10.0;

// The two layers of the Dual-Arc mark. An SVG takes one color, so each color is its own layer.
const MARK_OUTER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none"><path d="M 4.05 13.95 A 7 7 0 1 1 15.34 11.96" stroke="black" stroke-width="1.85" stroke-linecap="round"/><circle cx="9" cy="9" r="1.05" fill="black"/></svg>"#;
const MARK_INNER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none"><path d="M 6.38 11.62 A 3.7 3.7 0 1 1 12.35 7.44" stroke="black" stroke-width="1.85" stroke-linecap="round"/></svg>"#;

/// The open popover, if any.
#[derive(Default)]
struct OpenPopover(Option<AnyWindowHandle>);

impl Global for OpenPopover {}

/// Opens the popover under the status item, or closes it when it is open.
pub(crate) fn toggle(quota: Entity<QuotaState>, cx: &mut App) {
    if let Some(handle) = cx.default_global::<OpenPopover>().0.take() {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        return;
    }

    let height = estimate_height(&quota.read(cx).providers);
    let anchor = screen::anchor_under_mouse(WIDTH);
    let origin = anchor
        .as_ref()
        .map_or_else(Default::default, |anchor| anchor.origin);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: size(Pixels::from(WIDTH), Pixels::from(f64::from(height))),
        })),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: anchor.and_then(|anchor| anchor.display),
        // Transparent so the rounded corners of the popover show.
        window_background: WindowBackgroundAppearance::Transparent,
        ..WindowOptions::default()
    };

    // An accessory app must activate itself for its window to take focus, which is what
    // lets a click elsewhere deactivate and close the popover.
    cx.activate(true);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        Theme::sync_system_appearance(Some(window), cx);
        cx.new(|cx| PopoverView::new(quota, window, cx))
    });
    match opened {
        Ok((handle, _)) => cx.set_global(OpenPopover(Some(handle))),
        Err(err) => eprintln!("stint: open popover: {err:#}"),
    }
}

fn estimate_height(providers: &[ProviderQuota]) -> u32 {
    let list: u32 = providers
        .iter()
        .map(|quota| {
            let windows = u32::try_from(quota.windows.len()).unwrap_or(u32::MAX);
            let rows = if windows == 0 && quota.error.is_none() {
                EMPTY_HEIGHT
            } else {
                WINDOW_ROW_HEIGHT.saturating_mul(windows)
            };
            let notice = if quota.error.is_some() {
                NOTICE_HEIGHT
            } else {
                0
            };
            // One more point for the separator above each section.
            SECTION_HEIGHT + notice + rows + 1
        })
        .sum();
    CHROME_HEIGHT.saturating_add(list.min(MAX_LIST_HEIGHT))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "percentages are within 0..=100, far inside f32 range"
)]
fn percent_f32(value: f64) -> f32 {
    value as f32
}

struct PopoverView {
    quota: Entity<QuotaState>,
    _subscriptions: Vec<Subscription>,
}

impl PopoverView {
    fn new(quota: Entity<QuotaState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&quota, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |_, window, cx| {
                if !window.is_window_active() {
                    cx.set_global(OpenPopover(None));
                    window.remove_window();
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
            }),
        ];
        Self {
            quota,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for PopoverView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Timestamp::now();
        let state = self.quota.read(cx);
        let refreshing = state.refreshing;
        let providers = state.providers.clone();
        let last_update = providers.iter().filter_map(|quota| quota.fetched_at).max();
        let theme = cx.theme();

        let mut sections: Vec<AnyElement> = Vec::with_capacity(providers.len() * 2);
        for (index, quota) in providers.iter().enumerate() {
            if index > 0 {
                sections.push(Separator::horizontal().into_any_element());
            }
            sections.push(provider_section(quota, now, cx).into_any_element());
        }

        v_flex()
            .size_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .child(header(cx))
            .child(Separator::horizontal())
            .child(
                v_flex()
                    .id("providers")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(sections),
            )
            .child(Separator::horizontal())
            .child(footer(last_update, now, refreshing, cx))
    }
}

fn header(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let mark = div()
        .relative()
        .size(px(16.))
        .child(
            Icon::default()
                .data(MARK_OUTER)
                .absolute()
                .size(px(16.))
                .text_color(theme.foreground),
        )
        .child(
            Icon::default()
                .data(MARK_INNER)
                .absolute()
                .size(px(16.))
                .text_color(accent()),
        );
    let legend_tick = div()
        .w(px(2.))
        .h(px(8.))
        .rounded(px(1.))
        .bg(theme.foreground);

    h_flex()
        .px(px(14.))
        .py(px(10.))
        .justify_between()
        .items_center()
        .child(
            h_flex().gap(px(7.)).items_center().child(mark).child(
                div()
                    .text_size(px(14.))
                    .line_height(px(18.))
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child("stint"),
            ),
        )
        .child(
            h_flex()
                .gap(px(5.))
                .items_center()
                .child(legend_tick)
                .child(
                    caption()
                        .text_color(theme.muted_foreground)
                        .child("even pace"),
                ),
        )
}

fn provider_section(quota: &ProviderQuota, now: Timestamp, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let stale = quota
        .fetched_at
        .is_some_and(|at| now.duration_since(at) > STALE_AFTER);
    let opacity = if quota.error.is_some() {
        0.60
    } else if stale {
        0.75
    } else {
        1.0
    };

    let mut section = v_flex().px(px(14.)).py(px(12.)).gap(px(10.)).child(
        h_flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .font_semibold()
                    .text_color(theme.foreground)
                    .child(provider_name(quota.provider)),
            )
            .children(section_status(quota, now, stale, cx)),
    );

    if let Some(error) = &quota.error {
        section = section.child(notice(quota, error, cx));
    }

    if quota.windows.is_empty() {
        if quota.error.is_none() {
            section = section.child(
                caption()
                    .text_color(theme.muted_foreground)
                    .child("No usage yet"),
            );
        }
        return section;
    }

    section.child(
        v_flex().gap(px(10.)).opacity(opacity).children(
            quota
                .windows
                .iter()
                .enumerate()
                .map(|(index, window)| window_row(quota.provider, index, window, now, cx)),
        ),
    )
}

/// The right side of a provider's title: why its numbers may be out of date.
fn section_status(
    quota: &ProviderQuota,
    now: Timestamp,
    stale: bool,
    cx: &App,
) -> Option<AnyElement> {
    let muted = cx.theme().muted_foreground;
    if quota.error == Some(FetchError::NotSignedIn) {
        return Some(
            caption()
                .text_color(muted)
                .child("Not signed in")
                .into_any_element(),
        );
    }
    let updated = format::updated_ago(now, quota.fetched_at?);
    if quota.error.is_some() {
        return Some(
            caption()
                .text_color(muted)
                .font_features(tabular_numbers())
                .child(updated)
                .into_any_element(),
        );
    }
    if !stale {
        return None;
    }
    let warning = ink(Tone::Warning, cx);
    Some(
        h_flex()
            .gap(px(4.))
            .items_center()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(px(12.))
                    .text_color(warning),
            )
            .child(
                caption()
                    .text_color(warning)
                    .font_features(tabular_numbers())
                    .child(updated),
            )
            .into_any_element(),
    )
}

fn notice(quota: &ProviderQuota, error: &FetchError, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let name = provider_name(quota.provider);
    let (tone, icon, text) = match error {
        FetchError::NotSignedIn => (
            Tone::Muted,
            IconName::Info,
            format!(
                "Sign in with {} to show usage here.",
                cli_name(quota.provider)
            ),
        ),
        FetchError::TokenExpired => (
            Tone::Warning,
            IconName::TriangleAlert,
            format!("Session expired. {}", sign_in_again(quota.provider)),
        ),
        FetchError::KeychainLocked => (
            Tone::Warning,
            IconName::TriangleAlert,
            "Keychain is locked. Unlock your login keychain, then refresh.".to_owned(),
        ),
        FetchError::RateLimited {
            retry_after: Some(wait),
        } => (
            Tone::Warning,
            IconName::TriangleAlert,
            format!("Rate limited. Try again in {}.", format::duration(*wait)),
        ),
        FetchError::RateLimited { retry_after: None } => (
            Tone::Warning,
            IconName::TriangleAlert,
            format!("Rate limited by {name}. Will retry shortly."),
        ),
        FetchError::Network => (
            Tone::Danger,
            IconName::CircleAlert,
            format!("Can't reach {name}. Check your connection and refresh."),
        ),
        FetchError::Status(code) if quota.windows.is_empty() => (
            Tone::Danger,
            IconName::CircleAlert,
            format!("{name} returned HTTP {code}."),
        ),
        FetchError::Status(code) => (
            Tone::Danger,
            IconName::CircleAlert,
            format!("{name} returned HTTP {code}. Showing last saved usage."),
        ),
        FetchError::Other(message) => (Tone::Danger, IconName::CircleAlert, message.clone()),
    };

    let (fill, border) = match tone {
        Tone::Muted => (theme.muted, theme.border),
        Tone::Warning => (theme.warning.opacity(0.12), theme.warning.opacity(0.28)),
        Tone::Danger => (theme.danger.opacity(0.10), theme.danger.opacity(0.26)),
    };
    let color = ink(tone, cx);

    h_flex()
        .gap(px(6.))
        .items_start()
        .px(px(8.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .bg(fill)
        .child(Icon::new(icon).size(px(12.)).mt(px(1.5)).text_color(color))
        .child(caption().flex_1().text_color(color).child(text))
}

fn window_row(
    provider: ProviderId,
    index: usize,
    window: &QuotaWindow,
    now: Timestamp,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let used = window.used.get();
    let pace = Pace::of(window, now);

    let percent_color = if used >= DANGER_AT {
        ink(Tone::Danger, cx)
    } else if used >= WARN_AT {
        ink(Tone::Warning, cx)
    } else {
        theme.foreground
    };
    let pace_text = pace.as_ref().map(|pace| {
        let tone = pace_tone(used, pace);
        let text = caption()
            .font_features(tabular_numbers())
            .child(format::pace(now, used, pace));
        match tone {
            Tone::Muted => text.text_color(theme.muted_foreground),
            _ => text.font_medium().text_color(ink(tone, cx)),
        }
    });
    let reset_text = window.resets_at.map(|resets_at| {
        caption()
            .text_color(theme.muted_foreground)
            .font_features(tabular_numbers())
            .child(format::resets_in(now, resets_at))
    });

    v_flex()
        .gap(px(4.))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .text_xs()
                .child(
                    div()
                        .font_medium()
                        .child(format::window_label(window.kind, window.scope.as_deref())),
                )
                .child(
                    div()
                        .font_semibold()
                        .font_features(tabular_numbers())
                        .text_color(percent_color)
                        .child(format!("{used:.0}%")),
                ),
        )
        .child(usage_bar(provider, index, used, pace.as_ref(), cx))
        .child(
            h_flex()
                .justify_between()
                .gap(px(8.))
                .child(div().children(pace_text))
                .children(reset_text),
        )
}

/// The usage bar, with a tick where an even burn would be by now.
fn usage_bar(
    provider: ProviderId,
    index: usize,
    used: f64,
    pace: Option<&Pace>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let color = if used >= DANGER_AT {
        theme.danger
    } else if used >= WARN_AT || pace.is_some_and(|pace| pace.delta >= WARN_AHEAD_POINTS) {
        theme.warning
    } else {
        accent()
    };

    let bar = div()
        .relative()
        .w_full()
        .h(px(10.))
        .flex()
        .items_center()
        .child(
            Progress::new(SharedString::from(format!("{provider}-{index}")))
                .small()
                .value(percent_f32(used))
                .color(color),
        );
    let Some(pace) = pace else {
        return bar;
    };

    let expected = pace.expected.get();
    let tooltip = SharedString::from(format!(
        "Used {used:.0}% vs {expected:.0}% expected for even burn"
    ));
    bar.child(
        div()
            .id(SharedString::from(format!("{provider}-{index}-pace")))
            .absolute()
            .top_0()
            .left(relative((percent_f32(expected) / 100.0).clamp(0.01, 0.99)))
            .ml(px(-2.))
            .w(px(4.))
            .h(px(10.))
            .flex()
            .justify_center()
            // The popover-colored gap keeps the tick readable over the bar fill.
            .bg(theme.popover)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(
                div()
                    .w(px(2.))
                    .h_full()
                    .rounded(px(1.))
                    .bg(theme.foreground),
            ),
    )
}

fn footer(
    last_update: Option<Timestamp>,
    now: Timestamp,
    refreshing: bool,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let status = if refreshing {
        "Refreshing…".to_owned()
    } else {
        last_update.map_or_else(
            || "Not updated yet".to_owned(),
            |at| format::updated_ago(now, at),
        )
    };

    h_flex()
        .px(px(14.))
        .py(px(8.))
        .justify_between()
        .items_center()
        .bg(theme.status_bar)
        .child(
            caption()
                .text_color(theme.muted_foreground)
                .font_features(tabular_numbers())
                .child(status),
        )
        .child(
            h_flex()
                .gap(px(4.))
                .child(
                    Button::new("refresh")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(IconName::RefreshCw).text_color(accent()))
                        .loading_icon(Icon::new(IconName::Loader).text_color(accent()))
                        .loading(refreshing)
                        .label("Refresh")
                        .tooltip_with_action("Refresh usage", &Refresh, None)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx)),
                )
                .child(
                    Button::new("quit")
                        .xsmall()
                        .ghost()
                        .label("Quit")
                        .tooltip_with_action("Quit Stint", &Quit, None)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Quit), cx)),
                ),
        )
}

/// How loudly a piece of text speaks.
#[derive(Clone, Copy)]
enum Tone {
    Muted,
    Warning,
    Danger,
}

fn pace_tone(used: f64, pace: &Pace) -> Tone {
    if used >= 100.0 {
        Tone::Danger
    } else if pace.delta >= WARN_AHEAD_POINTS {
        if used >= DANGER_AT {
            Tone::Danger
        } else {
            Tone::Warning
        }
    } else {
        Tone::Muted
    }
}

/// Text color for a tone. Warning and danger are darkened in light mode so small text
/// keeps enough contrast on white.
fn ink(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    let dark = theme.is_dark();
    match tone {
        Tone::Muted => theme.muted_foreground,
        Tone::Warning if dark => theme.warning,
        Tone::Warning => theme.warning.darken(0.35),
        Tone::Danger if dark => theme.danger,
        Tone::Danger => theme.danger.darken(0.12),
    }
}

fn accent() -> Hsla {
    rgb(ACCENT_EMERALD).into()
}

/// 11 pt secondary text: pace, resets, notices, and the footer.
fn caption() -> Div {
    div().text_size(px(11.)).line_height(px(15.))
}

/// Tabular figures, so numbers keep their width as they tick.
fn tabular_numbers() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini",
    }
}

fn cli_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Claude Code",
        ProviderId::Codex => "Codex CLI",
        ProviderId::Gemini => "Gemini CLI",
    }
}

fn sign_in_again(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Open Claude Code to sign in again.",
        ProviderId::Codex => "Run codex to sign in again.",
        ProviderId::Gemini => "Run gemini to sign in again.",
    }
}

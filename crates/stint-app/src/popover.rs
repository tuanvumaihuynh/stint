use gpui_kit::base::{StyledExt, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::theme::Theme;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Global, Hsla, IntoElement,
    ParentElement, Pixels, Render, SharedString, Styled, Subscription, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, size,
};
use jiff::Timestamp;
use stint_core::{Pace, ProviderId, QuotaWindow};

use crate::app::{Quit, Refresh};
use crate::format;
use crate::quota::{FetchError, ProviderQuota, QuotaState};
use crate::screen;

const WIDTH: f64 = 340.0;
// Heights in points, used to size the window before its content is laid out.
const CHROME_HEIGHT: u32 = 96;
const PROVIDER_HEADER_HEIGHT: u32 = 36;
const WINDOW_ROW_HEIGHT: u32 = 52;
const NOTE_HEIGHT: u32 = 24;
// A provider not refreshed for this long is shown as stale.
const STALE_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(15);

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
        window_background: WindowBackgroundAppearance::Opaque,
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
    let rows: u32 = providers
        .iter()
        .map(|quota| {
            let windows = u32::try_from(quota.windows.len().max(1)).unwrap_or(u32::MAX);
            let note = if quota.error.is_some() {
                NOTE_HEIGHT
            } else {
                0
            };
            PROVIDER_HEADER_HEIGHT + WINDOW_ROW_HEIGHT.saturating_mul(windows) + note
        })
        .sum();
    CHROME_HEIGHT.saturating_add(rows)
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

        v_flex()
            .size_full()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .text_sm()
            .child(
                v_flex().flex_1().px_3().py_2().gap_3().children(
                    providers
                        .iter()
                        .map(|quota| provider_section(quota, now, cx)),
                ),
            )
            .child(footer(last_update, now, refreshing, cx))
    }
}

fn provider_section(quota: &ProviderQuota, now: Timestamp, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let stale = quota
        .fetched_at
        .is_some_and(|at| now.duration_since(at) > STALE_AFTER);
    let status = match (quota.fetched_at, stale) {
        (Some(at), true) => Some(format::updated_ago(now, at)),
        _ => None,
    };

    let mut section = v_flex().gap_2().child(
        h_flex()
            .justify_between()
            .child(div().font_semibold().child(provider_name(quota.provider)))
            .children(status.map(|text| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(text)
            })),
    );

    if let Some(error) = &quota.error {
        section = section.child(
            div()
                .text_xs()
                .text_color(error_color(error, cx))
                .child(error_text(quota.provider, error)),
        );
    }

    if quota.windows.is_empty() && quota.error.is_none() {
        section = section.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("No usage yet"),
        );
    }

    section.children(
        quota
            .windows
            .iter()
            .enumerate()
            .map(|(index, window)| window_row(quota.provider, index, window, now, cx)),
    )
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
    let detail = window.resets_at.map(|resets_at| {
        let mut text = format::resets_in(now, resets_at);
        if let Some(pace) = Pace::of(window, now) {
            text.push_str(" · ");
            text.push_str(&format::pace(now, &pace));
        }
        text
    });

    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(format::window_label(window.kind, window.scope.as_deref()))
                .child(div().font_semibold().child(format!("{used:.0}%"))),
        )
        .child(
            Progress::new(SharedString::from(format!("{provider}-{index}")))
                .value(percent_f32(used))
                .color(usage_color(used, cx)),
        )
        .children(detail.map(|text| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        }))
}

fn footer(
    last_update: Option<Timestamp>,
    now: Timestamp,
    refreshing: bool,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let updated = last_update.map_or_else(
        || "not updated yet".to_owned(),
        |at| format::updated_ago(now, at),
    );

    h_flex()
        .justify_between()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(updated),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("refresh")
                        .label("Refresh")
                        .loading(refreshing)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx)),
                )
                .child(
                    Button::new("quit")
                        .label("Quit")
                        .ghost()
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Quit), cx)),
                ),
        )
}

fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini",
    }
}

fn error_text(provider: ProviderId, error: &FetchError) -> String {
    let cli = match provider {
        ProviderId::Claude => "Claude Code",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini CLI",
    };
    match error {
        FetchError::NotSignedIn => format!("Not signed in. Sign in with {cli}."),
        FetchError::TokenExpired => format!("Session expired. Open {cli} to renew it."),
        FetchError::KeychainLocked => "Keychain is locked. Unlock it to read the login.".to_owned(),
        FetchError::RateLimited => "Rate limited. Showing the last known usage.".to_owned(),
        FetchError::Network => "Offline. Showing the last known usage.".to_owned(),
        FetchError::Other(message) => message.clone(),
    }
}

fn error_color(error: &FetchError, cx: &App) -> Hsla {
    let theme = cx.theme();
    match error {
        FetchError::RateLimited | FetchError::Network => theme.warning,
        _ => theme.danger,
    }
}

// Usage at or above these percentages turns the bar amber, then red.
const WARN_AT: f64 = 75.0;
const DANGER_AT: f64 = 90.0;

fn usage_color(used: f64, cx: &App) -> Hsla {
    let theme = cx.theme();
    if used >= DANGER_AT {
        theme.danger
    } else if used >= WARN_AT {
        theme.warning
    } else {
        theme.progress_bar
    }
}

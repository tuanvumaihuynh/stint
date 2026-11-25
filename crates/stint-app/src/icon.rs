use std::f32::consts::PI;

use gpui_tray::Icon;

// 18 x 18 points at 2x, the menu bar's template size.
const SIZE: u16 = 36;
const SCALE: f64 = 2.0;
// Each pixel is sampled on a 4 x 4 grid for anti-aliased edges.
const SAMPLES: u16 = 4;

// Two arcs, outer for the 7-day window and inner for the 5-hour one, both open at the top
// right like a gauge. Radii and widths are in pixels on the 36 px canvas.
const ARCS: [Arc; 2] = [
    Arc {
        radius: 14.0,
        width: 3.4,
        gap_start: -0.30 * PI,
        gap_end: 0.05 * PI,
    },
    Arc {
        radius: 8.0,
        width: 3.4,
        gap_start: -0.30 * PI,
        gap_end: 0.45 * PI,
    },
];

struct Arc {
    radius: f32,
    width: f32,
    // The gap runs clockwise from `gap_start` to `gap_end`, in radians from 12 o'clock.
    gap_start: f32,
    gap_end: f32,
}

impl Arc {
    fn covers(&self, dx: f32, dy: f32) -> bool {
        let distance = dx.hypot(dy);
        if (distance - self.radius).abs() > self.width / 2.0 {
            return false;
        }
        // Clockwise angle from 12 o'clock, in -PI..PI.
        let angle = dx.atan2(-dy);
        !(self.gap_start..=self.gap_end).contains(&angle)
    }
}

/// The menu bar mark as a template image, which macOS tints for light and dark menu bars.
pub(crate) fn template() -> gpui_tray::Result<Icon> {
    let center = f32::from(SIZE) / 2.0;
    let mut rgba = Vec::with_capacity(usize::from(SIZE) * usize::from(SIZE) * 4);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let covered = samples(x, y)
                .filter(|&(sx, sy)| ARCS.iter().any(|arc| arc.covers(sx - center, sy - center)))
                .count();
            let alpha =
                u8::try_from(covered * 255 / usize::from(SAMPLES * SAMPLES)).unwrap_or(u8::MAX);
            rgba.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }

    Icon::from_rgba(rgba, u32::from(SIZE), u32::from(SIZE))
        .scale_factor(SCALE)
        .template(true)
        .build()
}

fn samples(x: u16, y: u16) -> impl Iterator<Item = (f32, f32)> {
    let step = 1.0 / f32::from(SAMPLES);
    (0..SAMPLES).flat_map(move |sy| {
        (0..SAMPLES).map(move |sx| {
            (
                f32::from(x) + (f32::from(sx) + 0.5) * step,
                f32::from(y) + (f32::from(sy) + 0.5) * step,
            )
        })
    })
}

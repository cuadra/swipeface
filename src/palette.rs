//! Randomised dark colour schemes.
//!
//! Every scheme is built in OKLCH so that lightness and contrast stay constant
//! no matter which hues the dice land on: the background is always a deep,
//! faintly tinted neutral, and the accents are always legible pastels related
//! to each other through a classic colour harmony.

use iced::Color;
use std::time::{SystemTime, UNIX_EPOCH};

/// A tiny xorshift generator; colour shuffling does not need more than this.
pub struct Rng(u64);

impl Rng {
    pub fn from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let mut rng = Self(nanos ^ 0x9E37_79B9_7F4A_7C15 | 1);
        for _ in 0..8 {
            rng.next();
        }
        rng
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A float in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() % items.len() as u64) as usize]
    }
}

/// Hue offsets (in degrees) of the second and third accent for each harmony.
const HARMONIES: &[(&str, f32, f32)] = &[
    ("analogous", 32.0, -32.0),
    ("complementary", 180.0, 24.0),
    ("split complementary", 150.0, 210.0),
    ("triadic", 120.0, 240.0),
    ("tetradic", 90.0, 180.0),
    ("monochrome", 0.0, 12.0),
];

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub bg: Color,
    /// Panels that sit on the background (sidebar, status bar).
    pub surface: Color,
    /// Elements that sit on a panel (cells, inputs, hovered rows).
    pub raised: Color,
    /// Hairlines and outlines.
    pub border: Color,
    /// Primary text.
    pub text: Color,
    /// Secondary text.
    pub muted: Color,
    /// Tertiary text, barely there.
    pub faint: Color,
    pub accent: Color,
    pub accent2: Color,
    pub accent3: Color,
    /// Text drawn on top of an accent fill.
    pub on_accent: Color,
    pub harmony: &'static str,
    pub hue: f32,
}

impl Palette {
    /// Rolls a new scheme. When `previous` is given, the new base hue is kept
    /// well away from it so that every shuffle is visibly different.
    pub fn random(rng: &mut Rng, previous: Option<f32>) -> Self {
        let hue = loop {
            let hue = rng.range(0.0, 360.0);
            match previous {
                Some(prev) if hue_distance(hue, prev) < 50.0 => continue,
                _ => break hue,
            }
        };

        let (harmony, second, third) = rng.pick(HARMONIES);
        let monochrome = harmony == "monochrome";

        // How strongly the neutrals are tinted with the base hue.
        let tint = rng.range(0.012, 0.028);
        let depth = rng.range(0.150, 0.175);
        let chroma = rng.range(0.105, 0.150);
        let light = rng.range(0.78, 0.83);

        let (c2, l2, c3, l3) = if monochrome {
            (chroma * 0.55, light + 0.07, chroma * 0.8, light - 0.14)
        } else {
            (chroma, light, chroma * 0.9, light + 0.02)
        };

        Self {
            bg: oklch(depth, tint, hue),
            surface: oklch(depth + 0.035, tint * 1.1, hue),
            raised: oklch(depth + 0.075, tint * 1.25, hue),
            border: oklch(depth + 0.135, tint * 1.4, hue),
            text: oklch(0.935, 0.012, hue),
            muted: oklch(0.70, tint * 1.3, hue),
            faint: oklch(0.50, tint * 1.3, hue),
            accent: oklch(light, chroma, hue),
            accent2: oklch(l2, c2, hue + second),
            accent3: oklch(l3, c3, hue + third),
            on_accent: oklch(0.17, 0.03, hue),
            harmony,
            hue,
        }
    }
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Converts an OKLCH colour to sRGB, reducing chroma until it fits the gamut.
pub fn oklch(lightness: f32, chroma: f32, hue: f32) -> Color {
    let hue = hue.rem_euclid(360.0).to_radians();
    let mut chroma = chroma;

    for _ in 0..40 {
        let (r, g, b) = oklab_to_linear(lightness, chroma * hue.cos(), chroma * hue.sin());
        let in_gamut = [r, g, b].iter().all(|c| (-0.0005..=1.0005).contains(c));

        if in_gamut || chroma < 0.001 {
            return Color::from_rgb(encode(r), encode(g), encode(b));
        }
        chroma *= 0.93;
    }

    let (r, g, b) = oklab_to_linear(lightness, 0.0, 0.0);
    Color::from_rgb(encode(r), encode(g), encode(b))
}

fn oklab_to_linear(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;

    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

    (
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    )
}

/// Linear light to gamma-encoded sRGB.
fn encode(channel: f32) -> f32 {
    let c = channel.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// `color` with its alpha replaced.
pub fn alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

/// Linear blend from `a` to `b`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

//! Dusk: a procedural sunset over long-exposure water, with the white QR card, its shadow
//! and the frosted glass widgets baked in at their `DuskLayout` positions, a blurred copy
//! for the start and error screens, and the settings panel's frosted plate.

use slint::{ComponentHandle, Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer};

use super::*;
use crate::{DuskLayout, Faucet, FaucetApp, Layout};

/// The sky from zenith (0) down to the horizon (1)
const SKY: [(f32, Rgb); 6] = [
    (0.00, rgb(0x05, 0x07, 0x1a)),
    (0.38, rgb(0x0f, 0x14, 0x36)),
    (0.62, rgb(0x2a, 0x1e, 0x55)),
    (0.80, rgb(0x5e, 0x2b, 0x62)),
    (0.93, rgb(0xb2, 0x4a, 0x45)),
    (1.00, rgb(0xf0, 0x84, 0x3d)),
];
const SUN: Rgb = [1.0, 0.55, 0.22];
const HORIZON: Rgb = [1.0, 0.82, 0.6];
const SEA_NEAR: Rgb = rgb(0x1e, 0x12, 0x30);
const SEA_FAR: Rgb = rgb(0x04, 0x05, 0x0b);
/// Long-exposure water: soft horizontal bands, packed tighter toward the horizon
fn streaks(x: f32, depth: f32) -> f32 {
    let u = (depth + 6.0).ln() * 9.0 + x * 0.0015;
    (0.5 + 0.28 * u.sin() + 0.14 * (u * 2.3 + 1.7).sin() + 0.08 * (u * 5.1 + 0.4).sin()).clamp(0.0, 1.0)
}

/// A sun setting over long-exposure water, with a few stars high up
fn wallpaper(w: usize, h: usize) -> Canvas {
    let (wf, hf) = (w as f32, h as f32);
    let horizon = hf * 0.6;
    let sun_x = wf * 0.5;
    let glow = |d: f32| 0.9 * gauss(d, 190.0) + 0.6 * gauss(d, 64.0) + 0.22 * gauss(d, 420.0);

    let mut canvas = Canvas::from_fn(w, h, |x, y| {
        let dx = x - sun_x;
        let mut c = if y < horizon {
            let d = (dx * dx + ((y - horizon) * 1.5).powi(2)).sqrt();
            add(gradient(&SKY, y / horizon), scale(SUN, glow(d)))
        } else {
            let s = (y - horizon) / (hf - horizon);
            let depth = s.powf(0.7);
            // The sky mirrored in the water, fading with depth
            let mirrored = gradient(&SKY, 1.0 - (y - horizon) / horizon * 1.8);
            let mut sea = add(mix(SEA_NEAR, SEA_FAR, depth), scale(mirrored, 0.45 * (1.0 - depth).powi(2)));
            let band = streaks(x, y - horizon);
            // The sun's path across the water, widening toward the viewer
            let path = gauss(dx, 50.0 + 300.0 * s) * (0.95 * (1.0 - s).powf(1.4) + 0.06);
            sea = add(sea, scale(SUN, path * (0.45 + 0.75 * band)));
            scale(sea, 0.8 + 0.35 * band)
        };
        // A thin bright horizon, strongest under the sun
        c = add(c, scale(HORIZON, gauss(y - horizon, 1.8) * (0.25 + 0.9 * gauss(dx, 220.0))));
        let (vx, vy) = ((x - wf / 2.0) / (wf / 2.0), (y - hf / 2.0) / (hf / 2.0));
        scale(c, 1.0 - 0.22 * (vx * vx + vy * vy).powf(1.5).min(1.0)).map(tone)
    });

    let mut rng = Rng(0x2545_f491);
    let top = horizon * 0.62;
    for _ in 0..90 {
        let (sx, sy) = (rng.next() * wf, rng.next().powf(1.6) * top);
        let brightness = (0.25 + 0.75 * rng.next().powi(3)) * (1.0 - sy / top).sqrt();
        let size = 0.6 + 0.9 * rng.next();
        for y in (sy as isize - 3).max(0)..=(sy as isize + 3).min(h as isize - 1) {
            for x in (sx as isize - 3).max(0)..=(sx as isize + 3).min(w as isize - 1) {
                let d = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt();
                let i = y as usize * w + x as usize;
                canvas.px[i] = add(canvas.px[i], scale([0.85, 0.9, 1.0], brightness * gauss(d, size)));
            }
        }
    }
    canvas
}

/// Bright wallpaper values roll off smoothly above 0.85 instead of clipping flat
fn tone(v: f32) -> f32 {
    if v <= 0.85 { v.max(0.0) } else { 0.85 + 0.15 * ((v - 0.85) / 0.15).tanh() }
}

/// Where the baked shapes go, read from the UI's `Layout` so the two never drift apart
pub struct Geometry {
    size: usize,
    card: Rect,
    widgets: [Rect; 2],
    sheet_top: f32,
    sheet_radius: f32,
}

impl Geometry {
    pub fn of(app: &FaucetApp) -> Geometry {
        let l = app.global::<DuskLayout>();
        let widget = |x: f32| Rect { x, y: l.get_widget_y(), w: l.get_widget_w(), h: l.get_widget_h(), r: l.get_widget_radius() };
        Geometry {
            size: app.global::<Layout>().get_screen() as usize,
            card: Rect { x: l.get_card_x(), y: l.get_card_y(), w: l.get_card_size(), h: l.get_card_size(), r: l.get_card_radius() },
            widgets: [widget(l.get_gutter()), widget(app.global::<Layout>().get_screen() - l.get_gutter() - l.get_widget_w())],
            sheet_top: l.get_sheet_y(),
            sheet_radius: l.get_sheet_radius(),
        }
    }
}

pub struct Backdrops {
    main: SharedPixelBuffer<Rgb8Pixel>,
    soft: SharedPixelBuffer<Rgb8Pixel>,
    sheet: SharedPixelBuffer<Rgba8Pixel>,
}

impl Backdrops {
    pub fn install(self, faucet: &Faucet) {
        faucet.set_dusk_backdrop(slint::Image::from_rgb8(self.main));
        faucet.set_dusk_soft(slint::Image::from_rgb8(self.soft));
        faucet.set_dusk_sheet(slint::Image::from_rgba8(self.sheet));
    }
}

pub fn paint(g: &Geometry) -> Backdrops {
    let n = g.size;
    let sharp = wallpaper(n, n);

    let mut main = sharp.clone();
    main.shadow(g.card, 24.0, 30.0, 0.45);
    main.fill(g.card, WHITE);
    let behind = sharp.blurred(2);
    for widget in g.widgets {
        main.shadow(widget, 20.0, 18.0, 0.25);
        main.glass(&behind, widget);
    }

    let soft = sharp.blurred(12).map(|c| scale(c, 0.5));

    Backdrops { main: to_rgb8(&main), soft: to_rgb8(&soft), sheet: sheet(&sharp, g.sheet_top, g.sheet_radius) }
}

/// The owner panel: heavily frosted wallpaper under a dark tint, rounded at the top,
/// with light along the top edge
fn sheet(sharp: &Canvas, top: f32, radius: f32) -> SharedPixelBuffer<Rgba8Pixel> {
    let frost = sharp.blurred(30).map(|c| mix(scale(saturate(c, 1.5), 0.7), BLACK, 0.7));
    let (w, h) = (sharp.w, sharp.h - top as usize);
    // Extends below the plate so only the top corners are rounded
    let plate = Rect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 + 2.0 * radius, r: radius };
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
    let mut rng = Rng(0x68e3_1da4);
    for (i, dst) in buf.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let d = plate.sdf(px, py);
        let inside = (-d).max(0.0);
        let mut c = frost.at(x, y + top as usize);
        let rim = (1.0 - inside / 2.5).max(0.0);
        let up = (-plate.normal(px, py).1).max(0.0);
        c = add(c, scale(WHITE, rim * (0.075 + 0.25 * up * up)));
        if inside < 1.0 {
            c = mix(c, WHITE, 0.08 * (1.0 - inside));
        }
        let [r, g, b] = quantise(c, &mut rng);
        *dst = Rgba8Pixel { r, g, b, a: ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8 };
    }
    buf
}

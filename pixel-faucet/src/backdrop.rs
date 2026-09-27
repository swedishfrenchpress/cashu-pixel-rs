//! The faucet's backgrounds, painted once at startup.
//!
//! Slint's software renderer draws no blur, no shadows and no gradients on rounded shapes,
//! so everything soft is painted here into full-screen images:
//! - `main`: a procedural wallpaper (a sun setting over long-exposure water) with the white
//!   QR card, its shadow and the frosted glass widgets baked in at their `Layout` positions
//! - `soft`: the wallpaper blurred and dimmed, for the start and error screens
//! - `sheet`: the owner panel's frosted plate
//!
//! The UI draws only text, QR codes and solid shapes on top of these.

use slint::{ComponentHandle, Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer};

use crate::{Faucet, FaucetApp, Layout};

type Rgb = [f32; 3];

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

const WHITE: Rgb = [1.0, 1.0, 1.0];
const BLACK: Rgb = [0.0, 0.0, 0.0];
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
/// The glass tint's dark end (Umbrel-style top-lit glass: light at the top, smoky below)
const SMOKE: Rgb = rgb(12, 14, 18);

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn add(a: Rgb, b: Rgb) -> Rgb {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Rgb, k: f32) -> Rgb {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn saturate(c: Rgb, amount: f32) -> Rgb {
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    mix([luma; 3], c, amount)
}

fn gauss(d: f32, width: f32) -> f32 {
    (-(d / width).powi(2)).exp()
}

fn gradient(stops: &[(f32, Rgb)], t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            // Smoothstep between stops so no band edge shows
            let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            return mix(c0, c1, u * u * (3.0 - 2.0 * u));
        }
    }
    stops[stops.len() - 1].1
}

/// Complementary error function (Abramowitz and Stegun 7.1.26), for Gaussian shadow edges
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_6 + t * (-0.284_496_7 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let r = poly * (-z * z).exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// Small deterministic random numbers (xorshift), so every start paints the same picture
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// A rounded rectangle in screen pixels
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
}

impl Rect {
    /// Signed distance to the edge: negative inside
    fn sdf(&self, px: f32, py: f32) -> f32 {
        let qx = (px - self.x - self.w / 2.0).abs() - (self.w / 2.0 - self.r);
        let qy = (py - self.y - self.h / 2.0).abs() - (self.h / 2.0 - self.r);
        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - self.r
    }

    /// Outward unit normal of the nearest edge
    fn normal(&self, px: f32, py: f32) -> (f32, f32) {
        let dx = self.sdf(px + 0.5, py) - self.sdf(px - 0.5, py);
        let dy = self.sdf(px, py + 0.5) - self.sdf(px, py - 0.5);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        (dx / len, dy / len)
    }

    /// Pixel rows and columns within `pad` of the rectangle, clipped to the canvas
    fn span(&self, pad: f32, w: usize, h: usize) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let clip = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
        (
            clip(self.x - pad, w)..clip(self.x + self.w + pad + 1.0, w),
            clip(self.y - pad, h)..clip(self.y + self.h + pad + 1.0, h),
        )
    }
}

#[derive(Clone)]
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<Rgb>,
}

impl Canvas {
    fn from_fn(w: usize, h: usize, f: impl Fn(f32, f32) -> Rgb) -> Canvas {
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                px.push(f(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
        Canvas { w, h, px }
    }

    fn map(&self, f: impl Fn(Rgb) -> Rgb) -> Canvas {
        Canvas { w: self.w, h: self.h, px: self.px.iter().map(|&c| f(c)).collect() }
    }

    fn at(&self, x: usize, y: usize) -> Rgb {
        self.px[y * self.w + x]
    }

    /// Bilinear sample at a point in pixel coordinates, clamped to the edges
    fn sample(&self, x: f32, y: f32) -> Rgb {
        let fx = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        mix(mix(self.at(x0, y0), self.at(x1, y0), tx), mix(self.at(x0, y1), self.at(x1, y1), tx), ty)
    }

    /// Close to a Gaussian blur: three box-blur passes each way (sigma ≈ radius)
    fn blurred(&self, radius: usize) -> Canvas {
        let mut out = self.clone();
        let mut line = Vec::new();
        for _ in 0..3 {
            for y in 0..self.h {
                box_pass(&mut out.px, y * self.w, 1, self.w, radius, &mut line);
            }
            for x in 0..self.w {
                box_pass(&mut out.px, x, self.w, self.h, radius, &mut line);
            }
        }
        out
    }

    fn shadow(&mut self, rect: Rect, dy: f32, sigma: f32, alpha: f32) {
        let r = Rect { y: rect.y + dy, ..rect };
        let (xs, ys) = r.span(3.0 * sigma, self.w, self.h);
        for y in ys {
            for x in xs.clone() {
                let d = r.sdf(x as f32 + 0.5, y as f32 + 0.5);
                let a = alpha * 0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2));
                let i = y * self.w + x;
                self.px[i] = scale(self.px[i], 1.0 - a);
            }
        }
    }

    fn fill(&mut self, rect: Rect, color: Rgb) {
        let (xs, ys) = rect.span(1.0, self.w, self.h);
        for y in ys {
            for x in xs.clone() {
                let cover = (0.5 - rect.sdf(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0);
                let i = y * self.w + x;
                self.px[i] = mix(self.px[i], color, cover);
            }
        }
    }

    /// Clear glass over `behind`: the wallpaper shows through nearly sharp, bent near the
    /// rim like a thick lens (with a slight colour fringe), tinted light at the top and
    /// smoky at the bottom, with light catching the top edge and a faint hairline all round.
    fn glass(&mut self, behind: &Canvas, rect: Rect) {
        const BEVEL: f32 = 30.0;
        const BEND: f32 = 40.0;
        const FRINGE: f32 = 0.3;
        let (xs, ys) = rect.span(1.0, self.w, self.h);
        for y in ys {
            for x in xs.clone() {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = rect.sdf(px, py);
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover == 0.0 {
                    continue;
                }
                let inside = (-d).max(0.0);
                let (nx, ny) = rect.normal(px, py);
                let k = (1.0 - inside / BEVEL).max(0.0);
                let off = BEND * k * k;
                // Near the rim the lens pulls in the picture from further inside
                let tap = |spread: f32, ch: usize| behind.sample(px - nx * off * spread, py - ny * off * spread)[ch];
                let mut c = [tap(1.0 + FRINGE, 0), tap(1.0, 1), tap(1.0 - FRINGE, 2)];
                c = scale(saturate(c, 1.7), 0.84);
                let t = ((py - rect.y) / rect.h).clamp(0.0, 1.0);
                c = mix(c, WHITE, 0.08 * (1.0 - t));
                c = mix(c, SMOKE, 0.24 * t);
                let rim = (1.0 - inside / 2.5).max(0.0);
                let top = (-ny).max(0.0);
                c = add(c, scale(WHITE, rim * (0.05 + 0.22 * top * top)));
                if inside < 1.0 {
                    c = mix(c, WHITE, 0.10 * (1.0 - inside));
                }
                let i = y * self.w + x;
                self.px[i] = mix(self.px[i], c, cover);
            }
        }
    }
}

/// One box-blur pass along a row or column (`stride` 1 or the row width)
fn box_pass(px: &mut [Rgb], start: usize, stride: usize, len: usize, radius: usize, line: &mut Vec<Rgb>) {
    line.clear();
    line.extend((0..len).map(|i| px[start + i * stride]));
    let at = |i: isize| line[i.clamp(0, len as isize - 1) as usize];
    let r = radius as isize;
    let norm = 1.0 / (2 * radius + 1) as f32;
    let mut sum = [0.0f32; 3];
    for i in -r..=r {
        sum = add(sum, at(i));
    }
    for i in 0..len {
        px[start + i * stride] = scale(sum, norm);
        let (next, gone) = (at(i as isize + r + 1), at(i as isize - r));
        sum = [sum[0] + next[0] - gone[0], sum[1] + next[1] - gone[1], sum[2] + next[2] - gone[2]];
    }
}

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

/// Quantise with a little triangular noise: smooth dark gradients would otherwise band,
/// all the more so on a panel driven at 6 bits per channel. Pure white (the QR card) stays
/// clean for the best scanning contrast.
fn quantise(c: Rgb, rng: &mut Rng) -> [u8; 3] {
    let n = (rng.next() + rng.next() - 1.0) * 4.0;
    if c.iter().all(|&v| v >= 0.998) {
        return [255; 3];
    }
    c.map(|v| (v * 255.0 + n).round().clamp(0.0, 255.0) as u8)
}

fn to_rgb8(canvas: &Canvas) -> SharedPixelBuffer<Rgb8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(canvas.w as u32, canvas.h as u32);
    let mut rng = Rng(0x9e37_79b9);
    for (dst, &src) in buf.make_mut_slice().iter_mut().zip(&canvas.px) {
        let [r, g, b] = quantise(src, &mut rng);
        *dst = Rgb8Pixel { r, g, b };
    }
    buf
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
        let l = app.global::<Layout>();
        let widget = |x: f32| Rect { x, y: l.get_widget_y(), w: l.get_widget_w(), h: l.get_widget_h(), r: l.get_widget_radius() };
        Geometry {
            size: l.get_screen() as usize,
            card: Rect { x: l.get_card_x(), y: l.get_card_y(), w: l.get_card_size(), h: l.get_card_size(), r: l.get_card_radius() },
            widgets: [widget(l.get_gutter()), widget(l.get_screen() - l.get_gutter() - l.get_widget_w())],
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
        faucet.set_backdrop(slint::Image::from_rgb8(self.main));
        faucet.set_backdrop_soft(slint::Image::from_rgb8(self.soft));
        faucet.set_sheet_plate(slint::Image::from_rgba8(self.sheet));
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

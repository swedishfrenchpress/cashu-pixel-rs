//! The faucet's backgrounds, painted once at startup for each theme.
//!
//! Slint's software renderer draws no blur, no shadows and no gradients on rounded shapes,
//! so everything soft is painted here into full-screen images; the UI draws text, QR codes
//! and solid shapes on top. Both themes are painted at startup (the active one first), so
//! switching themes is instant.
//! - [`dusk`]: a procedural sunset with the QR card, glass widgets and shadows baked in
//! - [`berlin`]: the bitcoin++ Berlin market art, shaded so the text over it stays legible

pub mod berlin;
pub mod dusk;

use slint::{Rgb8Pixel, SharedPixelBuffer};

pub(super) type Rgb = [f32; 3];

pub(super) const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

pub(super) const WHITE: Rgb = [1.0, 1.0, 1.0];
pub(super) const BLACK: Rgb = [0.0, 0.0, 0.0];
/// The glass tint's dark end (Umbrel-style top-lit glass: light at the top, smoky below)
const SMOKE: Rgb = rgb(12, 14, 18);

pub(super) fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub(super) fn add(a: Rgb, b: Rgb) -> Rgb {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn scale(a: Rgb, k: f32) -> Rgb {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub(super) fn saturate(c: Rgb, amount: f32) -> Rgb {
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    mix([luma; 3], c, amount)
}

pub(super) fn gauss(d: f32, width: f32) -> f32 {
    (-(d / width).powi(2)).exp()
}

pub(super) fn gradient(stops: &[(f32, Rgb)], t: f32) -> Rgb {
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
pub(super) fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_6 + t * (-0.284_496_7 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let r = poly * (-z * z).exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// Small deterministic random numbers (xorshift), so every start paints the same picture
pub(super) struct Rng(pub(super) u32);

impl Rng {
    pub(super) fn next(&mut self) -> f32 {
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
pub(super) struct Canvas {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) px: Vec<Rgb>,
}

impl Canvas {
    pub(super) fn from_fn(w: usize, h: usize, f: impl Fn(f32, f32) -> Rgb) -> Canvas {
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                px.push(f(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
        Canvas { w, h, px }
    }

    pub(super) fn map(&self, f: impl Fn(Rgb) -> Rgb) -> Canvas {
        Canvas { w: self.w, h: self.h, px: self.px.iter().map(|&c| f(c)).collect() }
    }

    pub(super) fn at(&self, x: usize, y: usize) -> Rgb {
        self.px[y * self.w + x]
    }

    /// Bilinear sample at a point in pixel coordinates, clamped to the edges
    pub(super) fn sample(&self, x: f32, y: f32) -> Rgb {
        let fx = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        mix(mix(self.at(x0, y0), self.at(x1, y0), tx), mix(self.at(x0, y1), self.at(x1, y1), tx), ty)
    }

    /// Close to a Gaussian blur: three box-blur passes each way (sigma ≈ radius)
    pub(super) fn blurred(&self, radius: usize) -> Canvas {
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

    pub(super) fn shadow(&mut self, rect: Rect, dy: f32, sigma: f32, alpha: f32) {
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

    pub(super) fn fill(&mut self, rect: Rect, color: Rgb) {
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
    pub(super) fn glass(&mut self, behind: &Canvas, rect: Rect) {
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

impl Canvas {
    /// Decode an 8-bit RGB or RGBA PNG
    pub(super) fn from_png(bytes: &[u8]) -> Result<Canvas, String> {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let channels = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => return Err(format!("unsupported image format {:?}", other)),
        };
        let px = buf[..info.buffer_size()]
            .chunks_exact(channels)
            .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
            .collect();
        Ok(Canvas { w: info.width as usize, h: info.height as usize, px })
    }

    /// Like `map`, with each pixel's row
    pub(super) fn map_rows(&self, f: impl Fn(usize, Rgb) -> Rgb) -> Canvas {
        Canvas { w: self.w, h: self.h, px: self.px.iter().enumerate().map(|(i, &c)| f(i / self.w, c)).collect() }
    }
}

pub(super) fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Quantise with a little triangular noise: smooth dark gradients would otherwise band,
/// all the more so on a panel driven at 6 bits per channel. Pure white (the QR card) stays
/// clean for the best scanning contrast.
pub(super) fn quantise(c: Rgb, rng: &mut Rng) -> [u8; 3] {
    let n = (rng.next() + rng.next() - 1.0) * 4.0;
    if c.iter().all(|&v| v >= 0.998) {
        return [255; 3];
    }
    c.map(|v| (v * 255.0 + n).round().clamp(0.0, 255.0) as u8)
}

pub(super) fn to_rgb8(canvas: &Canvas) -> SharedPixelBuffer<Rgb8Pixel> {
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(canvas.w as u32, canvas.h as u32);
    let mut rng = Rng(0x9e37_79b9);
    for (dst, &src) in buf.make_mut_slice().iter_mut().zip(&canvas.px) {
        let [r, g, b] = quantise(src, &mut rng);
        *dst = Rgb8Pixel { r, g, b };
    }
    buf
}

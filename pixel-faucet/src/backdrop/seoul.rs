//! bitcoin++ Seoul: the event's persimmon-tree art (`ui/assets/seoul-persimmon.png`,
//! 720×720), taken down a little all over for the edition's "goes dark", much darker
//! behind the headline and toward the stats band, with the corners falling off (dithered,
//! so the dark gradients don't band on the 6-bit panel), and a blurred copy for the start
//! and error screens. Everything else in this theme is flat and drawn live by the UI.

use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};

use super::*;
use crate::{Faucet, FaucetApp, SeoulLayout};

const ART: &[u8] = include_bytes!("../../ui/assets/seoul-persimmon.png");

/// Where the text sits over the art, read from the UI's `Layout`
pub struct Geometry {
    /// Bottom edge of the headline area: the art is darkest above it
    headline_bottom: f32,
    /// Top of the stats band along the bottom
    band_top: f32,
}

impl Geometry {
    pub fn of(app: &FaucetApp) -> Geometry {
        let l = app.global::<SeoulLayout>();
        Geometry { headline_bottom: l.get_card_y(), band_top: l.get_band_y() }
    }
}

pub struct Backdrops {
    main: SharedPixelBuffer<Rgb8Pixel>,
    soft: SharedPixelBuffer<Rgb8Pixel>,
}

impl Backdrops {
    pub fn install(self, faucet: &Faucet) {
        faucet.set_seoul_backdrop(slint::Image::from_rgb8(self.main));
        faucet.set_seoul_soft(slint::Image::from_rgb8(self.soft));
    }
}

pub fn paint(g: &Geometry) -> Backdrops {
    let art = match Canvas::from_png(ART) {
        Ok(art) => art,
        Err(e) => {
            eprintln!("could not load the background art: {}", e);
            Canvas { w: 720, h: 720, px: vec![[0.08, 0.08, 0.08]; 720 * 720] }
        }
    };
    let (top, band) = (g.headline_bottom, g.band_top);
    let (w, h) = (art.w as f32, art.h as f32);
    let main = Canvas::from_fn(art.w, art.h, |x, y| {
        let c = art.at(x as usize, y as usize);
        // Darkest behind the headline and its Korean line, clearing below so the sun and
        // the tree show either side of the card
        let shade_top = 0.68 * (1.0 - smoothstep(top - 20.0, top + 150.0, y));
        // Darker toward the stats band, so the bright path doesn't compete with it
        let shade_bottom = 0.45 * smoothstep(band - 240.0, band, y);
        // The corners fall off
        let (dx, dy) = ((x - w / 2.0) / (w / 2.0), (y - h / 2.0) / (h / 2.0));
        let vignette = 0.3 * smoothstep(0.7, 1.4, (dx * dx + dy * dy).sqrt());
        scale(c, 0.82 * (1.0 - shade_top.max(shade_bottom)) * (1.0 - vignette))
    });
    let soft = art.blurred(10).map(|c| scale(c, 0.4));
    Backdrops { main: to_rgb8(&main), soft: to_rgb8(&soft) }
}

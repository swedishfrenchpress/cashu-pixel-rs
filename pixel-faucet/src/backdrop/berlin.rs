//! bitcoin++ Berlin: the event's market art (`ui/assets/berlin-market.png`, 720×720),
//! darkened behind the headline and a little toward the stats band (dithered, so the dark
//! gradients don't band on the 6-bit panel), and a blurred copy for the start and error
//! screens. Everything else in this theme is flat and drawn live by the UI.

use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};

use super::*;
use crate::{BerlinLayout, Faucet, FaucetApp};

const ART: &[u8] = include_bytes!("../../ui/assets/berlin-market.png");

/// Where the text sits over the art, read from the UI's `Layout`
pub struct Geometry {
    /// Bottom edge of the headline area: the art is darkest above it
    headline_bottom: f32,
    /// Top of the stats band along the bottom
    band_top: f32,
}

impl Geometry {
    pub fn of(app: &FaucetApp) -> Geometry {
        let l = app.global::<BerlinLayout>();
        Geometry { headline_bottom: l.get_card_y(), band_top: l.get_band_y() }
    }
}

pub struct Backdrops {
    main: SharedPixelBuffer<Rgb8Pixel>,
    soft: SharedPixelBuffer<Rgb8Pixel>,
}

impl Backdrops {
    pub fn install(self, faucet: &Faucet) {
        faucet.set_berlin_backdrop(slint::Image::from_rgb8(self.main));
        faucet.set_berlin_soft(slint::Image::from_rgb8(self.soft));
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
    let main = art.map_rows(|y, c| {
        let y = y as f32;
        // Darker behind the headline, clearing below it so the sunset and street show
        let shade_top = 0.58 * (1.0 - smoothstep(top - 20.0, top + 140.0, y));
        // A little darker toward the stats band, so the art's edges don't compete with it
        let shade_bottom = 0.35 * smoothstep(band - 200.0, band, y);
        scale(c, 1.0 - shade_top.max(shade_bottom))
    });
    let soft = art.blurred(10).map(|c| scale(c, 0.45));
    Backdrops { main: to_rgb8(&main), soft: to_rgb8(&soft) }
}

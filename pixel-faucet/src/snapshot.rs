//! `pixel-faucet --snapshot <dir>`: renders every screen with sample data to PNG files,
//! through the same software renderer the device uses, so the UI can be reviewed without
//! a Pi (for example inside the arm64 build container). Touches no wallet.

use std::cell::Cell;
use std::error::Error;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Rgb8Pixel, SharedString};

use crate::{backdrop, qr_image, Faucet, FaucetApp, Frames, Layout, Phase};

/// A platform whose clock only moves when a snapshot says so, so animations can be settled
struct SnapshotPlatform {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for SnapshotPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

/// Text shaped like a real token or invoice, so the sample QR codes have realistic density
fn sample(prefix: &str, alphabet: &[u8], len: usize) -> String {
    let mut seed = 0x1234_5678u32;
    let mut s = String::from(prefix);
    while s.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        s.push(alphabet[seed as usize % alphabet.len()] as char);
    }
    s
}

fn write_png(path: &Path, pixels: &[Rgb8Pixel], size: u32) -> Result<(), Box<dyn Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, size, size);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    encoder.write_header()?.write_image_data(&bytes)?;
    Ok(())
}

pub fn run(dir: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(SnapshotPlatform { window: window.clone(), clock: clock.clone() }))
        .map_err(|e| format!("{:?}", e))?;

    let app = FaucetApp::new()?;
    let size = 720u32;
    window.set_size(slint::PhysicalSize::new(size, size));
    let started = std::time::Instant::now();
    backdrop::paint(&backdrop::Geometry::of(&app)).install(&app.global::<Faucet>());
    println!("backdrops painted in {:?}", started.elapsed());
    app.show()?;

    let base64url = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bech32 = b"QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L";
    let layout = app.global::<Layout>();
    let (qr, small) = (layout.get_qr_size() as u32, layout.get_sheet_qr_size() as u32);
    let token_text = sample("cashuB", base64url, 546);
    let token = qr_image(&token_text, qr)?;
    let invoice_text = sample("LNBC10U1P", bech32, 290);
    let invoice = qr_image(&invoice_text, qr)?;
    let invoice_small = qr_image(&invoice_text, small)?;
    let frame = Frames::new(&token_text).and_then(|mut f| f.next()).ok_or("no animated frame")?;

    let f = app.global::<Faucet>();
    f.set_mint_host(SharedString::from("mint.minibits.cash"));
    f.set_drip(21);
    f.set_token_amount(21);
    f.set_balance(4200);
    f.set_drips_given(58);
    f.set_sats_given(1234);
    f.set_token_qr(slint::Image::from_rgb8(token.clone()));
    f.set_refill_qr(slint::Image::from_rgb8(invoice));
    f.set_refill_qr_small(slint::Image::from_rgb8(invoice_small));
    f.set_refill_amount(1000);

    let shoot = |name: &str| -> Result<(), Box<dyn Error>> {
        clock.set(clock.get() + Duration::from_secs(3));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size * size) as usize];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size as usize);
        });
        write_png(&dir.join(format!("{}.png", name)), &pixels, size)?;
        println!("{}", name);
        Ok(())
    };

    f.set_phase(Phase::Starting);
    shoot("1-starting")?;
    f.set_phase(Phase::Dripping);
    shoot("2-dripping")?;
    f.set_token_qr(slint::Image::from_rgb8(qr_image(&frame, qr)?));
    shoot("2b-dripping-animated-frame")?;
    f.set_token_qr(slint::Image::from_rgb8(token));
    f.set_message(SharedString::from("Can't reach the mint. Retrying…"));
    shoot("3-dripping-offline")?;
    f.set_message(SharedString::default());
    f.set_cooldown(60);
    f.set_phase(Phase::Claimed);
    shoot("4-claimed")?;
    f.set_cooldown_total(60);
    f.set_cooldown_left(42);
    f.set_phase(Phase::Cooling);
    shoot("4b-cooling")?;
    // The longest fact, to check it fits the card
    app.global::<crate::Trivia>().set_ticks(64);
    shoot("4c-cooling-fact-2")?;
    f.set_balance(12);
    f.set_phase(Phase::Empty);
    f.set_refill_ready(true);
    shoot("5-dry")?;
    f.set_refill_received(1000);
    f.set_balance(1012);
    shoot("6-refilled")?;
    f.set_refill_received(0);
    f.set_balance(4200);
    f.set_phase(Phase::Dripping);
    f.set_owner_open(true);
    shoot("7-owner")?;
    f.set_refill_amount(5000);
    f.set_refill_view(true);
    shoot("8-owner-refill")?;
    f.set_refill_received(5000);
    shoot("9-owner-refilled")?;
    f.set_owner_open(false);
    f.set_refill_view(false);
    f.set_refill_received(0);
    f.set_message(SharedString::from("The wallet didn't open: database is locked"));
    f.set_phase(Phase::Error);
    shoot("10-error")?;
    Ok(())
}

//! `cashu-pixel --snapshot <dir>`: renders screens with sample data to PNG files, through
//! the same software renderer the device uses, so the UI can be reviewed without a Pi
//! (for example inside the arm64 build container). Touches no wallet and no radio.

use std::cell::Cell;
use std::error::Error;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Rgb8Pixel, SharedString};

use crate::{BlePhase, MainApp, WalletState};

/// A platform whose clock only moves when a snapshot says so, so animations can be settled
/// or caught halfway
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

    let app = MainApp::new()?;
    let size = 720u32;
    window.set_size(slint::PhysicalSize::new(size, size));
    app.show()?;

    // Advances the clock by `after`, then renders
    let shoot = |name: &str, after: Duration| -> Result<(), Box<dyn Error>> {
        clock.set(clock.get() + after);
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
    let settled = Duration::from_secs(3);

    let s = app.global::<WalletState>();
    s.set_balance(SharedString::from("1234"));
    s.set_mint_url(SharedString::from("mint.minibits.cash"));
    shoot("1-home", settled)?;
    s.set_current_screen(1);
    shoot("2-receive", settled)?;

    s.set_current_screen(6);
    s.set_ble_phase(BlePhase::Starting);
    shoot("3-ble-starting", settled)?;
    s.set_ble_phase(BlePhase::Listening);
    s.set_ble_tick(20);
    shoot("4-ble-listening", settled)?;
    s.set_ble_tick(47);
    shoot("4b-ble-listening-later", Duration::ZERO)?;
    s.set_ble_linked(true);
    shoot("5-ble-linked", settled)?;
    s.set_ble_phase(BlePhase::Receiving);
    s.set_ble_bytes(412);
    shoot("6-ble-receiving", settled)?;

    s.set_ble_amount(21);
    s.set_ble_mint(SharedString::from("MINT.MINIBITS.CASH"));
    s.set_ble_memo(SharedString::from("Thanks for the coffee"));
    s.set_ble_redeemable(true);
    s.set_ble_phase(BlePhase::Token);
    // Halfway through the brackets opening into the card
    shoot("7a-ble-token-opening", Duration::from_millis(180))?;
    shoot("7-ble-token", settled)?;
    s.set_ble_phase(BlePhase::Redeeming);
    shoot("8-ble-redeeming", settled)?;
    s.set_received_amount(21);
    s.set_balance(SharedString::from("1255"));
    s.set_status(SharedString::from("NEW BALANCE 1255 SAT"));
    s.set_ble_phase(BlePhase::Received);
    shoot("9-ble-received", settled)?;

    s.set_status(SharedString::default());
    s.set_ble_amount(500);
    s.set_ble_mint(SharedString::from("MINT.COINOS.IO"));
    s.set_ble_memo(SharedString::default());
    s.set_ble_redeemable(false);
    s.set_ble_phase(BlePhase::Token);
    shoot("10-ble-other-mint", settled)?;
    s.set_ble_redeemable(true);
    s.set_ble_note(SharedString::from("TOKEN ALREADY SPENT"));
    s.set_ble_phase(BlePhase::Failed);
    shoot("11-ble-failed", settled)?;
    s.set_ble_linked(false);
    s.set_ble_bytes(0);
    s.set_status(SharedString::from("BLUETOOTH ERROR: NO BLUETOOTH ADAPTER FOUND"));
    s.set_ble_phase(BlePhase::Offline);
    shoot("12-ble-offline", settled)?;
    Ok(())
}

//! `cashu-pixel --snapshot <dir>`: renders screens with sample data to PNG files, through
//! the same software renderer the device uses, so the UI can be reviewed without a Pi
//! (for example inside the arm64 build container). Touches no wallet and no radio.

use std::cell::Cell;
use std::error::Error;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Rgb8Pixel, SharedString};

use crate::{BlePhase, MainApp, MintChoice, WalletState};

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
    // Halfway through the brackets opening into the card, then the amount counting up
    shoot("7a-ble-token-opening", Duration::from_millis(180))?;
    shoot("7b-ble-token-counting", Duration::from_millis(300))?;
    s.set_ble_tick(30);
    shoot("7-ble-token", settled)?;
    s.set_ble_phase(BlePhase::Redeeming);
    s.set_ble_tick(36);
    shoot("8-ble-redeeming", settled)?;
    s.set_received_amount(21);
    s.set_balance(SharedString::from("1255"));
    s.set_status(SharedString::from("NEW BALANCE 1255 SAT"));
    s.set_ble_phase(BlePhase::Received);
    // The shockwave on its way out
    shoot("9a-ble-received-wave", Duration::from_millis(160))?;
    shoot("9-ble-received", settled)?;

    s.set_status(SharedString::default());
    s.set_ble_amount(500);
    s.set_ble_mint(SharedString::from("MINT.COINOS.IO"));
    s.set_ble_memo(SharedString::default());
    s.set_ble_new_mint(true);
    s.set_ble_phase(BlePhase::Token);
    shoot("10-ble-new-mint", settled)?;
    s.set_ble_redeemable(false);
    s.set_ble_new_mint(false);
    s.set_ble_note(SharedString::from("THIS WALLET ONLY HOLDS SAT, NOT USD"));
    shoot("10b-ble-wrong-unit", settled)?;
    s.set_ble_redeemable(true);
    s.set_ble_note(SharedString::from("TOKEN ALREADY SPENT"));
    s.set_ble_phase(BlePhase::Failed);
    shoot("11-ble-failed", settled)?;
    s.set_ble_linked(false);
    s.set_ble_bytes(0);
    s.set_status(SharedString::from("BLUETOOTH ERROR: NO BLUETOOTH ADAPTER FOUND"));
    s.set_ble_phase(BlePhase::Offline);
    shoot("12-ble-offline", settled)?;

    s.set_status(SharedString::default());
    s.set_other_mints(2);
    s.set_current_screen(5);
    shoot("13-settings-mints", settled)?;

    // Home mint: the offered mints, then the others this wallet holds ecash from
    let choice = |host: &str, sats: i32, home: bool| MintChoice {
        host: SharedString::from(host),
        url: SharedString::from(format!("https://{}", host)),
        sats,
        home,
    };
    s.set_mints(slint::ModelRc::new(slint::VecModel::from(vec![
        choice("mint.minibits.cash", 934, true),
        choice("mint.macadamia.cash", 0, false),
        choice("antifiat.cash", 0, false),
        choice("mint.coinos.io", 200, false),
        choice("8333.space", 100, false),
    ])));
    s.set_current_screen(8);
    shoot("13b-home-mint", settled)?;
    s.set_mint_busy(true);
    s.set_status(SharedString::from("CHECKING ANTIFIAT.CASH..."));
    shoot("13c-home-mint-checking", settled)?;
    s.set_mint_busy(false);
    s.set_status(SharedString::from("HOME MINT NOW ANTIFIAT.CASH"));
    shoot("13d-home-mint-switched", settled)?;
    s.set_status(SharedString::default());

    s.set_current_screen(7);
    shoot("14-reset", settled)?;
    // A finger on HOLD TO ERASE: let go early, then halfway through a second press, then past
    // its end, which goes Home
    let tick = Duration::from_millis(50);
    let hold = |ticks: u32| {
        for _ in 0..ticks {
            clock.set(clock.get() + tick);
            slint::platform::update_timers_and_animations();
        }
    };
    let finger = slint::LogicalPosition::new(532.0, 608.0);
    window.dispatch_event(WindowEvent::PointerPressed { position: finger, button: PointerEventButton::Left });
    hold(20);
    window.dispatch_event(WindowEvent::PointerReleased { position: finger, button: PointerEventButton::Left });
    hold(2);
    shoot("14a-reset-let-go", Duration::ZERO)?;
    window.dispatch_event(WindowEvent::PointerPressed { position: finger, button: PointerEventButton::Left });
    hold(30);
    shoot("14b-reset-holding", Duration::ZERO)?;
    hold(40);
    window.dispatch_event(WindowEvent::PointerReleased { position: finger, button: PointerEventButton::Left });
    shoot("14c-reset-erasing", Duration::ZERO)?;
    Ok(())
}

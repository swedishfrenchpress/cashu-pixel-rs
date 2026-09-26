use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdk::nuts::CurrencyUnit;
use cdk::nuts::nut00::PaymentMethod;
use cdk::nuts::{MintQuoteState, TokenUrEncoder};
use cdk::wallet::{SendOptions, Wallet};
use cdk::Amount;
use slint::{Rgb8Pixel, SharedPixelBuffer, SharedString};
use tokio::sync::Mutex;

slint::include_modules!();

const MINT_URL: &str = "https://mint.minibits.cash/Bitcoin";

/// Tokens up to this many characters fit one comfortably scannable static QR;
/// longer ones are shown as an animated QR (NUT-16).
const STATIC_QR_MAX_LEN: usize = 320;
/// Bytes of token per animated frame; small fragments keep each frame sparse.
const UR_FRAGMENT_LEN: usize = 100;
/// Animated QR frame interval (5 frames per second).
const QR_FRAME_MS: u64 = 200;
/// How often an open invoice is checked for payment.
const PAYMENT_POLL_MS: u64 = 2000;

struct AppWallet {
    wallet: Wallet,
    current_quote: Option<cdk::wallet::MintQuote>,
}

/// Render a QR code as dark modules on a light field with a 4-module quiet zone,
/// one pixel per module. The UI scales it up with nearest-neighbour sampling.
fn qr_pixels(data: &str) -> Result<SharedPixelBuffer<Rgb8Pixel>, String> {
    use qrcode::{Color, EcLevel, QrCode};
    const QUIET: usize = 4;

    let code = QrCode::with_error_correction_level(data, EcLevel::L)
        .map_err(|e| format!("QR ERROR: {}", e))?;
    let width = code.width();
    let size = width + 2 * QUIET;

    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(size as u32, size as u32);
    let pixels = buf.make_mut_slice();
    pixels.fill(Rgb8Pixel { r: 255, g: 255, b: 255 });
    for (i, color) in code.to_colors().into_iter().enumerate() {
        if color == Color::Dark {
            let (x, y) = (i % width + QUIET, i / width + QUIET);
            pixels[y * size + x] = Rgb8Pixel { r: 0, g: 0, b: 0 };
        }
    }
    Ok(buf)
}

/// Outcome of checking every token this wallet has sent.
struct ReclaimSummary {
    sats: u64,
    reclaimed: usize,
    already_claimed: usize,
    failed: usize,
}

/// Check every token this wallet has sent and take back the ones nobody has claimed.
async fn reclaim_unclaimed(wallet: &Wallet) -> Result<ReclaimSummary, cdk::Error> {
    let mut sats = Amount::ZERO;
    let (mut reclaimed, mut already_claimed, mut failed) = (0, 0, 0);

    for id in wallet.get_pending_sends().await? {
        match wallet.check_send_status(id).await {
            Ok(true) => already_claimed += 1,
            Ok(false) => match wallet.revoke_send(id).await {
                Ok(amount) => {
                    sats += amount;
                    reclaimed += 1;
                }
                Err(_) => failed += 1,
            },
            Err(_) => failed += 1,
        }
    }

    // Settle proofs left pending outside any send (e.g. an interrupted operation)
    wallet.check_all_pending_proofs().await?;

    Ok(ReclaimSummary { sats: sats.into(), reclaimed, already_claimed, failed })
}

async fn create_wallet() -> Result<Wallet, Box<dyn std::error::Error + Send + Sync>> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cashu-pixel");
    std::fs::create_dir_all(&data_dir)?;

    let db_path = data_dir.join("wallet.db");
    let localstore = Arc::new(
        cdk_sqlite::wallet::WalletSqliteDatabase::new(&db_path).await?,
    );

    let seed_path = data_dir.join("seed");
    let seed: [u8; 64] = if seed_path.exists() {
        let bytes = std::fs::read(&seed_path)?;
        let mut seed = [0u8; 64];
        seed.copy_from_slice(&bytes[..64]);
        seed
    } else {
        let mut seed = [0u8; 64];
        rand::Rng::fill(&mut rand::thread_rng(), &mut seed);
        std::fs::write(&seed_path, &seed)?;
        seed
    };

    let wallet = Wallet::new(MINT_URL, CurrencyUnit::Sat, localstore, seed, None)?;
    wallet.recover_incomplete_sagas().await?;

    Ok(wallet)
}

fn update_ui(ui_weak: &slint::Weak<MainApp>, f: impl FnOnce(&MainApp) + Send + 'static) {
    let ui_w = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_w.upgrade() {
            f(&ui);
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = MainApp::new()?;
    let ui_weak = ui.as_weak();

    let app_wallet: Arc<Mutex<Option<AppWallet>>> = Arc::new(Mutex::new(None));

    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap(),
    );

    // Init wallet
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    rt.spawn(async move {
        match create_wallet().await {
            Ok(wallet) => {
                let balance: u64 = wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                let mut w = wallet_ref.lock().await;
                *w = Some(AppWallet { wallet, current_quote: None });
                drop(w);
                update_ui(&ui_w, move |ui| {
                    let state = ui.global::<WalletState>();
                    state.set_balance(SharedString::from(format!("{}", balance)));
                    state.set_mint_url(SharedString::from(
                        MINT_URL.replace("https://", "").split('/').next().unwrap_or(MINT_URL),
                    ));
                });

                // Mint any invoices that were paid while the app was closed
                let w = wallet_ref.lock().await;
                if let Some(ref app) = *w {
                    if let Ok(minted) = app.wallet.mint_unissued_quotes().await {
                        if minted > Amount::ZERO {
                            let minted: u64 = minted.into();
                            let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                            update_ui(&ui_w, move |ui| {
                                let state = ui.global::<WalletState>();
                                state.set_balance(SharedString::from(format!("{}", bal)));
                                state.set_status(SharedString::from(format!("RECEIVED {} SAT FROM A PAID INVOICE", minted)));
                            });
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("INIT ERROR: {}", e);
                update_ui(&ui_w, move |ui| {
                    ui.global::<WalletState>().set_status(SharedString::from(msg));
                });
            }
        }
    });

    // Mint quote: show the invoice, then watch for payment and mint automatically
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_request_mint_quote(move |amount| {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_h.spawn(async move {
            let mut w = wallet_ref.lock().await;
            let Some(ref mut app) = *w else { return };
            let quote = match app.wallet.mint_quote(PaymentMethod::BOLT11, Some(Amount::from(amount as u64)), None, None).await {
                Ok(quote) => quote,
                Err(e) => {
                    let msg = format!("ERROR: {}", e);
                    update_ui(&ui_w, move |ui| {
                        ui.global::<WalletState>().set_status(SharedString::from(msg));
                    });
                    return;
                }
            };
            // BOLT11 is case-insensitive; uppercase lets the QR use its denser alphanumeric mode
            let invoice = quote.request.to_uppercase();
            let (quote_id, expiry) = (quote.id.clone(), quote.expiry);
            app.current_quote = Some(quote);
            drop(w);

            let pixels = match qr_pixels(&invoice) {
                Ok(pixels) => pixels,
                Err(msg) => {
                    update_ui(&ui_w, move |ui| {
                        ui.global::<WalletState>().set_status(SharedString::from(msg));
                    });
                    return;
                }
            };
            let status = format!("SCAN WITH A LIGHTNING WALLET // PAY {} SAT", amount);
            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_qr_image(slint::Image::from_rgb8(pixels));
                state.set_qr_animated(false);
                state.set_qr_is_invoice(true);
                state.set_mint_complete(false);
                state.set_qr_ready(true);
                state.set_status(SharedString::from(status));
                state.set_current_screen(4);
            });

            // Poll until the invoice is paid (then mint), replaced by a newer one, or expired.
            // The wallet lock is held only for each check, so the rest of the app stays usable.
            loop {
                tokio::time::sleep(Duration::from_millis(PAYMENT_POLL_MS)).await;
                let mut w = wallet_ref.lock().await;
                let Some(ref mut app) = *w else { return };
                if app.current_quote.as_ref().map_or(true, |q| q.id != quote_id) {
                    return;
                }
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                if expiry > 0 && now > expiry {
                    app.current_quote = None;
                    update_ui(&ui_w, |ui| {
                        ui.global::<WalletState>().set_status(SharedString::from("INVOICE EXPIRED // CREATE A NEW ONE"));
                    });
                    return;
                }
                match app.wallet.check_mint_quote_status(&quote_id).await {
                    Ok(q) if q.state == MintQuoteState::Paid => {}
                    // Unpaid, or a transient network error: keep waiting
                    _ => continue,
                }
                match app.wallet.mint(&quote_id, Default::default(), None).await {
                    Ok(_) => {
                        app.current_quote = None;
                        let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                        update_ui(&ui_w, move |ui| {
                            let state = ui.global::<WalletState>();
                            state.set_balance(SharedString::from(format!("{}", bal)));
                            state.set_mint_complete(true);
                            state.set_status(SharedString::from(format!("NEW BALANCE {} SAT", bal)));
                        });
                        return;
                    }
                    Err(e) => {
                        // Paid but minting failed: show it and retry on the next poll
                        let msg = format!("PAID // MINT RETRYING: {}", e);
                        update_ui(&ui_w, move |ui| {
                            ui.global::<WalletState>().set_status(SharedString::from(msg));
                        });
                    }
                }
            }
        });
    });

    // Animated token QR: the encoder for the token on screen, advanced by a UI timer
    let qr_anim: Arc<std::sync::Mutex<Option<TokenUrEncoder>>> = Arc::new(std::sync::Mutex::new(None));
    let qr_timer = slint::Timer::default();
    {
        let ui_w = ui_weak.clone();
        let qr_anim = qr_anim.clone();
        qr_timer.start(slint::TimerMode::Repeated, Duration::from_millis(QR_FRAME_MS), move || {
            let Some(ui) = ui_w.upgrade() else { return };
            let state = ui.global::<WalletState>();
            if !state.get_qr_animated() || state.get_current_screen() != 4 {
                return;
            }
            let part = match qr_anim.lock().unwrap().as_mut().map(|encoder| encoder.next_part()) {
                Some(Ok(part)) => part,
                _ => return,
            };
            if let Ok(pixels) = qr_pixels(&part) {
                state.set_qr_image(slint::Image::from_rgb8(pixels));
            }
        });
    }

    // Send
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    let anim_ref = qr_anim.clone();
    ui.global::<WalletState>().on_request_send(move |amount| {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        let anim_ref = anim_ref.clone();
        rt_h.spawn(async move {
            let mut w = wallet_ref.lock().await;
            if let Some(ref mut app) = *w {
                match app.wallet.prepare_send(Amount::from(amount as u64), SendOptions::default()).await {
                    Ok(prepared) => match prepared.confirm(None).await {
                        Ok(token) => {
                            let token_str = token.to_string();
                            let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
                            drop(w);
                            // Small tokens: one static QR. Larger ones: animated NUT-16 frames.
                            let mut first_frame = token_str.clone();
                            let mut animated = false;
                            if token_str.len() > STATIC_QR_MAX_LEN {
                                if let Ok(mut encoder) = token.ur_encoder(UR_FRAGMENT_LEN) {
                                    if let Ok(part) = encoder.next_part() {
                                        first_frame = part;
                                        animated = true;
                                        *anim_ref.lock().unwrap() = Some(encoder);
                                    }
                                }
                            }
                            match qr_pixels(&first_frame) {
                                Ok(pixels) => {
                                    let status = if animated {
                                        format!("SCAN TO RECEIVE {} SAT // ANIMATED QR", amount)
                                    } else {
                                        format!("SCAN TO RECEIVE {} SAT", amount)
                                    };
                                    update_ui(&ui_w, move |ui| {
                                        let state = ui.global::<WalletState>();
                                        state.set_balance(SharedString::from(format!("{}", bal)));
                                        state.set_qr_image(slint::Image::from_rgb8(pixels));
                                        state.set_qr_animated(animated);
                                        state.set_qr_ready(true);
                                        state.set_status(SharedString::from(status));
                                        state.set_qr_is_invoice(false);
                                        state.set_mint_complete(false);
                                        state.set_current_screen(4);
                                    });
                                }
                                Err(_) => {
                                    let short = format!("{}...", &token_str[..60.min(token_str.len())]);
                                    update_ui(&ui_w, move |ui| {
                                        let state = ui.global::<WalletState>();
                                        state.set_balance(SharedString::from(format!("{}", bal)));
                                        state.set_status(SharedString::from(format!("TOKEN TOO LARGE FOR QR: {}", short)));
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("ERROR: {}", e);
                            update_ui(&ui_w, move |ui| { ui.global::<WalletState>().set_status(SharedString::from(msg)); });
                        }
                    },
                    Err(e) => {
                        let msg = format!("ERROR: {}", e);
                        update_ui(&ui_w, move |ui| { ui.global::<WalletState>().set_status(SharedString::from(msg)); });
                    }
                }
            }
        });
    });

    // Start receive (BLE scan — placeholder for now). Sets no status: the UI must not
    // claim a scan is running until BLE receive actually exists.
    ui.global::<WalletState>().on_start_receive(|| {});

    // Reclaim (Settings): check every token this wallet sent; take back unclaimed ones
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_reclaim_tokens(move || {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_h.spawn(async move {
            let w = wallet_ref.lock().await;
            let Some(ref app) = *w else { return };
            let status = match reclaim_unclaimed(&app.wallet).await {
                Ok(s) if s.reclaimed == 0 && s.failed == 0 && s.already_claimed > 0 => format!(
                    "NOTHING TO RECLAIM // {} SENT TOKEN{} ALREADY CLAIMED",
                    s.already_claimed,
                    if s.already_claimed == 1 { "" } else { "S" }
                ),
                Ok(s) if s.reclaimed == 0 && s.failed == 0 => "NO UNCLAIMED TOKENS // NOTHING TO RECLAIM".to_string(),
                Ok(s) => {
                    let mut msg = format!(
                        "RECLAIMED {} SAT FROM {} TOKEN{}",
                        s.sats,
                        s.reclaimed,
                        if s.reclaimed == 1 { "" } else { "S" }
                    );
                    if s.already_claimed > 0 {
                        msg += &format!(" // {} ALREADY CLAIMED", s.already_claimed);
                    }
                    if s.failed > 0 {
                        msg += &format!(" // {} FAILED, TRY AGAIN", s.failed);
                    }
                    msg
                }
                Err(e) => format!("RECLAIM ERROR: {}", e),
            };
            let bal: u64 = app.wallet.total_balance().await.unwrap_or(Amount::from(0)).into();
            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_balance(SharedString::from(format!("{}", bal)));
                state.set_status(SharedString::from(status));
            });
        });
    });

    // Exit (Settings): close the window and end the process, back to the desktop
    ui.global::<WalletState>().on_exit_app(|| {
        let _ = slint::quit_event_loop();
    });

    ui.run()?;
    Ok(())
}

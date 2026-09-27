use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdk::mint_url::MintUrl;
use cdk::nuts::CurrencyUnit;
use cdk::nuts::nut00::PaymentMethod;
use cdk::nuts::{
    MintQuoteState, PaymentRequest, PaymentRequestPayload, Token, TokenUrEncoder, Transport,
    TransportType,
};
use cdk::wallet::{ReceiveOptions, SendOptions, Wallet};
use cdk::Amount;
use nostr_sdk::nips::nip19::Nip19Profile;
use nostr_sdk::{Client, Filter, Keys, Kind, RelayPoolNotification, RelayUrl, ToBech32};
use slint::{Rgb8Pixel, SharedPixelBuffer, SharedString};
use tokio::sync::Mutex;

slint::include_modules!();

mod ble;
mod snapshot;

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
/// Relays a payer's wallet uses to deliver ecash for a Cashu payment request (NUT-18).
const NOSTR_RELAYS: [&str; 2] = ["wss://relay.damus.io", "wss://nos.lol"];
/// How long a receive request is watched when the mint gives no invoice expiry.
const RECEIVE_TTL_SECS: u64 = 600;

/// Each receive screen starts a new session; watchers of an older session stop.
static RECEIVE_SESSION: AtomicU64 = AtomicU64::new(0);
/// The same for the Bluetooth receive screen: events from an older session are dropped.
static BLE_SESSION: AtomicU64 = AtomicU64::new(0);

/// Bluetooth receive: the open session's off switch and the token waiting to be redeemed
#[derive(Default)]
struct BleReceive {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    token: Option<Token>,
}

/// A mint URL as the screen shows it: host only
fn mint_host(url: &str) -> String {
    url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or(url).to_string()
}

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

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Build a NUT-18 Cashu payment request for `amount` sat from our mint, delivered over
/// Nostr to a fresh key. Returns the encoded request (`creqA…`) and the key to listen with.
fn cashu_request(amount: u64) -> Result<(String, Keys), String> {
    let keys = Keys::generate();
    let relays = NOSTR_RELAYS.iter().filter_map(|r| RelayUrl::parse(r).ok());
    let nprofile = Nip19Profile::new(keys.public_key(), relays)
        .to_bech32()
        .map_err(|e| e.to_string())?;
    let transport = Transport::builder()
        .transport_type(TransportType::Nostr)
        .target(nprofile)
        .add_tag(vec!["n".to_string(), "17".to_string()])
        .build()
        .map_err(|e| e.to_string())?;
    let request = PaymentRequest::builder()
        .payment_id(format!("{:08x}", rand::random::<u32>()))
        .amount(amount)
        .unit(CurrencyUnit::Sat)
        .single_use(true)
        .add_mint(MintUrl::from_str(MINT_URL).map_err(|e| e.to_string())?)
        .description("Cashu NERV")
        .add_transport(transport)
        .build();
    Ok((request.to_string(), keys))
}

/// Show the paid state on the receive screen, however the payment arrived.
fn show_received(ui_w: &slint::Weak<MainApp>, received: u64, balance: u64) {
    update_ui(ui_w, move |ui| {
        let state = ui.global::<WalletState>();
        state.set_balance(SharedString::from(format!("{}", balance)));
        state.set_received_amount(received as i32);
        state.set_mint_complete(true);
        state.set_status(SharedString::from(format!("NEW BALANCE {} SAT", balance)));
    });
}

/// Listen on Nostr for ecash sent to a Cashu payment request and receive it into the wallet.
/// Stops once the session is paid (either way), replaced by a newer one, or past `deadline`.
async fn watch_cashu_request(
    keys: Keys,
    session: u64,
    paid: Arc<AtomicBool>,
    deadline: u64,
    wallet_ref: Arc<Mutex<Option<AppWallet>>>,
    ui_w: slint::Weak<MainApp>,
) {
    let pubkey = keys.public_key();
    let client = Client::new(keys);
    for relay in NOSTR_RELAYS {
        let _ = client.add_read_relay(relay).await;
    }
    client.connect().await;
    let mut notifications = client.notifications();
    if client.subscribe(Filter::new().pubkey(pubkey).kind(Kind::GiftWrap), None).await.is_err() {
        client.shutdown().await;
        return;
    }
    let our_mint = MintUrl::from_str(MINT_URL).ok();

    while !paid.load(Ordering::SeqCst)
        && RECEIVE_SESSION.load(Ordering::SeqCst) == session
        && now_secs() < deadline
    {
        let event = match tokio::time::timeout(Duration::from_secs(2), notifications.recv()).await {
            Ok(Ok(RelayPoolNotification::Event { event, .. })) => event,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            // Timeout (re-check the stop conditions), lag, or a non-event notification
            _ => continue,
        };
        let Ok(gift) = client.unwrap_gift_wrap(&event).await else { continue };
        let Ok(payload) = serde_json::from_str::<PaymentRequestPayload>(&gift.rumor.content) else { continue };
        // Single-mint wallet: only ecash from our mint, in sat, can be received
        if Some(&payload.mint) != our_mint.as_ref() || payload.unit != CurrencyUnit::Sat {
            continue;
        }
        let token = Token::new(payload.mint, payload.proofs, payload.memo, payload.unit);

        let w = wallet_ref.lock().await;
        let Some(ref app) = *w else { break };
        match app.wallet.receive(&token.to_string(), ReceiveOptions::default()).await {
            Ok(received) => {
                let balance: u64 = app.wallet.total_balance().await.unwrap_or(Amount::ZERO).into();
                drop(w);
                if !paid.swap(true, Ordering::SeqCst) {
                    show_received(&ui_w, received.into(), balance);
                } else {
                    // Lightning completed first; still credit this payment
                    update_ui(&ui_w, move |ui| {
                        ui.global::<WalletState>().set_balance(SharedString::from(format!("{}", balance)));
                    });
                }
                break;
            }
            Err(e) => {
                let msg = format!("CASHU PAYMENT FAILED: {}", e);
                update_ui(&ui_w, move |ui| {
                    ui.global::<WalletState>().set_status(SharedString::from(msg));
                });
            }
        }
    }
    client.shutdown().await;
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
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let dir = args.get(i + 1).map_or_else(|| PathBuf::from("snapshots"), PathBuf::from);
        return snapshot::run(&dir);
    }

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
                    state.set_mint_url(SharedString::from(mint_host(MINT_URL)));
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

    // Receive: create a Lightning invoice and a Cashu payment request for the same amount,
    // show both (toggled on screen), and complete on whichever is paid first
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_request_payment(move |amount| {
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

            let invoice_pixels = match qr_pixels(&invoice) {
                Ok(pixels) => pixels,
                Err(msg) => {
                    update_ui(&ui_w, move |ui| {
                        ui.global::<WalletState>().set_status(SharedString::from(msg));
                    });
                    return;
                }
            };
            // The Cashu request is optional: without it the screen still works over Lightning
            let request = cashu_request(amount as u64)
                .ok()
                .and_then(|(encoded, keys)| qr_pixels(&encoded).ok().map(|pixels| (pixels, keys)));
            let request_available = request.is_some();
            let (request_pixels, request_keys) = match request {
                Some((pixels, keys)) => (Some(pixels), Some(keys)),
                None => (None, None),
            };

            let session = RECEIVE_SESSION.fetch_add(1, Ordering::SeqCst) + 1;
            let paid = Arc::new(AtomicBool::new(false));
            let deadline = if expiry > 0 { expiry } else { now_secs() + RECEIVE_TTL_SECS };

            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_qr_image(slint::Image::from_rgb8(invoice_pixels));
                if let Some(pixels) = request_pixels {
                    state.set_request_image(slint::Image::from_rgb8(pixels));
                }
                state.set_request_available(request_available);
                state.set_receive_cashu(false);
                state.set_qr_animated(false);
                state.set_qr_is_invoice(true);
                state.set_mint_complete(false);
                state.set_qr_ready(true);
                state.set_status(SharedString::from(""));
                state.set_current_screen(4);
            });

            if let Some(keys) = request_keys {
                tokio::spawn(watch_cashu_request(
                    keys,
                    session,
                    paid.clone(),
                    deadline,
                    wallet_ref.clone(),
                    ui_w.clone(),
                ));
            }

            // Lightning: poll until paid (then mint), superseded, or expired.
            // The wallet lock is held only for each check, so the rest of the app stays usable.
            loop {
                tokio::time::sleep(Duration::from_millis(PAYMENT_POLL_MS)).await;
                if paid.load(Ordering::SeqCst) || RECEIVE_SESSION.load(Ordering::SeqCst) != session {
                    return;
                }
                let mut w = wallet_ref.lock().await;
                let Some(ref mut app) = *w else { return };
                if app.current_quote.as_ref().map_or(true, |q| q.id != quote_id) {
                    return;
                }
                if now_secs() > deadline {
                    app.current_quote = None;
                    update_ui(&ui_w, |ui| {
                        ui.global::<WalletState>().set_status(SharedString::from("REQUEST EXPIRED // CREATE A NEW ONE"));
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
                        let balance: u64 = app.wallet.total_balance().await.unwrap_or(Amount::ZERO).into();
                        if !paid.swap(true, Ordering::SeqCst) {
                            show_received(&ui_w, amount as u64, balance);
                        }
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

    // Receive over Bluetooth: advertise while the screen is open, show a token when one
    // arrives, and redeem it when the user says so
    let ble_receive = Arc::new(std::sync::Mutex::new(BleReceive::default()));
    let ble_link = ble::Link::default();

    let (receive, link, ui_w, rt_h) = (ble_receive.clone(), ble_link.clone(), ui_weak.clone(), rt.handle().clone());
    ui.global::<WalletState>().on_start_bluetooth(move || {
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        {
            let mut r = receive.lock().unwrap();
            if let Some(old) = r.stop.replace(stop_tx) {
                let _ = old.send(());
            }
            r.token = None;
        }
        let session = BLE_SESSION.fetch_add(1, Ordering::SeqCst) + 1;
        rt_h.spawn(ble::serve(events_tx, link.clone(), stop_rx));

        let (receive, link, ui_w) = (receive.clone(), link.clone(), ui_w.clone());
        rt_h.spawn(async move {
            let our_mint = MintUrl::from_str(MINT_URL).ok();
            while let Some(event) = events.recv().await {
                if BLE_SESSION.load(Ordering::SeqCst) != session {
                    break;
                }
                // While a token is on screen, further writes wait until it's redeemed or discarded
                let holding = receive.lock().unwrap().token.is_some();
                match event {
                    ble::Event::Ready => update_ui(&ui_w, |ui| {
                        ui.global::<WalletState>().set_ble_phase(BlePhase::Listening);
                    }),
                    ble::Event::Linked(linked) => update_ui(&ui_w, move |ui| {
                        let state = ui.global::<WalletState>();
                        state.set_ble_linked(linked);
                        if state.get_ble_phase() == BlePhase::Receiving {
                            state.set_ble_phase(BlePhase::Listening);
                            state.set_status(SharedString::from("LINK LOST // SEND THE TOKEN AGAIN"));
                        }
                    }),
                    ble::Event::Receiving(bytes) if !holding => update_ui(&ui_w, move |ui| {
                        let state = ui.global::<WalletState>();
                        state.set_ble_phase(BlePhase::Receiving);
                        state.set_ble_bytes(bytes as i32);
                        state.set_status(SharedString::from(""));
                    }),
                    ble::Event::Token(token) if !holding => {
                        let amount: u64 = token.value().map(u64::from).unwrap_or(0);
                        let mint = token.mint_url().map(|m| m.to_string()).unwrap_or_default();
                        let host = mint_host(&mint);
                        let memo = token.memo().clone().unwrap_or_default();
                        let redeemable = token.mint_url().ok() == our_mint && token.unit() == Some(CurrencyUnit::Sat);
                        receive.lock().unwrap().token = Some(token);
                        let reply = if redeemable {
                            format!("Got {} sat. Tap REDEEM on the screen.", amount)
                        } else {
                            format!("That token is from {}. Cashu NERV only takes ecash from {}.", host, mint_host(MINT_URL))
                        };
                        let shown = host.to_uppercase();
                        update_ui(&ui_w, move |ui| {
                            let state = ui.global::<WalletState>();
                            state.set_ble_amount(amount as i32);
                            state.set_ble_mint(SharedString::from(shown));
                            state.set_ble_memo(SharedString::from(memo));
                            state.set_ble_redeemable(redeemable);
                            state.set_ble_phase(BlePhase::Token);
                            state.set_status(SharedString::from(""));
                        });
                        link.reply(&reply).await;
                    }
                    ble::Event::NotToken if !holding => {
                        update_ui(&ui_w, |ui| {
                            let state = ui.global::<WalletState>();
                            state.set_ble_phase(BlePhase::Listening);
                            state.set_ble_bytes(0);
                            state.set_status(SharedString::from("THAT WAS NOT A CASHU TOKEN"));
                        });
                        link.reply("That's not a Cashu token. Send one that starts with cashuA or cashuB.").await;
                    }
                    ble::Event::Failed(e) => {
                        let note = format!("BLUETOOTH ERROR: {}", e);
                        update_ui(&ui_w, move |ui| {
                            let state = ui.global::<WalletState>();
                            state.set_ble_phase(BlePhase::Offline);
                            state.set_status(SharedString::from(note));
                        });
                    }
                    ble::Event::Receiving(_) | ble::Event::Token(_) | ble::Event::NotToken => {
                        link.reply("One token at a time: redeem or discard the one on screen first.").await;
                    }
                }
            }
        });
    });

    // Leaving the screen stops advertising and drops any token not redeemed
    let receive = ble_receive.clone();
    ui.global::<WalletState>().on_stop_bluetooth(move || {
        BLE_SESSION.fetch_add(1, Ordering::SeqCst);
        let mut r = receive.lock().unwrap();
        if let Some(stop) = r.stop.take() {
            let _ = stop.send(());
        }
        r.token = None;
    });

    let (receive, link, ui_w, rt_h) = (ble_receive.clone(), ble_link.clone(), ui_weak.clone(), rt.handle().clone());
    ui.global::<WalletState>().on_discard_bluetooth(move || {
        receive.lock().unwrap().token = None;
        update_ui(&ui_w, |ui| {
            let state = ui.global::<WalletState>();
            state.set_ble_phase(BlePhase::Listening);
            state.set_ble_bytes(0);
            state.set_status(SharedString::from(""));
        });
        let link = link.clone();
        rt_h.spawn(async move { link.reply("Listening again.").await });
    });

    let (receive, link, wallet_ref, ui_w, rt_h) =
        (ble_receive.clone(), ble_link.clone(), app_wallet.clone(), ui_weak.clone(), rt.handle().clone());
    ui.global::<WalletState>().on_redeem_bluetooth(move || {
        let Some(token) = receive.lock().unwrap().token.clone() else { return };
        update_ui(&ui_w, |ui| ui.global::<WalletState>().set_ble_phase(BlePhase::Redeeming));
        let (receive, link, wallet_ref, ui_w) = (receive.clone(), link.clone(), wallet_ref.clone(), ui_w.clone());
        rt_h.spawn(async move {
            let w = wallet_ref.lock().await;
            let Some(ref app) = *w else { return };
            let result = app.wallet.receive(&token.to_string(), ReceiveOptions::default()).await;
            let balance: u64 = app.wallet.total_balance().await.unwrap_or(Amount::ZERO).into();
            drop(w);
            receive.lock().unwrap().token = None;
            match result {
                Ok(received) => {
                    let received: u64 = received.into();
                    update_ui(&ui_w, move |ui| {
                        let state = ui.global::<WalletState>();
                        state.set_balance(SharedString::from(format!("{}", balance)));
                        state.set_received_amount(received as i32);
                        state.set_ble_phase(BlePhase::Received);
                        state.set_status(SharedString::from(format!("NEW BALANCE {} SAT", balance)));
                    });
                    link.reply(&format!("Redeemed {} sat. Thank you!", received)).await;
                }
                Err(e) => {
                    let reason = e.to_string();
                    let note = reason.to_uppercase();
                    update_ui(&ui_w, move |ui| {
                        let state = ui.global::<WalletState>();
                        state.set_ble_note(SharedString::from(note));
                        state.set_ble_phase(BlePhase::Failed);
                    });
                    link.reply(&format!("Couldn't redeem that token: {}", reason)).await;
                }
            }
        });
    });

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

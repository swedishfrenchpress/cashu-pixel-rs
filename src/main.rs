use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdk::cdk_database::WalletDatabase;
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
#[path = "../shared/mints.rs"]
mod mints;
mod snapshot;

/// The home mint until one is picked under Settings
const MINT_URL: &str = "https://mint.minibits.cash/Bitcoin";
/// Holds the picked home mint's URL, next to the wallet. A reset keeps it.
const HOME_MINT_FILE: &str = "home-mint";

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

/// The wallet: one CDK wallet per mint, all on the same seed and database
struct AppWallet {
    /// The home mint. Lightning and payment requests receive here, and sends draw from it
    /// first.
    wallet: Wallet,
    /// Other mints this wallet holds ecash from, added as tokens from them are redeemed
    others: Vec<Wallet>,
    localstore: Arc<cdk_sqlite::wallet::WalletSqliteDatabase>,
    seed: [u8; 64],
    current_quote: Option<cdk::wallet::MintQuote>,
}

impl AppWallet {
    /// Wallets for `home` and for every other mint the database holds ecash from, reachable
    /// or not, so all of it counts in the balance
    async fn open(
        localstore: Arc<cdk_sqlite::wallet::WalletSqliteDatabase>,
        seed: [u8; 64],
        home: &MintUrl,
    ) -> Result<AppWallet, Box<dyn std::error::Error + Send + Sync>> {
        let wallet = Wallet::new(&home.to_string(), CurrencyUnit::Sat, localstore.clone(), seed, None)?;
        let mut others: Vec<Wallet> = Vec::new();
        for proof in localstore.get_proofs(None, None, None, None).await? {
            if proof.mint_url != wallet.mint_url && !others.iter().any(|w| w.mint_url == proof.mint_url) {
                others.push(Wallet::new(&proof.mint_url.to_string(), CurrencyUnit::Sat, localstore.clone(), seed, None)?);
            }
        }
        Ok(AppWallet { wallet, others, localstore, seed, current_quote: None })
    }

    /// Every mint's wallet, home first
    fn wallets(&self) -> impl Iterator<Item = &Wallet> {
        std::iter::once(&self.wallet).chain(self.others.iter())
    }

    /// Sats across all mints
    async fn balance(&self) -> u64 {
        let mut total = 0;
        for wallet in self.wallets() {
            total += u64::from(wallet.total_balance().await.unwrap_or(Amount::ZERO));
        }
        total
    }

    fn holds(&self, mint: &MintUrl) -> bool {
        self.wallets().any(|w| &w.mint_url == mint)
    }

    /// The wallet for `mint`: the one this wallet has, or a new one that is only kept once
    /// something is received into it (see `keep`)
    fn wallet_for(&self, mint: &MintUrl) -> Result<Wallet, cdk::Error> {
        match self.wallets().find(|w| &w.mint_url == mint) {
            Some(wallet) => Ok(wallet.clone()),
            None => Wallet::new(&mint.to_string(), CurrencyUnit::Sat, self.localstore.clone(), self.seed, None),
        }
    }

    fn keep(&mut self, wallet: Wallet) {
        if !self.holds(&wallet.mint_url) {
            self.others.push(wallet);
        }
    }

    /// Make `wallet` the home mint. The old home stays on as another mint while the database
    /// holds ecash from it.
    async fn set_home(&mut self, wallet: Wallet) {
        let mint = wallet.mint_url.clone();
        let old = std::mem::replace(&mut self.wallet, wallet);
        self.others.retain(|w| w.mint_url != mint);
        let held = self.localstore.get_proofs(Some(old.mint_url.clone()), None, None, None).await;
        if held.is_ok_and(|proofs| !proofs.is_empty()) {
            self.others.push(old);
        }
    }

    /// The mints to choose a home from: the offered ones, then any other this wallet holds
    /// ecash from, each with the sats held there
    async fn mint_choices(&self, data_dir: &Path) -> Vec<MintChoice> {
        let mut mints = mints::offered(data_dir);
        for wallet in self.wallets() {
            if !mints.contains(&wallet.mint_url) {
                mints.push(wallet.mint_url.clone());
            }
        }
        let mut choices = Vec::new();
        for mint in mints {
            let mut sats = 0;
            if let Some(wallet) = self.wallets().find(|w| w.mint_url == mint) {
                sats = u64::from(wallet.total_balance().await.unwrap_or(Amount::ZERO));
            }
            choices.push(MintChoice {
                host: SharedString::from(mints::host(&mint)),
                url: SharedString::from(mint.to_string()),
                sats: sats as i32,
                home: mint == self.wallet.mint_url,
            });
        }
        choices
    }
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

/// Build a NUT-18 Cashu payment request for `amount` sat from `mint`, delivered over Nostr
/// to a fresh key. Returns the encoded request (`creqA…`) and the key to listen with.
fn cashu_request(amount: u64, mint: &MintUrl) -> Result<(String, Keys), String> {
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
        .add_mint(mint.clone())
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

/// Listen on Nostr for ecash sent to a Cashu payment request and receive it into `wallet`,
/// the mint the request asked for. Stops once the session is paid (either way), replaced by
/// a newer one, or past `deadline`.
async fn watch_cashu_request(
    keys: Keys,
    session: u64,
    paid: Arc<AtomicBool>,
    deadline: u64,
    wallet: Wallet,
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
        // Only ecash from the mint the request asked for, in sat
        if payload.mint != wallet.mint_url || payload.unit != CurrencyUnit::Sat {
            continue;
        }
        let token = Token::new(payload.mint, payload.proofs, payload.memo, payload.unit);

        let mut w = wallet_ref.lock().await;
        let Some(ref mut app) = *w else { break };
        match wallet.receive(&token.to_string(), ReceiveOptions::default()).await {
            Ok(received) => {
                // The home mint may have changed since the request was made
                app.keep(wallet.clone());
                let balance = app.balance().await;
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
#[derive(Default)]
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

/// Where the wallet's database and seed live
fn wallet_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("cashu-pixel")
}

/// Erase the wallet in `data_dir`: its database, then its seed. The database must be closed.
/// SQLite's side files go first, so a leftover one is never replayed into the next database,
/// and the seed goes last, so ecash never outlives the seed it was made from.
fn erase_wallet(data_dir: &Path) -> std::io::Result<()> {
    for name in ["wallet.db-wal", "wallet.db-shm", "wallet.db", "seed"] {
        match std::fs::remove_file(data_dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The home mint picked under Settings, or the default
fn home_mint(data_dir: &Path) -> MintUrl {
    std::fs::read_to_string(data_dir.join(HOME_MINT_FILE))
        .ok()
        .and_then(|url| MintUrl::from_str(url.trim()).ok())
        .unwrap_or_else(|| MintUrl::from_str(MINT_URL).expect("the default mint URL is valid"))
}

async fn open_wallets(data_dir: &Path) -> Result<AppWallet, Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(data_dir)?;

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

    let app = AppWallet::open(localstore, seed, &home_mint(data_dir)).await?;
    app.wallet.recover_incomplete_sagas().await?;
    Ok(app)
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
        match open_wallets(&wallet_dir()).await {
            Ok(app) => {
                let balance = app.balance().await;
                let others = app.others.len();
                let home = mints::host(&app.wallet.mint_url);
                let mut w = wallet_ref.lock().await;
                *w = Some(app);
                drop(w);
                update_ui(&ui_w, move |ui| {
                    let state = ui.global::<WalletState>();
                    state.set_balance(SharedString::from(format!("{}", balance)));
                    state.set_mint_url(SharedString::from(home));
                    state.set_other_mints(others as i32);
                });

                // Finish anything the other mints' wallets left half done
                let w = wallet_ref.lock().await;
                if let Some(ref app) = *w {
                    for other in &app.others {
                        let _ = other.recover_incomplete_sagas().await;
                    }
                }
                drop(w);

                // Mint any invoices that were paid while the app was closed
                let w = wallet_ref.lock().await;
                if let Some(ref app) = *w {
                    if let Ok(minted) = app.wallet.mint_unissued_quotes().await {
                        if minted > Amount::ZERO {
                            let minted: u64 = minted.into();
                            let bal = app.balance().await;
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
            // Both requests stay with this mint even if the home mint changes while they're open
            let wallet = app.wallet.clone();
            let quote = match wallet.mint_quote(PaymentMethod::BOLT11, Some(Amount::from(amount as u64)), None, None).await {
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
            let request = cashu_request(amount as u64, &wallet.mint_url)
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
                    wallet.clone(),
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
                match wallet.check_mint_quote_status(&quote_id).await {
                    Ok(q) if q.state == MintQuoteState::Paid => {}
                    // Unpaid, or a transient network error: keep waiting
                    _ => continue,
                }
                match wallet.mint(&quote_id, Default::default(), None).await {
                    Ok(_) => {
                        app.current_quote = None;
                        app.keep(wallet.clone());
                        let balance = app.balance().await;
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
            let w = wallet_ref.lock().await;
            let Some(ref app) = *w else { return };
            let amount = amount as u64;

            // A token comes from a single mint: the home mint when it covers the amount,
            // otherwise the other mint holding the most
            let mut candidates = Vec::new();
            for wallet in app.wallets() {
                let held = u64::from(wallet.total_balance().await.unwrap_or(Amount::ZERO));
                if held >= amount {
                    candidates.push((wallet.clone(), held));
                }
            }
            candidates.sort_by_key(|(wallet, held)| (wallet.mint_url != app.wallet.mint_url, std::cmp::Reverse(*held)));
            let mut sent = None;
            let mut error = None;
            for (wallet, _) in candidates {
                match wallet.prepare_send(Amount::from(amount), SendOptions::default()).await {
                    Ok(prepared) => {
                        match prepared.confirm(None).await {
                            Ok(token) => sent = Some((token, wallet.mint_url.clone())),
                            Err(e) => error = Some(e.to_string()),
                        }
                        break;
                    }
                    // Short once fees are counted: try the next mint
                    Err(e) => error = Some(e.to_string()),
                }
            }
            let bal = app.balance().await;
            let home = app.wallet.mint_url.clone();
            drop(w);

            let Some((token, mint)) = sent else {
                let msg = match error {
                    Some(e) => format!("ERROR: {}", e),
                    None if bal >= amount => format!("NO SINGLE MINT HOLDS {} SAT // BALANCE IS SPLIT ACROSS MINTS", amount),
                    None => format!("NOT ENOUGH SATS // BALANCE {} SAT", bal),
                };
                update_ui(&ui_w, move |ui| {
                    let state = ui.global::<WalletState>();
                    state.set_balance(SharedString::from(format!("{}", bal)));
                    state.set_status(SharedString::from(msg));
                });
                return;
            };

            let token_str = token.to_string();
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
                    let status = if mint != home {
                        format!("SCAN TO RECEIVE {} SAT // {}", amount, mints::host(&mint).to_uppercase())
                    } else if animated {
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
        });
    });

    // Receive over Bluetooth: advertise while the screen is open, show a token when one
    // arrives, and redeem it when the user says so
    let ble_receive = Arc::new(std::sync::Mutex::new(BleReceive::default()));
    let ble_link = ble::Link::default();

    let (receive, link, wallet_ref, ui_w, rt_h) =
        (ble_receive.clone(), ble_link.clone(), app_wallet.clone(), ui_weak.clone(), rt.handle().clone());
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

        let (receive, link, wallet_ref, ui_w) = (receive.clone(), link.clone(), wallet_ref.clone(), ui_w.clone());
        rt_h.spawn(async move {
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
                        let mint = token.mint_url().ok();
                        let host = mint.as_ref().map(mints::host).unwrap_or_default();
                        let memo = token.memo().clone().unwrap_or_default();
                        // Any mint will do; the wallet holds sat only (tokens without a unit are sat)
                        let unit = token.unit().unwrap_or(CurrencyUnit::Sat);
                        let redeemable = mint.is_some() && unit == CurrencyUnit::Sat;
                        let new_mint = {
                            let w = wallet_ref.lock().await;
                            match (&*w, &mint) {
                                (Some(app), Some(mint)) => !app.holds(mint),
                                _ => false,
                            }
                        };
                        receive.lock().unwrap().token = Some(token);
                        let reply = if !redeemable {
                            format!("That token is in {}. Cashu NERV only holds sat.", unit)
                        } else if new_mint {
                            format!("Got {} sat from {}, a mint this wallet hasn't used yet. Tap REDEEM on the screen.", amount, host)
                        } else {
                            format!("Got {} sat from {}. Tap REDEEM on the screen.", amount, host)
                        };
                        let note = if redeemable { String::new() } else { format!("THIS WALLET ONLY HOLDS SAT, NOT {}", unit.to_string().to_uppercase()) };
                        let shown = host.to_uppercase();
                        update_ui(&ui_w, move |ui| {
                            let state = ui.global::<WalletState>();
                            state.set_ble_amount(amount as i32);
                            state.set_ble_mint(SharedString::from(shown));
                            state.set_ble_memo(SharedString::from(memo));
                            state.set_ble_redeemable(redeemable);
                            state.set_ble_new_mint(new_mint);
                            state.set_ble_note(SharedString::from(note));
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
            let mut w = wallet_ref.lock().await;
            let Some(ref mut app) = *w else { return };
            // Into the wallet for the token's mint, which is kept from then on if it's new
            let result = match token.mint_url().map_err(|e| e.to_string()).and_then(|mint| app.wallet_for(&mint).map_err(|e| e.to_string())) {
                Ok(wallet) => match wallet.receive(&token.to_string(), ReceiveOptions::default()).await {
                    Ok(received) => {
                        app.keep(wallet);
                        Ok(received)
                    }
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e),
            };
            let balance = app.balance().await;
            let others = app.others.len();
            drop(w);
            receive.lock().unwrap().token = None;
            match result {
                Ok(received) => {
                    let received: u64 = received.into();
                    update_ui(&ui_w, move |ui| {
                        let state = ui.global::<WalletState>();
                        state.set_balance(SharedString::from(format!("{}", balance)));
                        state.set_other_mints(others as i32);
                        state.set_received_amount(received as i32);
                        state.set_ble_phase(BlePhase::Received);
                        state.set_status(SharedString::from(format!("NEW BALANCE {} SAT", balance)));
                    });
                    link.reply(&format!("Redeemed {} sat. Thank you!", received)).await;
                }
                Err(reason) => {
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
            // Every mint's sends; a mint that can't be checked counts as one failure
            let mut summary: Result<ReclaimSummary, cdk::Error> = Ok(ReclaimSummary::default());
            for wallet in app.wallets() {
                match (reclaim_unclaimed(wallet).await, summary.as_mut()) {
                    (Ok(s), Ok(total)) => {
                        total.sats += s.sats;
                        total.reclaimed += s.reclaimed;
                        total.already_claimed += s.already_claimed;
                        total.failed += s.failed;
                    }
                    // The home mint failing is the error to show
                    (Err(e), Ok(_)) if wallet.mint_url == app.wallet.mint_url => summary = Err(e),
                    (Err(_), Ok(total)) => total.failed += 1,
                    (_, Err(_)) => {}
                }
            }
            let status = match summary {
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
            let bal = app.balance().await;
            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_balance(SharedString::from(format!("{}", bal)));
                state.set_status(SharedString::from(status));
            });
        });
    });

    // Home mint (Settings): list the mints to choose from, each with the sats held there
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_load_mints(move || {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_h.spawn(async move {
            let w = wallet_ref.lock().await;
            let Some(ref app) = *w else { return };
            let choices = app.mint_choices(&wallet_dir()).await;
            drop(w);
            update_ui(&ui_w, move |ui| {
                ui.global::<WalletState>().set_mints(slint::ModelRc::new(slint::VecModel::from(choices)));
            });
        });
    });

    // Make the picked mint the home mint, once it answers and can issue sat over Lightning.
    // Ecash already held stays at its own mint and still counts.
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_set_home_mint(move |url| {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_h.spawn(async move {
            let picked = MintUrl::from_str(&url).map_err(|e| e.to_string());
            let checked = match picked {
                Ok(mint) => {
                    // Checked without holding the wallet: the mint may take a while to answer
                    let candidate = {
                        let w = wallet_ref.lock().await;
                        w.as_ref().map(|app| app.wallet_for(&mint).map_err(|e| e.to_string()))
                    };
                    match candidate {
                        Some(Ok(wallet)) => mints::check(&wallet).await.map(|()| wallet),
                        Some(Err(e)) => Err(e),
                        None => Err("the wallet isn't open".to_string()),
                    }
                }
                Err(e) => Err(e),
            };

            let data_dir = wallet_dir();
            let mut w = wallet_ref.lock().await;
            let Some(ref mut app) = *w else {
                update_ui(&ui_w, |ui| {
                    let state = ui.global::<WalletState>();
                    state.set_mint_busy(false);
                    state.set_status(SharedString::from("THE WALLET ISN'T OPEN"));
                });
                return;
            };
            let status = match checked {
                Ok(wallet) => {
                    let host = mints::host(&wallet.mint_url).to_uppercase();
                    match std::fs::write(data_dir.join(HOME_MINT_FILE), wallet.mint_url.to_string()) {
                        Ok(()) => {
                            app.set_home(wallet).await;
                            format!("HOME MINT NOW {}", host)
                        }
                        Err(e) => format!("COULD NOT SAVE THE HOME MINT: {}", e),
                    }
                }
                Err(e) => format!("CAN'T USE {}: {}", url.trim_start_matches("https://").to_uppercase(), e.to_uppercase()),
            };
            let choices = app.mint_choices(&data_dir).await;
            let (home, others) = (mints::host(&app.wallet.mint_url), app.others.len());
            drop(w);
            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_mints(slint::ModelRc::new(slint::VecModel::from(choices)));
                state.set_mint_url(SharedString::from(home));
                state.set_other_mints(others as i32);
                state.set_mint_busy(false);
                state.set_status(SharedString::from(status));
            });
        });
    });

    // Reset (Settings): erase the wallet and start a new, empty one on a new seed
    let wallet_ref = app_wallet.clone();
    let ui_w = ui_weak.clone();
    let rt_h = rt.handle().clone();
    ui.global::<WalletState>().on_reset_device(move || {
        let wallet_ref = wallet_ref.clone();
        let ui_w = ui_w.clone();
        rt_h.spawn(async move {
            // Anything still watching an open receive request stops
            RECEIVE_SESSION.fetch_add(1, Ordering::SeqCst);
            let mut w = wallet_ref.lock().await;
            let data_dir = wallet_dir();
            // The database closes when the last wallet on it is dropped, and only then can
            // its files go: closed late, SQLite would delete the new database's log instead
            let store = w.take().map(|app| app.localstore);
            let open_elsewhere = store.as_ref().is_some_and(|store| Arc::strong_count(store) > 1);
            drop(store);
            let erased = if open_elsewhere {
                Err(std::io::Error::other("WALLET BUSY // TRY AGAIN"))
            } else {
                erase_wallet(&data_dir)
            };
            // Open a wallet either way, so the app keeps working if the erase failed
            let opened = open_wallets(&data_dir).await;
            let status = match (&erased, &opened) {
                (Ok(()), Ok(_)) => "DEVICE RESET // NEW EMPTY WALLET".to_string(),
                (Err(e), _) => format!("RESET ERROR: {}", e),
                (_, Err(e)) => format!("RESET ERROR: {}", e),
            };
            let (balance, others) = match &opened {
                Ok(app) => (app.balance().await, app.others.len()),
                Err(_) => (0, 0),
            };
            *w = opened.ok();
            drop(w);
            update_ui(&ui_w, move |ui| {
                let state = ui.global::<WalletState>();
                state.set_balance(SharedString::from(format!("{}", balance)));
                state.set_other_mints(others as i32);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_reset_leaves_a_new_empty_wallet_on_a_new_seed() {
        let dir = std::env::temp_dir().join(format!("cashu-pixel-test-{:08x}", rand::random::<u32>()));
        let (store, old_seed) = {
            let app = open_wallets(&dir).await.unwrap();
            (app.localstore.clone(), app.seed)
        };
        assert!(dir.join("wallet.db").exists() && dir.join("seed").exists());

        // What the reset relies on: with the wallets gone, nothing else holds the database
        assert_eq!(Arc::strong_count(&store), 1);
        drop(store);

        erase_wallet(&dir).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // Nothing left to erase is fine too
        erase_wallet(&dir).unwrap();

        let app = open_wallets(&dir).await.unwrap();
        assert_ne!(app.seed, old_seed);
        assert_eq!(app.balance().await, 0);
        assert!(app.others.is_empty());

        drop(app);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

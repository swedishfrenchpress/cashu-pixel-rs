//! Pixel Faucet: gives out Cashu ecash on the HyperPixel, one token QR at a time.
//! A token stays on screen until someone scans and claims it; then the next one appears.
//! When the balance runs below one drip, the screen shows a Lightning invoice to refill it.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdk::nuts::nut00::PaymentMethod;
use cdk::nuts::{CurrencyUnit, MintQuoteState, Token};
use cdk::wallet::{SendMemo, SendOptions, Wallet};
use cdk::Amount;
use serde::{Deserialize, Serialize};
use slint::{Rgba8Pixel, SharedPixelBuffer, SharedString};
use tokio::sync::mpsc;
use tokio::time::Instant;
use uuid::Uuid;

slint::include_modules!();

mod backdrop;
mod snapshot;

const MINT_URL: &str = "https://mint.minibits.cash/Bitcoin";
/// Sats per token until the owner picks another amount
const DEFAULT_DRIP: u64 = 21;
/// Seconds between a claim and the next token, so one person can't empty the faucet
const DEFAULT_COOLDOWN_SECS: u64 = 60;
/// The invoice amount shown when the faucet runs dry
const DRY_REFILL: u64 = 1000;
/// Memo the claiming wallet shows in its history
const MEMO: &str = "Pixel Faucet";

/// How often the token on screen (and any open refill invoice) is checked
const POLL_MS: u64 = 3000;
/// How long the "claimed" moment stays up before the next token appears
const CELEBRATE_MS: u64 = 4000;
/// How long a refill invoice is watched when the mint gives no expiry
const REFILL_TTL_SECS: u64 = 600;
/// Failed mint checks in a row before the screen says the mint is unreachable
const OFFLINE_AFTER: u32 = 3;

/// With animated QR codes on, tokens longer than this animate (NUT-16); shorter ones are
/// easy to scan as one static code anyway
const STATIC_QR_MAX_LEN: usize = 300;
/// Bytes of token per animated frame: about 49×49 modules, 8 px each on the card
const UR_FRAGMENT_LEN: usize = 120;
/// Animated QR frame interval (5 frames per second)
const QR_FRAME_MS: u64 = 200;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("pixel-faucet")
}

/// Render a QR code at exactly `size`×`size` pixels, ink modules on a transparent field
/// (so each theme's card shows through, and is the quiet zone), for the UI to
/// draw 1:1. Slint's software renderer must not scale it: its fixed-point stepping drifts
/// on non-integer scales and drops the last rows and columns (seen on the device as a code
/// clipped at the bottom and right). Here every pixel maps to its module with integer
/// maths, so modules come out 4 or 5 px wide and none go missing. There is no margin: the
/// white card around the code is its quiet zone.
fn qr_image(data: &str, size: u32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    use qrcode::{Color, EcLevel, QrCode};

    let code = QrCode::with_error_correction_level(data, EcLevel::L).map_err(|e| format!("QR error: {}", e))?;
    let modules = code.width();
    let dark = code.to_colors();
    let px = size as usize;
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(size, size);
    for (i, pixel) in buf.make_mut_slice().iter_mut().enumerate() {
        let (row, col) = ((i / px) * modules / px, (i % px) * modules / px);
        // Ink #1C1C1E: 15:1 or better on either theme's card
        let alpha = if dark[row * modules + col] == Color::Dark { 255 } else { 0 };
        *pixel = Rgba8Pixel { r: 0x1c, g: 0x1c, b: 0x1e, a: alpha };
    }
    Ok(buf)
}

/// The frames of an animated QR (NUT-16) for a token. CDK's own encoder re-serializes the
/// token and would put back the `"d": null` that some wallets reject, so the frames are
/// built here from the faucet's encoding: a UR of type `bytes` holding the token string
/// as a CBOR byte string, like CDK's.
struct Frames {
    encoder: ur::Encoder<'static>,
    /// The single-part form, when the token fits one frame
    single: Option<String>,
}

impl Frames {
    fn new(token: &str) -> Option<Frames> {
        let mut cbor = Vec::new();
        ciborium::into_writer(&ciborium::Value::Bytes(token.as_bytes().to_vec()), &mut cbor).ok()?;
        let encoder = ur::Encoder::bytes(&cbor, UR_FRAGMENT_LEN).ok()?;
        let single = (encoder.fragment_count() == 1).then(|| ur::encode(&cbor, &ur::Type::Bytes));
        Some(Frames { encoder, single })
    }

    /// The next frame, uppercased: URs are case-insensitive, and uppercase lets the QR use
    /// its denser alphanumeric mode
    fn next(&mut self) -> Option<String> {
        let part = match &self.single {
            Some(single) => single.clone(),
            None => self.encoder.next_part().ok()?,
        };
        Some(part.to_uppercase())
    }
}

/// Encode a token without its DLEQ proofs. They let a wallet check the token offline, but
/// they nearly double its length, and a claiming phone is online anyway: a 21-sat token
/// stays one static QR.
///
/// CDK writes a missing DLEQ as `"d": null`, which some wallets reject (Macadamia's
/// CashuSwift tries to parse a proof out of the null and fails the whole token). NUT-00
/// makes `d` optional, so the key is left out instead, which every decoder accepts.
fn encode_without_dleq(token: &Token) -> String {
    use base64::Engine as _;
    use ciborium::Value;

    fn entry<'a>(map: &'a mut Value, key: &str) -> Option<&'a mut Value> {
        match map {
            Value::Map(entries) => entries.iter_mut().find(|(k, _)| k.as_text() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    let Token::TokenV4(v4) = token else { return token.to_string() };
    let Ok(mut doc) = Value::serialized(v4) else { return token.to_string() };
    for group in entry(&mut doc, "t").and_then(Value::as_array_mut).into_iter().flatten() {
        for proof in entry(group, "p").and_then(Value::as_array_mut).into_iter().flatten() {
            if let Value::Map(fields) = proof {
                fields.retain(|(k, _)| k.as_text() != Some("d"));
            }
        }
    }
    let mut bytes = Vec::new();
    match ciborium::into_writer(&doc, &mut bytes) {
        Ok(()) => format!("cashuB{}", base64::engine::general_purpose::URL_SAFE.encode(bytes)),
        Err(_) => token.to_string(),
    }
}

async fn open_wallet() -> Result<Wallet, Box<dyn std::error::Error + Send + Sync>> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;

    let localstore = Arc::new(cdk_sqlite::wallet::WalletSqliteDatabase::new(&dir.join("wallet.db")).await?);

    let seed_path = dir.join("seed");
    let mut seed = [0u8; 64];
    if seed_path.exists() {
        let bytes = std::fs::read(&seed_path)?;
        seed.copy_from_slice(bytes.get(..64).ok_or("seed file is shorter than 64 bytes")?);
    } else {
        rand::Rng::fill(&mut rand::thread_rng(), &mut seed);
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&seed_path)?.write_all(&seed)?;
    }

    let wallet = Wallet::new(MINT_URL, CurrencyUnit::Sat, localstore, seed, None)?;
    // Settles interrupted operations. Unclaimed tokens stay pending; claimed ones are closed.
    wallet.recover_incomplete_sagas().await?;
    Ok(wallet)
}

/// A token handed out by the faucet
#[derive(Serialize, Deserialize, Clone)]
struct Drip {
    /// The wallet's send operation, used to check whether the token was claimed
    op: String,
    token: String,
    amount: u64,
}

/// What the faucet remembers across restarts (`faucet.json` next to the wallet)
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    drip: u64,
    /// Seconds to wait after a claim before the next token appears
    cooldown: u64,
    /// Show longer tokens as animated QR codes (NUT-16) rather than one dense static code
    animated: bool,
    /// The look: "dusk" or "berlin"
    theme: String,
    drips_given: u64,
    sats_given: u64,
    /// The token on screen, so a restart shows the same one instead of leaving it outstanding
    current: Option<Drip>,
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Berlin => "berlin",
        _ => "dusk",
    }
}

fn theme_from(name: &str) -> Theme {
    if name == "berlin" { Theme::Berlin } else { Theme::Dusk }
}

impl Default for Saved {
    fn default() -> Self {
        Saved { drip: DEFAULT_DRIP, cooldown: DEFAULT_COOLDOWN_SECS, animated: true, theme: "dusk".to_string(), drips_given: 0, sats_given: 0, current: None }
    }
}

impl Saved {
    fn load(path: &Path) -> Saved {
        std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Write via a temporary file so a power cut never leaves half a file
    fn store(&self, path: &Path) {
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::from)
            .and_then(|json| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            eprintln!("could not save {}: {}", path.display(), e);
        }
    }
}

/// Handle for pushing state to the screen from the faucet task
#[derive(Clone)]
struct Screen {
    app: slint::Weak<FaucetApp>,
    /// Frames of the animated token QR on screen, advanced by a UI timer
    anim: Arc<Mutex<Option<Frames>>>,
    /// On-screen QR sizes in pixels: on the main card, and in the owner panel
    qr_size: u32,
    sheet_qr_size: u32,
}

impl Screen {
    fn new(app: &FaucetApp) -> Screen {
        let layout = app.global::<Layout>();
        Screen {
            app: app.as_weak(),
            anim: Arc::new(Mutex::new(None)),
            qr_size: layout.get_qr_size() as u32,
            sheet_qr_size: layout.get_sheet_qr_size() as u32,
        }
    }

    fn update(&self, f: impl FnOnce(&Faucet) + Send + 'static) {
        let app = self.app.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app.upgrade() {
                f(&app.global::<Faucet>());
            }
        });
    }

    fn message(&self, text: impl Into<String>) {
        let text = SharedString::from(text.into());
        self.update(move |f| f.set_message(text));
    }

    fn phase(&self, phase: Phase) {
        self.update(move |f| f.set_phase(phase));
    }

    fn show_token(&self, drip: &Drip, animate: bool) {
        let mut frame = drip.token.clone();
        let mut frames = None;
        if animate && drip.token.len() > STATIC_QR_MAX_LEN {
            if let Some(mut f) = Frames::new(&drip.token) {
                if let Some(part) = f.next() {
                    frame = part;
                    frames = Some(f);
                }
            }
        }
        let animated = frames.is_some();
        *self.anim.lock().unwrap() = frames;
        println!("token: {} sat, {} chars{}", drip.amount, drip.token.len(), if animated { ", animated" } else { "" });

        match qr_image(&frame, self.qr_size) {
            Ok(pixels) => {
                let amount = drip.amount as i32;
                self.update(move |f| {
                    f.set_token_qr(slint::Image::from_rgba8(pixels));
                    f.set_token_animated(animated);
                    f.set_token_amount(amount);
                    f.set_phase(Phase::Dripping);
                });
            }
            Err(e) => self.message(e),
        }
    }
}

/// Requests from the owner panel
enum Cmd {
    SetDrip(u64),
    SetCooldown(u64),
    SetAnimated(bool),
    SetTheme(Theme),
    Refill(u64),
    CloseRefill,
    Reclaim,
}

/// A Lightning invoice being watched for payment
struct Refill {
    quote_id: String,
    amount: u64,
    expires: u64,
    by_owner: bool,
}

struct Dispenser {
    wallet: Wallet,
    saved: Saved,
    path: PathBuf,
    screen: Screen,
    refill: Option<Refill>,
    /// When the next token may appear (a claim is celebrated first, then the cooldown runs)
    next_drip_at: Instant,
    /// When the last token was claimed, while waiting to show the next one
    claimed_at: Option<Instant>,
    /// Whether the countdown to the next token is on screen
    resting: bool,
    failures: u32,
}

impl Dispenser {
    async fn balance(&self) -> u64 {
        self.wallet.total_balance().await.unwrap_or(Amount::ZERO).into()
    }

    async fn show_stats(&self) {
        let balance = self.balance().await as i32;
        let (drip, given, sats) = (self.saved.drip as i32, self.saved.drips_given as i32, self.saved.sats_given as i32);
        let (cooldown, animated) = (self.saved.cooldown as i32, self.saved.animated);
        self.screen.update(move |f| {
            f.set_balance(balance);
            f.set_drip(drip);
            f.set_cooldown(cooldown);
            f.set_animated_qr(animated);
            f.set_drips_given(given);
            f.set_sats_given(sats);
        });
    }

    /// Track whether the mint answers; say so after a few failures in a row
    fn mint_reachable(&mut self, ok: bool) {
        if ok {
            if self.failures >= OFFLINE_AFTER {
                self.screen.message("");
            }
            self.failures = 0;
        } else {
            self.failures += 1;
            if self.failures == OFFLINE_AFTER {
                self.screen.message("Can't reach the mint. Retrying…");
            }
        }
    }

    fn current_op(&self) -> Option<Uuid> {
        self.saved.current.as_ref().and_then(|d| Uuid::parse_str(&d.op).ok())
    }

    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Cmd>) {
        self.startup().await;
        let mut tick = tokio::time::interval(Duration::from_millis(POLL_MS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // Wake for the end of the claimed moment and for the next drip, not just on ticks
            let now = Instant::now();
            let rest_at = self.claimed_at.filter(|_| !self.resting).map(|at| at + Duration::from_millis(CELEBRATE_MS));
            let wake = [Some(self.next_drip_at), rest_at].into_iter().flatten().filter(|t| *t > now).min();
            let waiting = self.saved.current.is_none() && wake.is_some();
            tokio::select! {
                _ = tick.tick() => {}
                _ = tokio::time::sleep_until(wake.unwrap_or(now)), if waiting => {}
                cmd = commands.recv() => match cmd {
                    Some(cmd) => self.handle(cmd).await,
                    None => return,
                },
            }
            self.step().await;
        }
    }

    async fn startup(&mut self) {
        // Invoices paid while the app was closed
        if let Ok(minted) = self.wallet.mint_unissued_quotes().await {
            if minted > Amount::ZERO {
                self.screen.message(format!("Received {} sats from a paid refill", u64::from(minted)));
            }
        }

        // The token that was on screen when the app last closed: show it again if it's
        // still unclaimed. Recovery already closed it if it was claimed in the meantime.
        let pending = self.wallet.get_pending_sends().await.unwrap_or_default();
        if let Some(mut drip) = self.saved.current.clone() {
            if self.current_op().map_or(false, |op| pending.contains(&op)) {
                // Re-encode in case it was saved by a build that wrote `"d": null`
                if let Ok(token) = Token::from_str(&drip.token) {
                    drip.token = encode_without_dleq(&token);
                    self.saved.current = Some(drip.clone());
                }
                self.screen.show_token(&drip, self.saved.animated);
            } else {
                self.count_claim(&drip);
            }
        }

        // Tokens made but never shown (the app stopped between making one and saving it)
        let current = self.current_op();
        for op in pending.into_iter().filter(|op| Some(*op) != current) {
            let _ = self.wallet.revoke_send(op).await;
        }

        self.saved.store(&self.path);
        self.show_stats().await;
    }

    async fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetDrip(amount) => {
                if amount == self.saved.drip {
                    return;
                }
                self.saved.drip = amount;
                // Swap the token on screen for one of the new amount
                if self.saved.current.as_ref().map_or(false, |d| d.amount != amount) {
                    self.take_back_current().await;
                }
                self.saved.store(&self.path);
                self.show_stats().await;
            }
            Cmd::SetCooldown(secs) => {
                self.saved.cooldown = secs;
                self.saved.store(&self.path);
                // A countdown already running follows the new setting
                if let Some(at) = self.claimed_at {
                    self.next_drip_at = at + Duration::from_millis(CELEBRATE_MS) + Duration::from_secs(secs);
                    if self.resting {
                        self.show_countdown();
                    }
                }
                self.show_stats().await;
            }
            // The UI has already switched; remember the choice
            Cmd::SetTheme(theme) => {
                self.saved.theme = theme_name(theme).to_string();
                self.saved.store(&self.path);
            }
            Cmd::SetAnimated(animated) => {
                self.saved.animated = animated;
                self.saved.store(&self.path);
                if let Some(drip) = &self.saved.current {
                    self.screen.show_token(drip, animated);
                }
                self.show_stats().await;
            }
            Cmd::Refill(amount) => self.open_refill(amount, true).await,
            // The dry screen's own invoice stays open; an owner's one (paid or not) is dropped
            Cmd::CloseRefill => {
                if self.refill.as_ref().map_or(true, |r| r.by_owner) {
                    self.refill = None;
                    self.screen.update(|f| {
                        f.set_refill_ready(false);
                        f.set_refill_received(0);
                    });
                }
            }
            Cmd::Reclaim => {
                let before = self.balance().await;
                self.take_back_current().await;
                for op in self.wallet.get_pending_sends().await.unwrap_or_default() {
                    let _ = self.wallet.revoke_send(op).await;
                }
                let _ = self.wallet.check_all_pending_proofs().await;
                let back = self.balance().await.saturating_sub(before);
                self.saved.store(&self.path);
                self.screen.message(if back > 0 {
                    format!("Took back {} sats from unclaimed tokens", back)
                } else {
                    "No unclaimed tokens to take back".to_string()
                });
                self.show_stats().await;
            }
        }
    }

    /// Take back the token on screen, if nobody has claimed it; the next step makes a new one
    async fn take_back_current(&mut self) {
        let Some(op) = self.current_op() else { return };
        match self.wallet.revoke_send(op).await {
            Ok(_) => {
                self.saved.current = None;
                self.next_drip_at = Instant::now();
            }
            // Claimed in the meantime, or the mint is unreachable: the next check sorts it out
            Err(e) => eprintln!("could not take back token: {}", e),
        }
    }

    async fn step(&mut self) {
        self.check_refill().await;

        if let Some(drip) = self.saved.current.clone() {
            let Some(op) = self.current_op() else {
                self.saved.current = None;
                return;
            };
            // Closed by the wallet already (claimed and settled during recovery)
            if !self.wallet.get_pending_sends().await.unwrap_or_default().contains(&op) {
                self.claimed(&drip).await;
                return;
            }
            match self.wallet.check_send_status(op).await {
                Ok(true) => {
                    self.mint_reachable(true);
                    self.claimed(&drip).await;
                }
                Ok(false) => self.mint_reachable(true),
                Err(_) => self.mint_reachable(false),
            }
            return;
        }

        let now = Instant::now();
        if now >= self.next_drip_at {
            self.claimed_at = None;
            self.resting = false;
            self.dispense().await;
        } else if let Some(at) = self.claimed_at {
            // After the claimed moment, count down to the next token
            if !self.resting && now >= at + Duration::from_millis(CELEBRATE_MS) {
                self.resting = true;
                self.show_countdown();
            }
        }
    }

    fn show_countdown(&self) {
        let left = self.next_drip_at.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as i32;
        let total = self.saved.cooldown as i32;
        self.screen.update(move |f| {
            f.set_cooldown_total(total.max(1));
            f.set_cooldown_left(left);
            f.set_phase(Phase::Cooling);
        });
    }

    fn count_claim(&mut self, drip: &Drip) {
        self.saved.drips_given += 1;
        self.saved.sats_given += drip.amount;
        self.saved.current = None;
    }

    async fn claimed(&mut self, drip: &Drip) {
        self.count_claim(drip);
        self.saved.store(&self.path);
        let now = Instant::now();
        self.claimed_at = Some(now);
        self.resting = false;
        self.next_drip_at = now + Duration::from_millis(CELEBRATE_MS) + Duration::from_secs(self.saved.cooldown);
        *self.screen.anim.lock().unwrap() = None;
        self.screen.phase(Phase::Claimed);
        self.show_stats().await;
    }

    async fn dispense(&mut self) {
        let amount = self.saved.drip;
        if self.balance().await < amount {
            self.run_dry().await;
            return;
        }
        match self.make_token(amount).await {
            Ok(drip) => {
                self.mint_reachable(true);
                self.saved.current = Some(drip.clone());
                self.saved.store(&self.path);
                self.screen.show_token(&drip, self.saved.animated);
                self.show_stats().await;
            }
            // Enough sats, but not once the mint's fee is added
            Err(cdk::Error::InsufficientFunds) => self.run_dry().await,
            Err(e) => {
                eprintln!("could not make a token: {}", e);
                self.mint_reachable(false);
            }
        }
    }

    async fn make_token(&self, amount: u64) -> Result<Drip, cdk::Error> {
        // The token carries the mint's redeem fee, so the claimer gets the full drip
        let options = SendOptions { include_fee: true, ..Default::default() };
        let prepared = self.wallet.prepare_send(Amount::from(amount), options).await?;
        let op = prepared.operation_id();
        let token = prepared.confirm(Some(SendMemo { memo: MEMO.to_string(), include_memo: true })).await?;
        Ok(Drip { op: op.to_string(), token: encode_without_dleq(&token), amount })
    }

    /// Too few sats for another drip: show a refill invoice anyone can pay
    async fn run_dry(&mut self) {
        self.screen.phase(Phase::Empty);
        if self.refill.is_none() {
            self.open_refill(DRY_REFILL, false).await;
        }
    }

    async fn open_refill(&mut self, amount: u64, by_owner: bool) {
        if by_owner {
            self.screen.update(|f| {
                f.set_refill_ready(false);
                f.set_refill_received(0);
            });
        }
        let quote = match self.wallet.mint_quote(PaymentMethod::BOLT11, Some(Amount::from(amount)), None, None).await {
            Ok(quote) => quote,
            Err(e) => {
                self.screen.message(format!("Couldn't create an invoice: {}", e));
                self.mint_reachable(false);
                return;
            }
        };
        // BOLT11 is case-insensitive; uppercase lets the QR use its denser alphanumeric mode
        let invoice = quote.request.to_uppercase();
        let (pixels, small) = match (qr_image(&invoice, self.screen.qr_size), qr_image(&invoice, self.screen.sheet_qr_size)) {
            (Ok(pixels), Ok(small)) => (pixels, small),
            (Err(e), _) | (_, Err(e)) => {
                self.screen.message(e);
                return;
            }
        };
        let expires = if quote.expiry > 0 { quote.expiry } else { now_secs() + REFILL_TTL_SECS };
        self.refill = Some(Refill { quote_id: quote.id, amount, expires, by_owner });
        self.screen.update(move |f| {
            f.set_refill_qr(slint::Image::from_rgba8(pixels));
            f.set_refill_qr_small(slint::Image::from_rgba8(small));
            f.set_refill_amount(amount as i32);
            f.set_refill_received(0);
            f.set_refill_ready(true);
        });
    }

    async fn check_refill(&mut self) {
        let Some(refill) = self.refill.as_ref() else { return };
        if now_secs() > refill.expires {
            // The dry screen opens a fresh one; the owner panel asks again
            self.refill = None;
            self.screen.update(|f| f.set_refill_ready(false));
            return;
        }
        let (quote_id, amount, by_owner) = (refill.quote_id.clone(), refill.amount, refill.by_owner);
        match self.wallet.check_mint_quote_status(&quote_id).await {
            Ok(quote) if quote.state == MintQuoteState::Paid => {}
            Ok(_) => return,
            Err(_) => return self.mint_reachable(false),
        }
        match self.wallet.mint(&quote_id, Default::default(), None).await {
            Ok(_) => {
                self.refill = None;
                // A refill on the dry screen gets a moment of thanks before the next token
                self.next_drip_at = Instant::now() + Duration::from_millis(CELEBRATE_MS);
                self.screen.update(move |f| {
                    f.set_refill_received(amount as i32);
                    // The owner sheet shows its paid invoice until closed; the dry screen moves on
                    f.set_refill_ready(by_owner);
                });
                self.show_stats().await;
            }
            // Paid but not minted yet: the next check retries
            Err(e) => self.screen.message(format!("Invoice paid; minting failed, retrying: {}", e)),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let dir = args.get(i + 1).map_or_else(|| PathBuf::from("snapshots"), PathBuf::from);
        return snapshot::run(&dir);
    }

    let app = FaucetApp::new()?;
    let screen = Screen::new(&app);

    // Start in the saved look
    let theme = theme_from(&Saved::load(&data_dir().join("faucet.json")).theme);
    app.global::<Faucet>().set_theme(theme);

    // Paint both themes' backgrounds off the UI thread, the one on screen first, so the
    // other is ready by the time anyone switches; the start screen shows until then
    let (dusk, berlin) = (backdrop::dusk::Geometry::of(&app), backdrop::berlin::Geometry::of(&app));
    let painter = screen.clone();
    std::thread::spawn(move || {
        let paint_dusk = || {
            let started = std::time::Instant::now();
            let backdrops = backdrop::dusk::paint(&dusk);
            println!("dusk backdrops painted in {:?}", started.elapsed());
            painter.update(move |f| backdrops.install(f));
        };
        let paint_berlin = || {
            let started = std::time::Instant::now();
            let backdrops = backdrop::berlin::paint(&berlin);
            println!("berlin backdrops painted in {:?}", started.elapsed());
            painter.update(move |f| backdrops.install(f));
        };
        if theme == Theme::Berlin {
            paint_berlin();
            paint_dusk();
        } else {
            paint_dusk();
            paint_berlin();
        }
    });
    let (commands, receiver) = mpsc::unbounded_channel();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build()?;

    let state = app.global::<Faucet>();
    state.set_mint_host(SharedString::from(MINT_URL.trim_start_matches("https://").split('/').next().unwrap_or(MINT_URL)));
    state.set_drip(DEFAULT_DRIP as i32);
    let send = move |cmd: Cmd| {
        let _ = commands.send(cmd);
    };
    let send_drip = send.clone();
    state.on_set_drip(move |amount| send_drip(Cmd::SetDrip(amount.max(1) as u64)));
    let send_theme = send.clone();
    state.on_set_theme(move |theme| send_theme(Cmd::SetTheme(theme)));
    let send_animated = send.clone();
    state.on_set_animated(move |animated| send_animated(Cmd::SetAnimated(animated)));
    let send_cooldown = send.clone();
    state.on_set_cooldown(move |secs| send_cooldown(Cmd::SetCooldown(secs.max(0) as u64)));
    let send_refill = send.clone();
    state.on_request_refill(move |amount| send_refill(Cmd::Refill(amount.max(1) as u64)));
    let send_close = send.clone();
    state.on_close_refill(move || send_close(Cmd::CloseRefill));
    state.on_reclaim(move || send(Cmd::Reclaim));
    state.on_exit_app(|| {
        let _ = slint::quit_event_loop();
    });

    // Advance the animated token QR, only while it's on screen
    let frame_timer = slint::Timer::default();
    {
        let app = app.as_weak();
        let anim = screen.anim.clone();
        let size = screen.qr_size;
        frame_timer.start(slint::TimerMode::Repeated, Duration::from_millis(QR_FRAME_MS), move || {
            let Some(app) = app.upgrade() else { return };
            let state = app.global::<Faucet>();
            if !state.get_token_animated() || state.get_phase() != Phase::Dripping {
                return;
            }
            let Some(part) = anim.lock().unwrap().as_mut().and_then(Frames::next) else { return };
            if let Ok(pixels) = qr_image(&part, size) {
                state.set_token_qr(slint::Image::from_rgba8(pixels));
            }
        });
    }

    rt.spawn(async move {
        match open_wallet().await {
            Ok(wallet) => {
                let path = data_dir().join("faucet.json");
                let saved = Saved::load(&path);
                let dispenser = Dispenser {
                    wallet,
                    saved,
                    path,
                    screen,
                    refill: None,
                    next_drip_at: Instant::now(),
                    claimed_at: None,
                    resting: false,
                    failures: 0,
                };
                dispenser.run(receiver).await;
            }
            Err(e) => {
                let text = SharedString::from(format!("The wallet didn't open: {}", e));
                screen.update(move |f| {
                    f.set_message(text);
                    f.set_phase(Phase::Error);
                });
            }
        }
    });

    app.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdk::mint_url::MintUrl;
    use cdk::nuts::{Id, Proof, SecretKey};
    use cdk::secret::Secret;

    #[test]
    fn token_without_dleq_omits_the_key_and_round_trips() {
        let keyset = Id::from_str("00500550f0494146").unwrap();
        let proofs = [16u64, 4, 1]
            .into_iter()
            .map(|a| Proof::new(Amount::from(a), keyset, Secret::generate(), SecretKey::generate().public_key()))
            .collect::<Vec<_>>();
        let token = Token::new(MintUrl::from_str(MINT_URL).unwrap(), proofs, Some(MEMO.to_string()), CurrencyUnit::Sat);

        // CDK's own encoding writes the missing DLEQ as a null, which some wallets reject
        assert!(cbor_of(&token.to_string()).contains("\"d\": null"));

        let encoded = encode_without_dleq(&token);
        let doc = cbor_of(&encoded);
        assert!(!doc.contains("\"d\": null"), "{}", doc);
        assert!(doc.contains(MEMO), "the token memo (also `d`) must stay");
        assert!(encoded.len() < token.to_string().len());

        let decoded = Token::from_str(&encoded).unwrap();
        assert_eq!(decoded, token);
    }

    #[test]
    fn animated_frames_reassemble_to_our_encoding() {
        let keyset = Id::from_str("00500550f0494146").unwrap();
        let proofs = [16u64, 4, 1]
            .into_iter()
            .map(|a| Proof::new(Amount::from(a), keyset, Secret::generate(), SecretKey::generate().public_key()))
            .collect::<Vec<_>>();
        let token = Token::new(MintUrl::from_str(MINT_URL).unwrap(), proofs, Some(MEMO.to_string()), CurrencyUnit::Sat);
        let encoded = encode_without_dleq(&token);

        let mut frames = Frames::new(&encoded).unwrap();
        assert!(frames.single.is_none(), "a 21-sat token should span several frames");
        let mut decoder = cdk::nuts::TokenUrDecoder::default();
        for _ in 0..100 {
            let part = frames.next().unwrap();
            assert_eq!(part, part.to_uppercase());
            decoder.receive(&part).unwrap();
            if decoder.complete() {
                break;
            }
        }
        assert!(decoder.complete());
        assert_eq!(decoder.token().unwrap(), Some(token));
    }

    #[test]
    fn qr_image_keeps_every_module() {
        // 81 modules drawn at 392 px: 4 or 5 px each, and the last row and column are there
        let data = "x".repeat(546);
        let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L).unwrap();
        assert_eq!(code.width(), 81);
        let image = qr_image(&data, 392).unwrap();
        let px = image.as_slice();
        let dark = |x: usize, y: usize| px[y * 392 + x].a == 255;
        // All three finder patterns are 7 modules (33 or 34 px) across
        let run = |xs: &mut dyn Iterator<Item = (usize, usize)>| xs.take_while(|&(x, y)| dark(x, y)).count();
        assert!((33..=35).contains(&run(&mut (0..392).map(|x| (x, 0)))));
        assert!((33..=35).contains(&run(&mut (0..392).rev().map(|x| (x, 0)))));
        assert!((33..=35).contains(&run(&mut (0..392).rev().map(|y| (0, y)))));
    }

    /// The CBOR inside a `cashuB` token, as JSON-ish text
    fn cbor_of(token: &str) -> String {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::URL_SAFE.decode(token.strip_prefix("cashuB").unwrap()).unwrap();
        let value: ciborium::Value = ciborium::from_reader(&bytes[..]).unwrap();
        format!("{:?}", value).replace("Text(\"d\"), Null", "\"d\": null").replace(&format!("Text(\"{}\")", MEMO), MEMO)
    }
}

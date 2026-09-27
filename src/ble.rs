//! Receive ecash over Bluetooth LE.
//!
//! While the Bluetooth receive screen is open, the Pi advertises as "Cashu NERV" with the
//! Nordic UART Service: the de facto BLE serial port that terminal apps (Serial Bluetooth
//! Terminal, nRF Toolbox, LightBlue) speak. A phone writes a Cashu token to the RX
//! characteristic in MTU-sized pieces; once they add up to a whole token, the screen shows
//! it for the user to redeem. Short replies go back to the phone on the TX characteristic.
//! No pairing: a token only ever gives this wallet sats.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bluer::adv::Advertisement;
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotifier, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicWrite, CharacteristicWriteMethod, Service,
};
use bluer::Uuid;
use cdk::nuts::Token;
use tokio::sync::{mpsc, oneshot, Mutex};

/// The name phones see when they scan
pub const NAME: &str = "Cashu NERV";

const NUS_SERVICE: Uuid = Uuid::from_u128(0x6e400001_b5a3_f393_e0a9_e50e24dcca9e);
/// Phone to Pi
const NUS_RX: Uuid = Uuid::from_u128(0x6e400002_b5a3_f393_e0a9_e50e24dcca9e);
/// Pi to phone
const NUS_TX: Uuid = Uuid::from_u128(0x6e400003_b5a3_f393_e0a9_e50e24dcca9e);

/// Text held while waiting for a token to complete; far more than any sane token
const MAX_TEXT: usize = 64 * 1024;
/// Notifications are sent in pieces that fit the smallest BLE MTU (23 bytes, minus 3)
const NOTIFY_CHUNK: usize = 20;
/// How often the link is checked for a phone connecting or leaving
const LINK_POLL_MS: u64 = 700;

/// What the Bluetooth session reports to the app
pub enum Event {
    /// Advertising: phones can find us
    Ready,
    /// A phone connected (true) or the last one left (false)
    Linked(bool),
    /// Part of something has arrived: this many bytes so far
    Receiving(usize),
    /// A whole token arrived
    Token(Token),
    /// A line ended without a token in it
    NotToken,
    /// Bluetooth could not start
    Failed(String),
}

/// Collects the pieces a phone writes until they hold a whole Cashu token
#[derive(Default)]
pub struct Assembler {
    text: String,
}

impl Assembler {
    /// Adds one write and says what it completed. Returns nothing for writes that only
    /// carry whitespace, such as the line ending a terminal sends after a token.
    pub fn push(&mut self, bytes: &[u8]) -> Option<Event> {
        self.text.push_str(&String::from_utf8_lossy(bytes));
        if let Some(token) = find_token(&self.text) {
            self.text.clear();
            return Some(Event::Token(token));
        }
        let ended = self.text.contains(['\r', '\n']);
        if self.text.trim().is_empty() {
            self.text.clear();
            return None;
        }
        if ended || self.text.len() > MAX_TEXT {
            self.text.clear();
            return Some(Event::NotToken);
        }
        Some(Event::Receiving(self.text.len()))
    }

    /// Drops a half-received token, as when the phone disconnects
    pub fn clear(&mut self) {
        self.text.clear();
    }
}

/// The first complete token in `text`: `cashuA…` or `cashuB…`, with or without a
/// `cashu:` URI prefix, running up to the first whitespace
fn find_token(text: &str) -> Option<Token> {
    let start = ["cashuA", "cashuB"].iter().filter_map(|p| text.find(p)).min()?;
    let candidate = text[start..].split_whitespace().next()?;
    Token::from_str(candidate).ok()
}

/// The way back to the phone: short text lines on the TX characteristic
#[derive(Clone, Default)]
pub struct Link {
    notifier: Arc<Mutex<Option<CharacteristicNotifier>>>,
}

impl Link {
    /// Sends one line to the phone, if it's listening
    pub async fn reply(&self, text: &str) {
        let mut guard = self.notifier.lock().await;
        let Some(notifier) = guard.as_mut() else { return };
        let line = format!("{}\r\n", text);
        for piece in line.as_bytes().chunks(NOTIFY_CHUNK) {
            if notifier.notify(piece.to_vec()).await.is_err() {
                *guard = None;
                return;
            }
        }
    }

    pub async fn forget(&self) {
        *self.notifier.lock().await = None;
    }
}

/// Advertises and serves the UART service until `stop` fires; dropping the handles then
/// unregisters both. Everything that happens is sent to `events`.
pub async fn serve(events: mpsc::UnboundedSender<Event>, link: Link, stop: oneshot::Receiver<()>) {
    if let Err(e) = run(events.clone(), link, stop).await {
        let _ = events.send(Event::Failed(e.to_string()));
    }
}

async fn run(events: mpsc::UnboundedSender<Event>, link: Link, mut stop: oneshot::Receiver<()>) -> bluer::Result<()> {
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    let assembler = Arc::new(std::sync::Mutex::new(Assembler::default()));
    let on_write = {
        let (assembler, events) = (assembler.clone(), events.clone());
        CharacteristicWriteMethod::Fun(Box::new(move |value, _request| {
            if let Some(event) = assembler.lock().unwrap().push(&value) {
                let _ = events.send(event);
            }
            Box::pin(async { Ok(()) })
        }))
    };
    let on_subscribe = {
        let link = link.clone();
        CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
            let link = link.clone();
            Box::pin(async move {
                *link.notifier.lock().await = Some(notifier);
                link.reply("Cashu NERV is listening. Paste a Cashu token and send it.").await;
            })
        }))
    };
    let app = Application {
        services: vec![Service {
            uuid: NUS_SERVICE,
            primary: true,
            characteristics: vec![
                Characteristic {
                    uuid: NUS_RX,
                    write: Some(CharacteristicWrite {
                        write: true,
                        write_without_response: true,
                        method: on_write,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Characteristic {
                    uuid: NUS_TX,
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: on_subscribe,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app = adapter.serve_gatt_application(app).await?;
    let _advertising = adapter
        .advertise(Advertisement {
            service_uuids: [NUS_SERVICE].into(),
            local_name: Some(NAME.to_string()),
            discoverable: Some(true),
            ..Default::default()
        })
        .await?;
    let _ = events.send(Event::Ready);

    // Report phones connecting and leaving until the screen closes
    let mut linked = false;
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = tokio::time::sleep(Duration::from_millis(LINK_POLL_MS)) => {}
        }
        let mut now = false;
        for address in adapter.device_addresses().await.unwrap_or_default() {
            if let Ok(device) = adapter.device(address) {
                if device.is_connected().await.unwrap_or(false) {
                    now = true;
                    break;
                }
            }
        }
        if now != linked {
            linked = now;
            if !linked {
                assembler.lock().unwrap().clear();
                link.forget().await;
            }
            let _ = events.send(Event::Linked(linked));
        }
    }
    link.forget().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use cdk::mint_url::MintUrl;
    use cdk::nuts::{CurrencyUnit, Id, Proof, SecretKey};
    use cdk::secret::Secret;
    use cdk::Amount;

    /// A 21 sat token, as a wallet would encode it
    fn token() -> String {
        let keyset = Id::from_str("00500550f0494146").unwrap();
        let proofs = [16u64, 4, 1]
            .into_iter()
            .map(|a| Proof::new(Amount::from(a), keyset, Secret::generate(), SecretKey::generate().public_key()))
            .collect();
        let mint = MintUrl::from_str("https://mint.minibits.cash/Bitcoin").unwrap();
        Token::new(mint, proofs, None, CurrencyUnit::Sat).to_string()
    }

    fn pieces(text: &str, size: usize) -> Vec<Event> {
        let mut assembler = Assembler::default();
        text.as_bytes().chunks(size).filter_map(|piece| assembler.push(piece)).collect()
    }

    #[test]
    fn a_token_in_mtu_sized_pieces_completes_once() {
        let events = pieces(&format!("{}\r\n", token()), 20);
        let tokens = events.iter().filter(|e| matches!(e, Event::Token(_))).count();
        assert_eq!(tokens, 1);
        assert!(matches!(events.last(), Some(Event::Token(_))));
        let Some(Event::Token(token)) = events.last() else { unreachable!() };
        assert_eq!(u64::from(token.value().unwrap()), 21);
    }

    #[test]
    fn a_uri_prefix_is_fine() {
        assert!(matches!(pieces(&format!("cashu:{}\n", token()), 64).last(), Some(Event::Token(_))));
    }

    #[test]
    fn a_line_without_a_token_is_rejected() {
        assert!(matches!(pieces("hello there\r\n", 20).last(), Some(Event::NotToken)));
        // A cut-off token is not a token either
        assert!(matches!(pieces(&format!("{}\r\n", &token()[..200]), 20).last(), Some(Event::NotToken)));
    }

    #[test]
    fn bare_line_endings_are_ignored() {
        assert!(pieces("\r\n", 20).is_empty());
    }
}

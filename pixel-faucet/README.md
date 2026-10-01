# Pixel Faucet

A Cashu ecash faucet for the Raspberry Pi Zero 2 W and the HyperPixel 4.0 Square (720×720 touch).

The screen always shows one token QR, 21 sats by default. When someone scans and claims it with a Cashu wallet, the faucet shows a short "claimed" moment. It then pauses (1 minute by default), so one person can't empty it. The countdown runs in the headline while the card cycles through Cashu and ecash trivia, and then the next token appears. When the balance runs below one drip, the screen shows a Lightning invoice that anyone can pay to refill it.

- **Own wallet:** it keeps its seed, `wallet.db` and `faucet.json` (drip size, mint, stats, the token on screen) in `~/.local/share/pixel-faucet`. It never touches Cashu NERV's wallet.
- **Mint:** Minibits until the owner picks another under Settings › Mint. The list is `shared/known-mints.txt` plus any in `~/.local/share/pixel-faucet/mints.txt`, and it shows the sats the faucet holds at each. Switching checks the new mint (it must mint and pay sat over Lightning), takes back the token on screen, and moves every sat over Lightning: the new mint issues an invoice and the old one pays it, less the Lightning fee. Change from the fee reserve stays at the old mint. If the payment fails, the faucet stays on the old mint, and the owner can switch anyway and leave the sats there; switching back later finds them again.
- **Restarts:** the token on screen is saved, so after a restart the faucet shows the same token again instead of leaving it outstanding. Tokens that were created but never shown are taken back at startup.
- **Small tokens:** tokens drop their DLEQ proofs. The proofs let a wallet verify a token offline, but they nearly double its length, and a phone claiming the token is online anyway. The faucet encodes tokens itself and leaves the `d` key out. CDK would write it as `null`, and Macadamia (CashuSwift) rejects the whole token when it does.
- **Animated QR by default:** tokens over 300 characters are shown as animated QRs (NUT-16), about 49×49 modules per frame at 5 frames a second. The frames are built from the faucet's own encoding, not by CDK's encoder, which would put the `null` back. The owner panel can switch to one static QR, for wallets without NUT-16.
- **Exact-size QRs:** QR codes are rendered in Rust at their exact on-screen size and drawn 1:1. Slint's software renderer drops the last rows and columns when it scales an image by a non-integer factor.
- **Settings:** the gear at the top right. From there you can set the sats per drip, the time between drips (off, 30 s, 1 min or 5 min), animated or static QR, the theme and the mint; refill with Lightning; take back unclaimed tokens; or exit to the desktop.

## Themes

The faucet has two looks. Switch between them under **Settings**, the gear at the top right. The choice is saved in `faucet.json`.

**Dusk** is the faucet's own look, inspired by umbrelOS:
- dark glass over a cinematic wallpaper
- white text at a few fixed opacities, one amber accent taken from the wallpaper, Inter type, and pill buttons

It borrows design values and ideas only. No Umbrel code, wallpapers, icons or other assets are used, because Umbrel is PolyForm Noncommercial and its assets are unlicensed. The wallpaper is procedural: a sun setting behind the QR card over long-exposure water.

**bitcoin++ Berlin** is styled after [btcpp.dev/berlin26](https://btcpp.dev/berlin26):
- **Colours:** paper `#F6F3EE` and ink `#1C1C1E`, with one apricot accent `#F9AF5E` used as flat slabs
- **Shapes:** square corners, 2 px ink rules in a ledger-style grid, and hard offset shadows
- **Type:**
  - IBM Plex Mono for the uppercase labels and buttons
  - IBM Plex Sans Bold for statements
  - Source Serif 4 italic for asides and the one italic word in each headline ("Scan. Get *paid.*")
  - Ubuntu Bold Italic for the `bitcoin++` wordmark
- **Art:** the event's market illustration as the hero

In both themes the card turns into Cashu trivia during the pause between claims. The countdown stays in the headline.

**How the themes are built:**
- **Rendering limits:** Slint's software renderer can't draw blur, shadows or gradients on rounded shapes. So `src/backdrop/` paints each theme's background once at startup, the theme on screen first, which makes switching instant:
  - `dusk.rs` bakes the wallpaper, the white card and its shadow, the frosted glass widgets and the settings panel's plate.
  - `berlin.rs` shades the market art (`ui/assets/berlin-market.png`) so the text over it stays legible.

  On the Pi, Dusk takes about 3 s and bitcoin++ about 1 s.
- **Files:** `ui/dusk.slint` and `ui/berlin.slint` each hold a theme's screens and settings panel. They share `ui/state.slint` and `ui/common.slint` (formatting, trivia, QR codes, the gear), and `ui/faucet.slint` is the window that picks between them.
- **QR codes:** rendered in Rust at their exact on-screen size, as ink modules on a transparent field, and drawn 1:1. Slint's software renderer drops rows and columns when it scales an image by a non-integer factor.

**Licences:**
- Inter, IBM Plex and Source Serif 4 are under the SIL Open Font License, and Ubuntu is under the Ubuntu Font Licence; see `ui/fonts/licenses/`.
- The bitcoin++ name, wordmark, sparkles and artwork belong to bitcoin++. Get the organisers' OK before showing that theme at their event.

## Build

From the repo root (a Cargo workspace with Cashu NERV), using the arm64 build image described in `Dockerfile.build`:

```sh
docker run --rm --platform linux/arm64 \
  -v cashu-pixel-target:/app/target -v "$PWD":/app -w /app cashu-pixel-build \
  cargo build --release -p pixel-faucet
```

## Snapshots

`pixel-faucet --snapshot <dir>` renders every screen with sample data to PNG files. It uses the same software renderer as the device and doesn't touch the wallet, so it runs inside the build container:

```sh
docker run --rm --platform linux/arm64 -v cashu-pixel-target:/app/target -v "$PWD":/app -w /app \
  cashu-pixel-build ./target/release/pixel-faucet --snapshot snapshots
```

The tests (token encoding, animated frames, QR rendering) run in the same container with `cargo test --release -p pixel-faucet`.

## Install on the Pi

Copy the binary and `deploy/` to the Pi, then run this there:

```sh
deploy/install.sh ./pixel-faucet
```

This adds a menu entry, a desktop icon and a taskbar launcher. The launcher runs the app fullscreen inside the desktop, or straight on the display (KMS) when no desktop is running. Its log is `~/.cache/pixel-faucet.log`.

# Pixel Faucet

A Cashu ecash faucet for the Raspberry Pi Zero 2 W and the HyperPixel 4.0 Square (720×720 touch).

The screen always shows one token QR, 21 sats by default. When someone scans and claims it with a Cashu wallet, the faucet shows a short "claimed" moment. It then pauses (1 minute by default), so one person can't empty it. The countdown runs in the headline while the card cycles through Cashu and ecash trivia, and then the next token appears. When the balance runs below one drip, the screen shows a Lightning invoice that anyone can pay to refill it.

- **Own wallet:** it keeps its seed, `wallet.db` and `faucet.json` (drip size, stats, the token on screen) in `~/.local/share/pixel-faucet`. It never touches Cashu NERV's wallet.
- **Mint:** Minibits, the same one Cashu NERV uses.
- **Restarts:** the token on screen is saved, so after a restart the faucet shows the same token again instead of leaving it outstanding. Tokens that were created but never shown are taken back at startup.
- **Small tokens:** tokens drop their DLEQ proofs. The proofs let a wallet verify a token offline, but they nearly double its length, and a phone claiming the token is online anyway. The faucet encodes tokens itself and leaves the `d` key out. CDK would write it as `null`, and Macadamia (CashuSwift) rejects the whole token when it does.
- **Animated QR by default:** tokens over 300 characters are shown as animated QRs (NUT-16), about 49×49 modules per frame at 5 frames a second. The frames are built from the faucet's own encoding, not by CDK's encoder, which would put the `null` back. The owner panel can switch to one static QR, for wallets without NUT-16.
- **Exact-size QRs:** QR codes are rendered in Rust at their exact on-screen size and drawn 1:1. Slint's software renderer drops the last rows and columns when it scales an image by a non-integer factor.
- **Owner panel:** hold the top of the screen for about a second. From there you can set the sats per drip, the time between drips (off, 30 s, 1 min or 5 min), and animated or static QR; refill with Lightning; take back unclaimed tokens; or exit to the desktop.

## Design

The look takes its cues from umbrelOS: dark glass over a cinematic wallpaper, white text at a few fixed opacities, one accent colour taken from the wallpaper, Inter type, and pill buttons. It borrows only design values and ideas. No Umbrel code, wallpapers, icons or other assets are used, because Umbrel is PolyForm Noncommercial and its assets are unlicensed.

The Slint software renderer can't draw blur, shadows or gradients on rounded shapes. So `src/backdrop.rs` paints everything soft once at startup, which takes about 3 s on the Pi:

- a procedural wallpaper of a sun setting over long-exposure water
- the white QR card and its shadow
- the frosted glass widgets, with light bending at their rims
- the owner panel's plate

`ui/faucet.slint` draws only text, QR codes and solid shapes on top. The two share their geometry through the `Layout` global.

Inter is under the SIL Open Font License; see `ui/fonts/OFL.txt`.

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

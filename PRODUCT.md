# Product

<!-- impeccable:product-schema 1 -->

## Platform

embedded

## Users

Cashu Pixel is a demo piece for Bitcoin and Cashu meetups and conferences. Two people share one device:

- **The owner** knows Cashu, carries the device, and starts the demo.
- **Visitors** are strangers with mixed familiarity with ecash. They watch over the owner's shoulder, then take the device and tap through it themselves. They receive sats by scanning the device's QR with their own phone wallet.

The job is to see and feel ecash working as a physical object in a few minutes of hand-to-hand use.

## Product Purpose

A dedicated hardware Cashu ecash wallet that shows in person what bearer cash on a device feels like. It holds a balance, mints ecash by showing a Lightning invoice, and hands ecash to someone else as a QR code.

## Positioning

A phone wallet is one app among many. Cashu Pixel is a single-purpose object that only holds and moves sats, with four things it stands for together:

- **Offline bearer cash:** sending produces an ecash token as a QR, with no network on the sending side.
- **Device-to-device over Bluetooth:** BLE transfer is a planned core feature (see Capabilities: today it is a placeholder).
- **A dedicated object:** no phone, no apps, no notifications.
- **An open hardware showcase:** a reference build of Cashu (CDK) running on a Raspberry Pi Zero 2 W.

## Operating Context

- Used standing or at a table at events, passed between owner and visitors, often with several people looking at the screen at once.
- The receiving side is the visitor's own Cashu phone wallet scanning the on-screen QR.
- Minting: the device shows a Lightning invoice QR, someone pays it from a Lightning wallet, then the operator taps CHECK PAYMENT to mint.
- Minting needs WiFi. Sending does not.
- The device runs as a kiosk: the Slint app draws straight to the display (linuxkms) with no desktop behind it.

## Capabilities and Constraints

**Hardware (binding, the only target):** Raspberry Pi Zero 2 W (about 416 MB usable RAM) with a Pimoroni HyperPixel 4.0 Square, a 720×720 capacitive touchscreen at about 254 PPI. Touch is the only input. The current build uses no keyboard, camera, or sound.

**Software:** Rust, Slint 1.15 (linuxkms backend, software renderer; no GPU rendering), CDK 0.15 wallet with a local SQLite store and a locally generated seed. The window is fixed at 720×720 with no frame. RAM is the hard ceiling: a browser-based frontend was tried and reverted for RAM reasons. Every animation costs CPU under software rendering.

**Mint (binding):** a single hardcoded mint, Minibits (`https://mint.minibits.cash/Bitcoin`), is the intended model, not a stopgap before multi-mint.

**Current capabilities, from `ui/app.slint` and `src/main.rs`:**

- Home: sat balance, mint host, and SEND, RECEIVE, SETTINGS.
- Receive: mint via Lightning with preset amounts (21, 100, 500, 1K, 5K, 10K sat), an invoice QR, and a manual payment check.
- Send: preset amounts (10, 21, 50, 100, 500, 1K sat) produce an ecash token QR. A token too large for a QR falls back to a truncated text message.
- Settings: read-only balance, mint URL, and version string.
- BLE receive: UI only. The screen says "SCANNING FOR BLUETOOTH" but nothing scans yet.

**Not implemented:** receiving an ecash token (no scan or paste path), custom amounts, restore or backup of the seed.

**Terminology:** sat / SAT, mint, ecash, token, Lightning invoice.

**Undecided:** the status of the React frontend in `web/` (built on `@mdrbx/nerv-ui`, served by the earlier CDK HTTP API). It is neither confirmed retired nor active.

## Brand Commitments

- **NERV identity is binding.** The NERV / Evangelion terminal framing is part of the product, not an experiment:
  - the on-screen name "NERV // CASHU TERMINAL"
  - the voice: uppercase, `//` separators, system-status language such as "SYSTEM READY" and "QR // DATA OUTPUT"
  - the look

  The incumbent look lives in `ui/app.slint` and is not yet recorded in a DESIGN.md.
- **Product name:** Cashu Pixel (crate name, version string "CASHU PIXEL V0.2.0").

## Evidence on Hand

- A working device running the current build, plus source in `ui/app.slint` and `src/main.rs`.
- A React prototype in `web/`.
- None of the following exist, and future work must not fabricate them: committed screenshots or visual goldens, photos of the device, event names, usage numbers, endorsements, or testimonials.

## Product Principles

1. **Legible to a stranger in hand.** Visitors take over the device mid-demo, so every screen must explain itself without the owner narrating.
2. **Readable over a shoulder.** Several people watch one small screen at once. Balance, amount, and QR are the show.
3. **The QR is the handoff.** The moment ecash leaves the device is the heart of the demo. It must scan first time on an ordinary phone wallet.
4. **Honest about what's real.** A showcase loses trust the moment it claims something it isn't doing, so planned features (BLE) must never read as live.
5. **Fit the hardware.** Design within 416 MB of RAM, software rendering, and a 720×720 touch surface, never around them.

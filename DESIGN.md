---
name: Cashu Pixel
description: A NERV command-monitor interface for a Cashu ecash wallet, laid out in millimetres for the 720×720 HyperPixel 4.0 Square.
colors:
  nerv-orange: "#FF9900"
  phosphor-green: "#00FF00"
  alarm-red: "#FF2B1D"
  lightning-amber: "#FFAA00"
  pattern-blue: "#00F6FF"
  void-black: "#000000"
  console-black: "#0A0A0A"
  standby-gray: "#555555"
  standby-text: "#838383"
typography:
  display:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "112px"
    fontWeight: 700
  headline:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "64px"
    fontWeight: 700
  keypad:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "44px"
    fontWeight: 700
  channel:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "40px"
    fontWeight: 700
    letterSpacing: "6px"
  action:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "28px"
    fontWeight: 700
    letterSpacing: "2px"
  title:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "24px"
    fontWeight: 700
  caption:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "18px"
    fontWeight: 700
    letterSpacing: "2px"
  meta:
    fontFamily: "DejaVu Sans, sans-serif"
    fontSize: "18px"
    fontWeight: 400
rounded:
  none: "0px"
spacing:
  keypad-gap: "12px"
  stack-gap: "16px"
  section-gap: "20px"
  gutter: "24px"
components:
  channel-key-send:
    backgroundColor: "#FF2B1D0D"
    textColor: "{colors.alarm-red}"
    typography: "{typography.channel}"
    rounded: "{rounded.none}"
    width: "328px"
  channel-key-receive:
    backgroundColor: "#00FF000D"
    textColor: "{colors.phosphor-green}"
    typography: "{typography.channel}"
    rounded: "{rounded.none}"
    width: "328px"
  button-action:
    backgroundColor: "#FFAA000D"
    textColor: "{colors.lightning-amber}"
    typography: "{typography.action}"
    rounded: "{rounded.none}"
    height: "112px"
  button-action-disabled:
    backgroundColor: "{colors.void-black}"
    textColor: "{colors.standby-text}"
    typography: "{typography.action}"
    rounded: "{rounded.none}"
    height: "112px"
  button-standby:
    backgroundColor: "#8383830D"
    textColor: "{colors.standby-text}"
    typography: "{typography.action}"
    rounded: "{rounded.none}"
    height: "100px"
  amount-key:
    backgroundColor: "{colors.void-black}"
    textColor: "{colors.lightning-amber}"
    typography: "{typography.keypad}"
    rounded: "{rounded.none}"
    width: "216px"
    height: "96px"
  amount-key-selected:
    backgroundColor: "#FFAA0024"
    textColor: "{colors.lightning-amber}"
    typography: "{typography.keypad}"
    rounded: "{rounded.none}"
    width: "216px"
    height: "96px"
  header:
    backgroundColor: "{colors.console-black}"
    textColor: "{colors.nerv-orange}"
    typography: "{typography.title}"
    height: "72px"
  readout:
    backgroundColor: "{colors.void-black}"
    textColor: "{colors.phosphor-green}"
    typography: "{typography.headline}"
    height: "112px"
  status-bar:
    backgroundColor: "{colors.console-black}"
    textColor: "{colors.standby-text}"
    typography: "{typography.caption}"
    height: "44px"
---

# Design System: Cashu Pixel

## Overview

**Creative North Star: "The MAGI Readout"**

The wallet is a monitor in NERV's command centre: a black glass field on which the system reports state. Nothing here decorates for its own sake. Frames, bracket corners, and scanlines exist to make the screen read as an instrument, and every number on it (balance, amount, invoice) reads as a live readout. The mood is **urgent, precise, and cinematic**: a live operation, where every value matters and every action is announced.

The system runs quiet structure with loud channels. Structure sits back in low-strength hairlines, while channels (send, receive, Lightning, Bluetooth) carry saturated alarm colour. Controls feel like an **alarm panel**: loud, colour-coded, unmistakable, and big enough for a stranger's thumb. At rest the screen is flat. Under a finger, or when selected, a control lights up like an indicator on a console.

The system is designed natively for one physical object: the 72 × 72 mm HyperPixel 4.0 Square at about 254 PPI. **Every size is chosen in millimetres (10px ≈ 1mm)**, never in desktop pixels. It has one column, no scrolling, and touch only. It must never read as a **generic crypto dashboard** (gradients, glassy cards, purple-blue fintech sheen, coin logos) or be assembled from **stock OS widgets** (Material, Fluent, or Cupertino controls).

**Key Characteristics:**
- The whole square is used. Each screen is one full-height stack: header, readout, controls, action, status.
- One saturated channel colour per function; orange is the system's own voice.
- Uppercase terminal type, bold almost everywhere, with `//` as the separator.
- Square corners, 1px hairlines, bracketed corners, stepped pixel icons, scanlines.
- Flat at rest, lit on touch: glow is built from frames and tints, never from shadows.

## Colors

A black field under saturated signal colours, with each colour bound to one meaning.

### Primary
- **NERV Orange** (#FF9900): the system's own voice. Home and Settings headers, settings rows, the QR frame and its brackets, the idle status accent, and the scanlines. When nothing else claims a surface, it is orange.

### Secondary
- **Phosphor Green** (#00FF00): value. The balance, every selected-amount readout (on the red Send screen too), the RECEIVE channel, CHECK PAYMENT, and success states.

### Tertiary
- **Alarm Red** (#FF2B1D): the SEND channel. Its home key, header bar, keypad, and CREATE TOKEN QR.
- **Lightning Amber** (#FFAA00): the Lightning mint channel. The Receive keypad and readout frame, CREATE LIGHTNING INVOICE, and the invoice QR header.
- **Pattern Blue** (#00F6FF): Bluetooth, and only Bluetooth. The scan banner in its active state.

### Neutral
- **Void Black** (#000000): the field. Every screen background, key fill, and panel fill.
- **Console Black** (#0A0A0A): chrome. Header bar, status bar, and scan banner. The only tonal step in the system.
- **Standby Gray** (#555555): inactive marks only. Disabled frames and brackets, and the idle status indicator. It is never used for text: at 2.8:1 on black it is unreadable.
- **Standby Text** (#838383): inactive and secondary text at 5.2:1. The idle "SYSTEM READY", placeholders, disabled labels, header subtitles, the mint host, the SETTINGS header action, and DONE.

### Named Rules
**The Channel Rule.** Every function owns exactly one colour, and nothing else wears it: SEND is red, RECEIVE and value are green, Lightning is amber, Bluetooth is blue, and the system is orange. A screen takes its channel's colour in its header bar, its keypad, and its primary action.

**The Green Means Sats Rule.** Phosphor Green is reserved for sat amounts, receiving, and success. An amount is green even on the red Send screen.

**The Strength Ladder Rule.** Colours sit on black at stepped strengths, never as blended mid-tones:
- text at 85–100%
- control frames at 60–75% (100% when lit)
- structural hairlines at 12–45%
- tints at 5% at rest, 12–25% when lit or pressed, and 6% for scanlines

## Typography

**Display Font:** DejaVu Sans Bold (with the system sans-serif)
**Body Font:** DejaVu Sans (with the system sans-serif)

**Character:** a plain, sturdy humanist sans, pushed into terminal voice by weight, case, and tracking rather than by a special face. The `.slint` source sets no font family, so this face is the Pi's fontconfig default. Changing the OS image can change the look.

### Hierarchy
- **Display** (700, 112px ≈ 11mm; 88px when the balance has 6+ digits): the balance on Home. The largest thing on any screen.
- **Headline** (700, 64px): the selected-amount readout on Receive and Send. Settings uses a 72px variant for the balance.
- **Keypad** (700, 44px): amount keys (21, 100, 1K…).
- **Channel** (700, 40px, 6px tracking): SEND and RECEIVE on the Home keys.
- **Action** (700, 28px, 2px tracking): primary action labels (CREATE LIGHTNING INVOICE, CHECK PAYMENT, DONE).
- **Title** (700, 24px): screen header titles ("RECEIVE // INCOMING").
- **Caption** (700, 18px, 2px tracking): readout captions, status bar text, banner text, and header subtitles.
- **Meta** (400, 18–20px): hints under the Home keys (18px) and the mint host (20px).

Units ("SAT") sit on the number's baseline. Because the two sizes differ, the unit is lifted by 0.236 × (number size − unit size), DejaVu's descent.

### Named Rules
**The Millimetre Rule.** Size everything physically: 10px ≈ 1mm on this panel. A desktop-sized pixel value is roughly 60% too small here.

**The 18px Floor Rule.** No text is smaller than 18px (about 1.8mm). Primary labels are 24px or larger.

**The Terminal Voice Rule.** Interface text is uppercase, and `//` separates a subject from its qualifier ("SEND // TOKEN", "INVOICE // LIGHTNING"). Hostnames and URLs keep their real case ("mint.minibits.cash").

## Layout

A fixed 720×720 canvas with no scrolling; the HyperPixel 4.0 Square is the only screen. Every screen is one full-height vertical stack:
- a 72px header bar
- content with 24px gutters and 16px stack gaps
- a 44px status bar pinned to the bottom (the QR screen puts its status line under the code instead)

The controls stretch to fill whatever the fixed parts leave, so no screen ends in empty space.

- **Home:** a 236px balance readout, then SEND and RECEIVE as two equal keys, each 328px wide, filling the rest (about 33 × 30 mm). Settings lives in the header, as a 176px action on the right.
- **Receive and Send:** a 112px amount readout, then a 3×2 keypad of 216px-wide keys that stretch vertically (114px on Receive, 148px on Send), then a 112px full-width action. Receive adds the 52px Bluetooth banner at the bottom.
- **QR:** a 456px framed code, centred, with a 40px status line under it. Below that, CHECK PAYMENT (456px) and DONE (200px), both 100px tall; DONE goes full width when there is no payment to check.
- **Settings:** three info rows that share the height at 1.5 : 1 : 1.

Touch targets are 80px (8mm) or larger everywhere; the smallest is the 104 × 72px header back key.

## Elevation & Depth

Flat at rest. Depth comes from one tonal step (Void Black field, Console Black chrome) and from hairline frames, never from shadows. Pressed and selected controls light up like phosphor indicators. That glow must be **constructed**: Slint 1.15's software renderer, the only renderer the Pi uses, silently ignores `drop-shadow-*`. Every lit state in the system is built the same way:
- the tint fill steps up (5% at rest, 12–14% selected, 20–25% pressed)
- the frame goes to full strength and thickens (1px to 2–3px)
- a second 1px frame appears 5–6px inside at 35–40%

The balance panel keeps a permanent inner frame at 12%, its resting glow.

### Named Rules
**The Built Glow Rule.** Never use `drop-shadow-*`; it renders nothing here. Light is made of frames and tints.

**The Lit-On-Active Rule.** Glow means "active, selected, or under a finger now". A resting element never glows, and a glowing element is always something the user can act on or is currently acting on.

## Shapes

Square everything: zero corner radius on every surface and control. Every geometric motif is square or right-angled:
- 1px hairline frames and rules
- bracket corners marking lead elements, at three scales: 12 × 2px on amount keys, and 20 × 3px on the Home keys, the balance panel, and the QR frame
- 6px channel bars on the left edge of headers, buttons, readouts, and the status bar
- **stepped pixel icons**: triangles built from stacked rectangles (▲ for SEND, ▼ for RECEIVE), and a stepped head plus a shaft for the back arrow
- small solid squares (8–10px) as indicators

There are no circles and no text glyphs standing in for icons.

## Components

Character: **alarm-panel channels on a quiet instrument frame.** Channel controls are loud, saturated, and finger-sized. Structural frames stay in the hairline band.

### Channel Key (Home)
- **Size:** each of the two keys is 328px wide and fills the height below the balance (about 300px).
- **Rest:** 5% channel tint, a 1px frame at 75%, and 20 × 3px full-strength bracket corners.
- **Content:** a stepped pixel triangle (6 steps of 7px), the Channel label, and a one-line Meta hint saying what the key does ("ECASH TOKEN AS QR", "MINT VIA LIGHTNING").
- **Pressed:** 20% tint, a 3px full-strength frame, and an inner frame at 40%.

### Action Button
- **Shape:** full width (672px), 104–112px tall, square, with a 6px left channel bar at 45%.
- **Rest:** 5% channel tint, 75% frame, and an Action label in the full channel colour. An optional Meta hint can sit under the label.
- **Pressed:** 20% tint, a 2px full-strength frame, a solid channel bar, and an inner frame at 35%.
- **Disabled:** a Void Black fill, a Standby Gray frame at 60%, and a Standby Text label. It still accepts a tap, so it can explain itself in the status bar ("SELECT AN AMOUNT FIRST").
- **Standby variant:** Standby Text as the channel colour, for secondary actions (DONE).

### Amount Key
- **Shape:** 216px wide, at least 96px tall (it stretches to fill the keypad), square, with 12 × 2px bracket corners.
- **Unselected:** a 60% frame, brackets at 85%, and the label at 90% in the screen's channel colour.
- **Selected:** 14% tint, a 2px full-strength frame, an inner frame at 35%, and the label at full strength. Pressed raises the tint to 25%.
- **Disabled:** Standby Gray frame and brackets with a Standby Text label, used for amounts above the balance on Send. A tap explains why ("NOT ENOUGH SATS // BALANCE 22 SAT").

### Header Bar
- 72px Console Black with a 6px full-strength channel bar and a 2px bottom rule at 45%.
- An optional 104px back key (stepped pixel arrow) with a hairline divider.
- The title in Title type in the channel colour, with an optional Caption subtitle in Standby Text on the right, or a 176px text action (SETTINGS).
- Pressed keys in the header fill with a 20% tint.

### Readout
- **Balance panel:**
  - 236px tall, with a green frame at 35%, a resting inner frame at 12%, and 20 × 3px green brackets
  - an orange Caption "BALANCE" at 8px tracking, over the Display balance with a baseline-aligned 40px "SAT", over the mint host in Meta
- **Amount readout:** a 112px black cell with a 45% channel frame and a 6px channel bar.
  - A Caption at the top-left ("MINT AMOUNT", "SEND AMOUNT") and an optional Caption aside at the top-right ("AVAILABLE 22 SAT").
  - Below them, "SELECT AN AMOUNT BELOW" in 24px Standby Text, or the Headline amount with a baseline-aligned 28px "SAT".

### Status Bar
- 44px Console Black with a 1px top rule at 30% and a 6px channel bar at 50%.
- An 8px square indicator, explicitly centred.
- A single line of Caption text that shortens with an ellipsis. Idle shows "SYSTEM READY" in Standby Text; live status shows in the channel colour.

### Scan Banner (signature)
- The Bluetooth channel's banner: 52px Console Black, with hazard bars at both ends (8px and 4px).
- A Caption label between two 10px indicators.
- Active: Pattern Blue. Inactive (current, since Bluetooth receive isn't built): Standby Gray marks with a Standby Text label, "BLUETOOTH RECEIVE // IN DEVELOPMENT".

### QR Frame (signature)
- A 456px black square with a 2px orange frame at 45% and 20 × 3px full-strength brackets.
- The code fills it with pixelated (nearest-neighbour) scaling so modules stay crisp.
- The code itself is inverted: light modules on black.

## Do's and Don'ts

### Do:
- **Do** size in millimetres: touch targets 80px (8mm) or larger, primary actions 100px or larger, text 18px or larger.
- **Do** fill the square. Let keypads and keys stretch to absorb leftover height instead of leaving empty space below the content.
- **Do** give each screen exactly one channel colour, shown in its header bar, keypad, and primary action.
- **Do** keep Phosphor Green for sats, receiving, and success only.
- **Do** give every control a pressed state and, where it can be unavailable, a disabled state that still explains itself when tapped.
- **Do** build lit states from stacked 1px frames and a stepped tint (see Elevation & Depth).
- **Do** position decorative squares explicitly, or wrap them in a centred `VerticalLayout`. Inside a `HorizontalLayout`, a fixed-size rectangle sits at the top, not the middle.
- **Do** write interface text in uppercase with `//` separators, leaving hostnames in their real case.

### Don't:
- **Don't** carry over desktop pixel sizes. 12px text and 44px targets are about 1.2mm and 4.4mm on this panel.
- **Don't** use `drop-shadow-*`; the software renderer draws nothing.
- **Don't** round a corner, introduce a circle, or use a text glyph (◄, ▶, ✓) as an icon. Draw stepped pixel icons from rectangles.
- **Don't** import `std-widgets.slint` or any Material, Fluent, or Cupertino control.
- **Don't** use gradients, glass or blur, purple-blue fintech colour, or coin logos.
- **Don't** let Pattern Blue stand for anything but Bluetooth, or any channel colour stand for a different function.
- **Don't** set text in Standby Gray (#555555); use Standby Text (#838383).
- **Don't** name a Slint component property `color` on anything that inherits `Rectangle`; it collides with a deprecated built-in and fails the build. Use `tint`.

# Audio-Reactive Ring Visualizer — "Audiowave Orb" rice skin

**Status: IMPLEMENTED** (2026-07-14). Shipped as a selectable RICE profile in the HUD.
This doc + `audio-reactive-visualizer.html` are the design history; the live code is in `app/`.

## What shipped
- **RICE profiles** — right-click the HUD (or the tray) → **RICE** submenu (radio): **Classic** (the usage card) and **Audiowave Orb**. Stored in `UiPrefs.rice` (ui.json). Right-clicking the HUD now pops the FULL settings menu (previously Opacity-only).
- **Audiowave Orb skin** — circular **54-bar** ring (style 12 "double") wrapped around center **layout D** (5H/7D usage + reset countdown). Reacts to real system audio; 160×160 square window (IR=52, MAXLEN=22 → bar tip ≈ 92% of radius, like the mockup).
- **Audio pipeline** — WASAPI **loopback** (default render endpoint, shared) → Hann **2048-FFT** → **36** log bands (30 Hz–16 kHz), **peak** magnitude per band (not average), GAIN 8 → `audio-spectrum` Tauri event ~33 Hz → orb engine's `spec[]`. Capture runs only while the orb skin is active.

## Locked decisions (all resolved 2026-07-14)
- Bar style = **12 "double"** (54 thin bars). Engine = **v3 anti-sticky**, **Spring = Jitter = 100** (1.0).
- Color yellow-green `hsl(60 + fold(a)*110, 82%, 56%)`. Center layout **D**.
- Integration = **separate selectable rice skin** (not halo / not replace-card).
- Loopback captures the full system mix (all apps); can't isolate a single app.
- **Reactivity is audio-only** — the mockup's `+0.03` target floor is dropped and the jitter is scaled by each bar's envelope (`jj *= b.env`), so a silent ring is perfectly still; the fold takes the **peak** bin per band (average flattened it). GAIN 8 calibrated against a real loopback probe. Do not revert these.

## Code
- `app/src-tauri/src/audio.rs` — WASAPI loopback + FFT + emit (wasapi 0.23, rustfft 6).
- `app/src-tauri/src/ui_prefs.rs` — `rice` field.
- `app/src-tauri/src/main.rs` — RICE submenu, full-menu-on-right-click, profile switch + resize + audio start/stop, startup activation.
- `app/ui/orb.js` — orb engine (port of style 12) fed by `audio-spectrum` + center D from `state-updated`.
- `app/ui/index.html`, `style.css`, `main.js` — `#orb` skin + `body[data-rice]` switch.

## Cost realized
Binary +~1.3 MB (wasapi + rustfft + windows). CPU ≈ 0 % in Classic (no capture); orb ~2–6 % of one core when visible (WebView2 rAF render + FFT).

## Open / could tune later
- `GAIN` (now 8; audio sensitivity), `NOISE_FLOOR`, and band range in `audio.rs`; emit rate.
- Idle silence now reads flat: WASAPI keeps delivering zero-frames, so each bar's envelope decays to the 0.04 floor and the ring goes still. Only a fully *parked* device (no events at all) would freeze the last frame — could emit zeros after a long event timeout if that ever shows up.
- More rice profiles are cheap: add to `RICE_PROFILES` in `main.rs` + a matching skin.

## Reference
`audio-reactive-visualizer.html` — the 20-style mockup board (fake spectrum) that drove the design.

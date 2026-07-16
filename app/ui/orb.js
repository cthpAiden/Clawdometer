// Audiowave Orb skin. A circular 54-bar ring reacting to real system audio
// (Rust WASAPI loopback -> FFT -> "audio-spectrum" event), wrapped around a
// center usage disc fed by "state-updated" (the same snapshot the card uses).
//
// Three style variants share this ring + center disc:
//  - "bars": ported from the style-12 "double" mockup, v3 anti-sticky engine
//    (per-bar envelope follower + 3 scattered bins + osc spread + spring
//    bounce) at spring = jitter = 1.0.
//  - "peak": "bars" plus a floating peak-hold cap per bar.
//  - "led": ported from docs/mockups/audio-orb-led-usage-zones.html — 5-rung
//    stacked LED segments per bar, colored from the live usage zone (not a
//    fixed palette), fed by 6 independent per-region (bass->treble) onset
//    detectors that each bloom a 5-bar kernel inside their own arc.
// Geometry is scaleY/opacity-only inside a rotated spoke, so bars stay crisp
// radial lines (never ovals) — the transform/opacity-only rule the
// transparent WebView2 window forces. Exposes window.Orb.{start,stop,setStyle};
// main.js runs the loop only while this is the active skin.
(function () {
  if (!window.__TAURI__) return;

  const N = 36;        // spectrum bins — matches the Rust emitter's band count
  const BARS = 54;     // style 12 "double": dense thin bars
  const IR = 52;       // inner radius (px) — bar roots sit here, around the disc
  const MAXLEN = 22;   // bar length at frac 1.0 (must match #orb .bar height)
  const SPRING = 1.0;  // 100/100 — chosen settings
  const JIT = 1.0;
  const RESPONSE = 1.82; // overall reactivity boost: bars jump stronger,
                         // independent of the 100/100 spring/jitter feel.
  const BOUNCE = 1.56;  // spring-overshoot boost: scales the restoring force so
                        // bars overshoot more (past the tgt=1 ceiling, up to the
                        // frac 1.25 clamp) — the real "stronger" knob once
                        // RESPONSE saturates.
  const HOLD = 26;      // frames a peak cap freezes at a new max before it falls
  const FALL = 0.014;   // per-frame drop of the held cap (in frac units)

  // ---- "led" variant constants, ported from
  // docs/mockups/audio-orb-led-usage-zones.html ----
  const SEGMENTS = 5;    // LED rungs per bar
  const SEGSTEP = 5;     // px between rung bottoms
  const KICK_DECAY = 0.85;
  const KERNEL = [0.30, 0.62, 1.0, 0.62, 0.30]; // 5-bar bloom lobe, center highest
  const KERNEL_SUM = KERNEL.reduce((a, c) => a + c, 0);
  // Light spatial blend of the same bloom kernel into the continuous band
  // read (not just bloom moments), so a bar's height is nudged toward its
  // neighbors' average -- rounds off raw per-bar spikes into more of a lobe
  // shape. 0 = fully raw/per-band accurate, 1 = fully the kernel-weighted
  // neighbor average. User picked "Light" over "Even"/"Full" to keep most
  // of the per-band realism.
  const LED_BAND_BLEND = 0.25;
  const ledRawBand = new Float32Array(BARS);
  const REGIONS = 6;
  const REGION_BARS = BARS / REGIONS; // 9 bars per region
  const REGION_BINS = N / REGIONS;    // 6 bins per region
  // "Bouncy" preset from the mockup's 6-preset comparison board. Reverted
  // back here after a brief detour to "Snappy" didn't land with the user.
  // Band weight bumped below (frameLed's tgt calc, 0.55 -> 0.65) per request
  // for the ring to swing a bit higher/taller on ordinary signal, not just
  // on blooms.
  // Attack/release/k/decay matched to Bars/Peak hold's per-bar average
  // (frameBars: attack mean 0.45, release-used mean 0.4525, k*force-gain
  // mean 0.696 @ fixed LED_BOUNCE=1.9 -> k 0.37, decay-used mean 0.76) so
  // LED Bloom rises and falls just as fast as the other two variants.
  const LED_ATTACK = 0.45, LED_RELEASE = 0.45, LED_K = 0.37, LED_DECAY = 0.76,
    LED_BOUNCE = 1.9, LED_RND_WEIGHT = 0.12;
  // Right half of the ring (bar i < BARS/2) sits on bins 0-18, bass/low-mid --
  // genuinely louder/more dynamic in most audio than the left half's bins
  // 18-36 (mid/treble), so it visibly jumps more. Per-half gain on the
  // positional band read, user's explicit ask after confirming coverage was
  // already complete (all 36 bins used, no missing band).
  const LED_LEFT_GAIN = 1.80, LED_RIGHT_GAIN = 1.20;
  // 5-stop ramps per usage zone (locked 2026-07-16). Bottom 4 stops are
  // usage-color.js's bar/lit shades pulled through the ramp; the 5th (tip) is
  // the peak accent picked with the user: blue->cyan, yellow->deep orange,
  // red->gold/amber. Zone thresholds themselves stay owned by usage-color.js
  // (window.UsageColor.level) — only these shade stops are LED-specific.
  const LED_RAMPS = {
    ok: ["#2d5d8f", "#3f7fbf", "#5b98d4", "#7cc3e6", "#67e8f9"],
    warn: ["#a8670a", "#f59e0b", "#ffb733", "#ffd580", "#f97316"],
    crit: ["#a8353a", "#e5484d", "#f26a6e", "#f8a3a5", "#f5a623"],
  };

  const fold = (x) => { x %= 1; if (x < 0) x += 1; return x < 0.5 ? x * 2 : (1 - x) * 2; };
  const col = (a) => `hsl(${(60 + fold(a) * 110).toFixed(0)},82%,56%)`;

  // Latest real spectrum (0..1), written by the audio-spectrum listener and
  // read every frame. Starts silent, so an idle ring rests at its floor.
  // Shared by all three variants — "led" reads it positionally (b.i/BARS) and
  // per-region, same array "bars"/"peak" read via the scattered bins.
  const spec = new Float32Array(N);
  const lerpBin = (fr) => {
    const x = fr * N, i = Math.floor(x);
    const a = spec[((i % N) + N) % N], b = spec[(((i + 1) % N) + N) % N];
    return a + (b - a) * (x - i);
  };

  // Usage colors come from the shared table so the orb and the card can't drift.
  const { paint, num: numColor, level: usageLevel } = window.UsageColor;

  let orbEl = null, bars = [], co = null, running = false, raf = 0, t = 0;
  let lastPayload = null;
  let styleMode = "bars"; // "bars" | "peak" | "led"

  // "led"-only region-onset state, (re)allocated in build() when entering led.
  let regionFast = null, regionAvg = null, regionCooldown = null;
  let ledZoneKey = null; // last zone actually painted, so recolor is a one-time write

  function build() {
    orbEl = document.getElementById("orb");
    if (!orbEl) return;
    const ring = orbEl.querySelector(".ring");
    ring.textContent = "";
    bars = [];
    const led = styleMode === "led";
    for (let i = 0; i < BARS; i++) {
      const ang = (i / BARS) * 360;
      const spoke = document.createElement("div");
      spoke.className = "spoke";
      spoke.style.transform = `rotate(${ang}deg)`;
      if (led) {
        const segs = [];
        for (let s = 0; s < SEGMENTS; s++) {
          const seg = document.createElement("div");
          seg.className = "seg";
          seg.style.bottom = (IR + s * SEGSTEP) + "px";
          seg.style.opacity = ".12";
          spoke.appendChild(seg);
          segs.push(seg);
        }
        ring.appendChild(spoke);
        bars.push({ i: i, seg: segs, env: 0, y: 0, v: 0, kick: 0, rnd: 0, lastLit: 0 });
      } else {
        const d = document.createElement("div");
        d.className = "bar";
        d.style.setProperty("--ir", IR + "px");
        spoke.appendChild(d);
        // Peak-hold cap: a floating marker that snaps to this bar's peak, holds,
        // then falls. Pure visual overlay — the bar physics below are untouched.
        const cap = document.createElement("div");
        cap.className = "cap";
        cap.style.setProperty("--ir", IR + "px");
        spoke.appendChild(cap);
        ring.appendChild(spoke);
        // Per-bar v3 state: three scattered bins (one drifting +t, one static,
        // one drifting -t) plus its own envelope attack/release, spring constant
        // and decay — this is what stops neighbors moving as one wave.
        bars.push({
          el: d, cap: cap, peak: 0.04, holdT: 0, y: 0, v: 0, env: 0,
          binA: ((i * 13) % N) / N, binB: ((i * 29 + 7) % N) / N, binC: ((i * 7 + 3) % N) / N,
          phase: i * 2.399, oscf: 3.5 + (i % 9) * 0.85,
          attack: 0.30 + (i % 6) * 0.06, release: 0.18 + (i % 8) * 0.035,
          decay: 0.74 + (i % 9) * 0.02, k: 0.14 + (i % 7) * 0.018, i: i,
        });
      }
    }
    if (led) {
      regionFast = new Float32Array(REGIONS);
      regionAvg = new Float32Array(REGIONS).fill(0.05);
      regionCooldown = new Int32Array(REGIONS);
      ledZoneKey = null; // force a repaint of the freshly built segs
      if (lastPayload) applyLedColor(lastPayload);
    }
    co = {
      n5: orbEl.querySelector("#o5h"), b5: orbEl.querySelector("#ob5"),
      n7: orbEl.querySelector("#o7d"), b7: orbEl.querySelector("#ob7"),
      reset: orbEl.querySelector("#oreset"),
    };
    orbEl.classList.toggle("peak", styleMode === "peak");
    if (lastPayload) renderStats(lastPayload);
  }

  // 6 independent onset detectors (bass->treble), one per contiguous
  // 6-bin/9-bar region: fast-vs-slow running average per region, cooldown so
  // one transient doesn't retrigger every frame while it decays.
  function detectRegionHits() {
    const fired = [];
    for (let r = 0; r < REGIONS; r++) {
      let e = 0; const b0 = r * REGION_BINS;
      for (let i = 0; i < REGION_BINS; i++) e += spec[b0 + i];
      e /= REGION_BINS;
      regionFast[r] += (e - regionFast[r]) * 0.55;
      regionAvg[r] += (e - regionAvg[r]) * 0.04;
      regionCooldown[r] = Math.max(0, regionCooldown[r] - 1);
      if (regionFast[r] > regionAvg[r] * 1.35 + 0.10 && regionCooldown[r] === 0) {
        regionCooldown[r] = 10;
        fired.push(r);
      }
    }
    return fired;
  }

  function frameLed() {
    for (const r of detectRegionHits()) {
      // Bloom center is random WITHIN the region that actually fired, not
      // anywhere on the ring — stays truthful to bar j's positional band.
      const center = r * REGION_BARS + Math.floor(Math.random() * REGION_BARS);
      for (let o = -2; o <= 2; o++) {
        const idx = ((center + o) % BARS + BARS) % BARS;
        bars[idx].kick = Math.max(bars[idx].kick, KERNEL[o + 2]);
      }
    }
    // Raw per-bar band, computed once so the blend pass below reads true
    // neighbor values instead of re-deriving lerpBin per read.
    for (let i = 0; i < BARS; i++) ledRawBand[i] = lerpBin(i / BARS);
    for (const b of bars) {
      let blended = 0;
      for (let o = -2; o <= 2; o++) {
        blended += ledRawBand[((b.i + o) % BARS + BARS) % BARS] * KERNEL[o + 2];
      }
      blended /= KERNEL_SUM;
      const halfGain = b.i < BARS / 2 ? LED_RIGHT_GAIN : LED_LEFT_GAIN;
      const band = (ledRawBand[b.i] * (1 - LED_BAND_BLEND) + blended * LED_BAND_BLEND) * halfGain;
      // Persistent random walk, independent of audio — organic variation on
      // top of the band signal instead of a per-frame dice roll.
      b.rnd += (Math.random() * 2 - 1) * 0.03;
      b.rnd = Math.max(-1, Math.min(1, b.rnd));
      b.rnd *= 0.97;
      b.kick *= KICK_DECAY;
      const tgt = Math.min(1.3, band * 0.65 + b.rnd * LED_RND_WEIGHT + b.kick);
      if (tgt > b.env) b.env += (tgt - b.env) * LED_ATTACK;
      else b.env += (tgt - b.env) * LED_RELEASE;
      const force = (b.env - b.y) * LED_K * LED_BOUNCE;
      b.v = (b.v + force) * LED_DECAY;
      b.y += b.v;
      if (b.y < 0) { b.y = 0; b.v *= -0.3; }
      const frac = Math.max(0, Math.min(1.3, b.y));
      // Floor of 1: the innermost rung stays lit at all times (the user's
      // "always on" ask) so the ring never goes fully dark between hits;
      // rungs 2-5 still ride the envelope above that floor.
      const lit = Math.max(1, Math.min(SEGMENTS, Math.round(frac * SEGMENTS)));
      // Delta-checked: only write opacity on the segments whose lit state
      // actually flipped, not all 5 every frame.
      if (lit !== b.lastLit) {
        if (lit > b.lastLit) for (let s = b.lastLit; s < lit; s++) b.seg[s].style.opacity = 1;
        else for (let s = lit; s < b.lastLit; s++) b.seg[s].style.opacity = .12;
        b.lastLit = lit;
      }
    }
  }

  // Recolor all 270 segments to the current usage zone's ramp — a one-time
  // write on zone change (guarded by ledZoneKey), never a per-frame cost.
  function applyLedColor(payload) {
    if (styleMode !== "led" || !bars.length) return;
    const st = payload && payload.state;
    const fh = st && st.rate_limits && st.rate_limits.five_hour;
    const p5 = fh && typeof fh.used_percentage === "number" ? fh.used_percentage : 0;
    const zoneKey = usageLevel(p5);
    if (zoneKey === ledZoneKey) return;
    ledZoneKey = zoneKey;
    const ramp = LED_RAMPS[zoneKey];
    for (const b of bars) for (let s = 0; s < SEGMENTS; s++) b.seg[s].style.background = ramp[s];
  }

  function frameBars() {
    for (const b of bars) {
      const a = lerpBin(fold(b.binA + t * 0.05));
      const c = lerpBin(b.binB);
      const dd = lerpBin(fold(b.binC - t * 0.03));
      const osc = Math.sin(b.phase + t * b.oscf) * 0.5 + 0.5;
      // No constant floor term: the mockup added +0.03 so its fake spectrum
      // always looked alive, but for real audio silence must read as silence.
      // Every term here is audio (a, c, dd); osc only modulates c's share.
      const tgt = Math.min(1, (a * 0.40 + c * 0.30 * osc + dd * 0.20) * RESPONSE);
      // Fast attack, own (faster, spring-boosted) release.
      if (tgt > b.env) b.env += (tgt - b.env) * b.attack;
      else b.env += (tgt - b.env) * (b.release + SPRING * 0.15);
      // Spring toward the envelope, with decay and a small floor bounce.
      const force = (b.env - b.y) * b.k * (1 + SPRING * 1.3) * BOUNCE;
      b.v = (b.v + force) * (b.decay - SPRING * 0.06);
      b.y += b.v;
      if (b.y < 0) { b.y = 0; b.v *= -0.3; }
      // Jitter is texture ON TOP of audio, scaled by this bar's envelope — so a
      // silent ring is dead still. The old ungated jitter wobbled idle bars,
      // which read as "not really reacting to the audio".
      const jj = JIT * 0.06 * Math.sin(t * (9 + b.i * 1.3) + b.phase) * b.env;
      const frac = Math.max(0.04, Math.min(1.25, b.y + jj));
      b.el.style.transform = `scaleY(${frac})`;
      b.el.style.opacity = (0.4 + frac * 0.55).toFixed(2);
      b.el.style.background = col(b.binA + frac * 0.35);
      // Peak-hold (only the "Peak hold" variant): snap up to a new max, freeze
      // HOLD frames, then fall by FALL. The "Bars" variant skips this and CSS
      // hides the caps, so the bar physics above are the whole show.
      if (styleMode === "peak") {
        if (frac >= b.peak) { b.peak = frac; b.holdT = HOLD; }
        else if (b.holdT > 0) { b.holdT--; }
        else { b.peak = Math.max(frac, b.peak - FALL); }
        b.cap.style.transform = `translateY(${(-b.peak * MAXLEN).toFixed(2)}px)`;
      }
    }
  }

  function frame() {
    t += 0.016;
    if (styleMode === "led") frameLed();
    else frameBars();
    if (running) raf = requestAnimationFrame(frame);
  }

  const clampPct = (v) => (typeof v === "number" ? Math.max(0, Math.min(100, v)) : null);

  function fmtReset(epochSec, nowMs) {
    if (!Number.isFinite(epochSec)) return "";
    const mins = Math.ceil((epochSec * 1000 - nowMs) / 60000);
    if (mins <= 0) return "reset";
    const core = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
    return `resets ${core}`;
  }

  function renderStats(payload) {
    if (!co) return;
    const st = payload && payload.state;
    const rl = st && st.rate_limits;
    const fh = rl && rl.five_hour;
    const sd = rl && rl.seven_day;
    const nowMs = Date.now();

    const p5 = fh && typeof fh.used_percentage === "number" ? fh.used_percentage : null;
    if (p5 === null) {
      co.n5.textContent = "—"; co.n5.style.color = ""; co.b5.style.width = "0%";
    } else {
      co.n5.textContent = p5 + "%";
      co.n5.style.color = numColor(p5);
      co.b5.style.width = clampPct(p5) + "%";
      paint(co.b5, p5);
    }

    const p7 = sd && typeof sd.used_percentage === "number" ? sd.used_percentage : null;
    if (p7 === null) {
      co.n7.textContent = "—"; co.b7.style.width = "0%";
    } else {
      co.n7.textContent = p7 + "%";
      co.b7.style.width = clampPct(p7) + "%";
      paint(co.b7, p7);
    }

    // Reset countdown for the 5h window; drop the line past its reset.
    co.reset.textContent = fh && fh.resets_at * 1000 > nowMs ? fmtReset(fh.resets_at, nowMs) : "";
  }

  window.Orb = {
    // Pick the ring's rendering style. "bars"/"peak" share one DOM shape (no
    // rebuild between them, same as before); "led" has a different per-bar
    // shape (5 seg divs, no .bar/.cap) so crossing that boundary rebuilds.
    setStyle(style) {
      const next = style === "peak" ? "peak" : style === "led" ? "led" : "bars";
      const needsRebuild = (next === "led") !== (styleMode === "led");
      styleMode = next;
      if (orbEl) orbEl.classList.toggle("peak", styleMode === "peak");
      if (needsRebuild) build();
      else if (styleMode === "led") applyLedColor(lastPayload);
    },
    start() {
      if (!orbEl) build();
      if (running) return;
      running = true;
      raf = requestAnimationFrame(frame);
    },
    stop() {
      running = false;
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
    },
  };

  // Real spectrum in (Vec<f32> length 36, values 0..1).
  window.__TAURI__.event.listen("audio-spectrum", (e) => {
    const arr = e.payload;
    if (Array.isArray(arr)) {
      const n = Math.min(N, arr.length);
      for (let i = 0; i < n; i++) spec[i] = arr[i];
    }
  }).catch(console.error);

  // Center usage stats from the same snapshot the card renders.
  window.__TAURI__.event.listen("state-updated", (e) => {
    lastPayload = e.payload;
    if (!orbEl) build();
    renderStats(lastPayload);
    applyLedColor(lastPayload);
  }).catch(console.error);

  // The reset countdown ticks locally between snapshots (minute granularity).
  setInterval(() => { if (lastPayload) renderStats(lastPayload); }, 30000);
})();

// Audiowave Orb skin. A circular 54-bar ring reacting to real system audio
// (Rust WASAPI loopback -> FFT -> "audio-spectrum" event), wrapped around a
// center usage disc fed by "state-updated" (the same snapshot the card uses).
//
// Ported from the style-12 "double" mockup with the v3 anti-sticky engine
// (per-bar envelope follower + 3 scattered bins + osc spread + spring bounce)
// at spring = jitter = 1.0. Geometry is scaleY-only inside a rotated spoke, so
// bars stay crisp radial lines (never ovals) — the transform/opacity-only rule
// the transparent WebView2 window forces. Exposes window.Orb.{start,stop};
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

  const fold = (x) => { x %= 1; if (x < 0) x += 1; return x < 0.5 ? x * 2 : (1 - x) * 2; };
  const col = (a) => `hsl(${(60 + fold(a) * 110).toFixed(0)},82%,56%)`;

  // Latest real spectrum (0..1), written by the audio-spectrum listener and
  // read every frame. Starts silent, so an idle ring rests at its floor.
  const spec = new Float32Array(N);
  const lerpBin = (fr) => {
    const x = fr * N, i = Math.floor(x);
    const a = spec[((i % N) + N) % N], b = spec[(((i + 1) % N) + N) % N];
    return a + (b - a) * (x - i);
  };

  // Usage colors: same thresholds as the card so a low number reads as safe.
  const barColor = (pct) => (pct >= 90 ? "#e5484d" : pct >= 70 ? "#f59e0b" : "#4a7c47");
  const numColor = (pct) => (pct >= 90 ? "#e5484d" : pct >= 70 ? "#f0b429" : "#63b35f");

  let orbEl = null, bars = [], co = null, running = false, raf = 0, t = 0;
  let lastPayload = null;
  let peakOn = false;  // "Peak hold" variant on? Off = plain bars (the "Bars" skin).

  function build() {
    orbEl = document.getElementById("orb");
    if (!orbEl) return;
    const ring = orbEl.querySelector(".ring");
    ring.textContent = "";
    bars = [];
    for (let i = 0; i < BARS; i++) {
      const ang = (i / BARS) * 360;
      const spoke = document.createElement("div");
      spoke.className = "spoke";
      spoke.style.transform = `rotate(${ang}deg)`;
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
    co = {
      n5: orbEl.querySelector("#o5h"), b5: orbEl.querySelector("#ob5"),
      n7: orbEl.querySelector("#o7d"), b7: orbEl.querySelector("#ob7"),
      reset: orbEl.querySelector("#oreset"),
    };
    if (orbEl) orbEl.classList.toggle("peak", peakOn);
    if (lastPayload) renderStats(lastPayload);
  }

  function frame() {
    t += 0.016;
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
      if (peakOn) {
        if (frac >= b.peak) { b.peak = frac; b.holdT = HOLD; }
        else if (b.holdT > 0) { b.holdT--; }
        else { b.peak = Math.max(frac, b.peak - FALL); }
        b.cap.style.transform = `translateY(${(-b.peak * MAXLEN).toFixed(2)}px)`;
      }
    }
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
      co.b5.style.background = barColor(p5);
    }

    const p7 = sd && typeof sd.used_percentage === "number" ? sd.used_percentage : null;
    if (p7 === null) {
      co.n7.textContent = "—"; co.b7.style.width = "0%";
    } else {
      co.n7.textContent = p7 + "%";
      co.b7.style.width = clampPct(p7) + "%";
      co.b7.style.background = barColor(p7);
    }

    // Reset countdown for the 5h window; drop the line past its reset.
    co.reset.textContent = fh && fh.resets_at * 1000 > nowMs ? fmtReset(fh.resets_at, nowMs) : "";
  }

  window.Orb = {
    // Toggle the peak-hold caps (main.js passes true only for the
    // "audiowave_orb_peak" skin). CSS shows/hides caps off the "peak" class.
    setPeak(on) {
      peakOn = !!on;
      if (orbEl) orbEl.classList.toggle("peak", peakOn);
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
  }).catch(console.error);

  // The reset countdown ticks locally between snapshots (minute granularity).
  setInterval(() => { if (lastPayload) renderStats(lastPayload); }, 30000);
})();

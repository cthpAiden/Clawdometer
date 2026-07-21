// Globals (drag / context menu / compact / opacity / working) + renderer for the
// Underline (U8, the "classic" default) and Rowline (U1, "rowline") cards, which
// share one payload. Contract: `state-updated` (usage data) and `ui-prefs`
// (opacity/compact/rice) events in; `ui-ready` and `toggle-compact` events out.

// If Tauri's bridge injection ever fails, show a hint instead of throwing
// mid-setup (an uncaught throw here would also kill render/setInterval below).
// No footer any more, so the hint rides the title of whichever skin is up.
if (!window.__TAURI__) {
  document.querySelectorAll(".ttl").forEach((t) => (t.textContent = "restart HUD"));
  throw new Error("__TAURI__ not injected");
}

// Every Tauri IPC call returns a promise; a rejection (e.g. a capability
// narrowed in a future build) should land in the console, not as a silent
// unhandledrejection in a hidden webview.
const logRejection = (p) => p.catch(console.error);

// The HUD is chrome, not a page — suppress WebView2's context menu
// (Back/Refresh/Save as/Print) and instead pop the native Opacity menu, so the
// panel itself is a right-click target for opacity (same items as the tray).
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  logRejection(window.__TAURI__.event.emit("hud-context"));
});

// The whole panel is the grab target. Drag anywhere to move; double-click
// anywhere to toggle compact size (same as the tray's "Compact size" item).
// Start the native OS drag only once the pointer moves past a few pixels, so a
// stationary double-click is never swallowed by the move loop and a plain click
// stays inert.
let dragOrigin = null;
window.addEventListener("mousedown", (e) => {
  if (e.button === 0) dragOrigin = { x: e.screenX, y: e.screenY };
});
window.addEventListener("mousemove", (e) => {
  if (!dragOrigin || !(e.buttons & 1)) { dragOrigin = null; return; }
  if (Math.abs(e.screenX - dragOrigin.x) > 4 || Math.abs(e.screenY - dragOrigin.y) > 4) {
    dragOrigin = null;
    logRejection(window.__TAURI__.window.getCurrentWindow().startDragging());
  }
});
window.addEventListener("mouseup", () => { dragOrigin = null; });
window.addEventListener("dblclick", (e) => {
  if (e.button === 0) logRejection(window.__TAURI__.event.emit("toggle-compact"));
});

const els = {
  card: document.getElementById("card"),
  countdown: document.getElementById("countdown"),
  bar5h: document.getElementById("bar5h"),
  bar7d: document.getElementById("bar7d"),
  barfb: document.getElementById("barfb"),
  txt5h: document.getElementById("txt5h"),
  txt7d: document.getElementById("txt7d"),
  txtfb: document.getElementById("txtfb"),
  // Underline (U8) — the "classic" default skin. renderRow paints its .uline <i>
  // exactly like a bar fill, so it shares this render path with the Rowline card.
  ureset: document.getElementById("ureset"),
  uln5h: document.getElementById("uln5h"),
  uln7d: document.getElementById("uln7d"),
  ulnfb: document.getElementById("ulnfb"),
  utxt5h: document.getElementById("utxt5h"),
  utxt7d: document.getElementById("utxt7d"),
  utxtfb: document.getElementById("utxtfb"),
};

let current = null; // last payload
let compactMode = false; // mirrors the tray's "Compact size" toggle

// Threshold colors live in usage-color.js — the orb skin reads the same table.
// Only the bar hue is threshold-driven here; the numbers stay off-white (the
// bars carry the color), so we take paint() but not num().
const { paint } = window.UsageColor;

// Reset countdown for the 5h window, shown top-right. Account-wide limits, so
// this beats a model name. Regular spells "Resets"; compact drops the word to
// fit. Past the reset the backend derives 0% (the window is gone until the next
// request opens one), so it reads a plain "reset".
function fmtCountdown(resetsAtEpochSec, nowMs, compact) {
  if (!Number.isFinite(resetsAtEpochSec)) return "—";
  const mins = Math.ceil((resetsAtEpochSec * 1000 - nowMs) / 60000);
  if (mins <= 0) return "reset";
  const core = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
  return compact ? core : `Resets ${core}`;
}

// A usage row: threshold-colored bar + off-white percentage. An absent window
// renders "—", never 0% — fable_week is missing until a /usage refresh has seen
// it, and missing entirely until Fable is used this week, neither of which
// means "none used".
function renderRow(win, bar, txt) {
  if (!win || typeof win.used_percentage !== "number") {
    txt.textContent = "—";
    bar.style.width = "0%";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  // textContent, never innerHTML: the state files are the input here, and a
  // template injection sink is one refactor away from XSS.
  txt.textContent = `${win.used_percentage}%`;
  bar.style.width = pct + "%";
  paint(bar, win.used_percentage);
}

// Clawd's working animation is picked per turn from a no-repeat shuffle bag:
// draw down the whole pool before reshuffling, and never let a reshuffle
// repeat the last-shown one back-to-back. The CSS keys off body[data-canim].
const CLAWD_ANIMS = ["coder", "a1", "a2", "a10", "hop", "shiver", "turn", "sway", "squat"];
let clawdBag = [];
let clawdCur = null;
let wasWorking = false;

function nextClawdAnim() {
  if (clawdBag.length === 0) {
    clawdBag = CLAWD_ANIMS.slice();
    for (let i = clawdBag.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [clawdBag[i], clawdBag[j]] = [clawdBag[j], clawdBag[i]];
    }
    if (clawdBag[0] === clawdCur && clawdBag.length > 1) {
      [clawdBag[0], clawdBag[1]] = [clawdBag[1], clawdBag[0]];
    }
  }
  clawdCur = clawdBag.shift();
  document.body.dataset.canim = clawdCur;
}

function render() {
  const nowMs = Date.now();
  // Animate the mascot while a Claude Code session is mid-turn; the backend
  // derives this flag from transcript turn state (see
  // watcher::any_session_generating) so idle live.json refreshes never trip it.
  const isWorking = !!(current && current.working);
  document.body.classList.toggle("working", isWorking);
  // New turn (idle -> working): advance to the next shuffled animation.
  if (isWorking && !wasWorking) nextClawdAnim();
  wasWorking = isWorking;
  const state = current && current.state;
  if (!state || !state.rate_limits) {
    els.countdown.textContent = "—";
    els.ureset.textContent = "—";
    renderRow(null, els.bar5h, els.txt5h);
    renderRow(null, els.bar7d, els.txt7d);
    renderRow(null, els.barfb, els.txtfb);
    renderRow(null, els.uln5h, els.utxt5h);
    renderRow(null, els.uln7d, els.utxt7d);
    renderRow(null, els.ulnfb, els.utxtfb);
    return;
  }
  const rl = state.rate_limits;
  const resetAt = rl.five_hour && rl.five_hour.resets_at;
  // One payload feeds both card skins; only the visible one shows (CSS), the
  // other's writes are cheap off-screen no-ops. Rowline (172px) drops the
  // "Resets" word so a long "2h 47m" never collides with the centred title;
  // Underline (200px) has room for the full phrasing.
  els.countdown.textContent = fmtCountdown(resetAt, nowMs, true);
  els.ureset.textContent = fmtCountdown(resetAt, nowMs, compactMode);
  renderRow(rl.five_hour, els.bar5h, els.txt5h);
  renderRow(rl.seven_day, els.bar7d, els.txt7d);
  renderRow(rl.fable_week, els.barfb, els.txtfb);
  renderRow(rl.five_hour, els.uln5h, els.utxt5h);
  renderRow(rl.seven_day, els.uln7d, els.utxt7d);
  renderRow(rl.fable_week, els.ulnfb, els.utxtfb);
}

logRejection(window.__TAURI__.event.listen("state-updated", (event) => {
  current = event.payload;
  render();
}));

logRejection(window.__TAURI__.event.listen("ui-prefs", (event) => {
  const p = event.payload || {};
  compactMode = !!p.compact;
  document.body.classList.toggle("compact", compactMode);
  const opacity = typeof p.opacity === "number" ? p.opacity : 1;
  els.card.style.opacity = opacity;
  for (const id of ["bento", "orb", "underline"]) {
    const el = document.getElementById(id);
    if (el) el.style.opacity = opacity;
  }
  // Switch skins: Classic card, Bento Box, or Audiowave Orb. The orb's
  // animation loop only runs while it's the active skin (the backend likewise
  // gates audio capture on it), so the other two cost nothing extra.
  const rice = typeof p.rice === "string" ? p.rice : "classic";
  document.body.dataset.rice = rice;
  if (window.Orb) {
    // Three orb variants share the "audiowave_orb" prefix: plain "Bars",
    // "audiowave_orb_peak" (bars + peak-hold caps), and "audiowave_orb_led"
    // (LED rungs colored by usage zone + band-specific bloom).
    if (rice.startsWith("audiowave_orb")) {
      const style = rice === "audiowave_orb_peak" ? "peak"
        : rice === "audiowave_orb_led" ? "led" : "bars";
      window.Orb.setStyle(style);
      window.Orb.start();
    } else {
      window.Orb.stop();
    }
  }
  render();
}));
// Ask the backend to (re)send prefs — emissions before this listener
// attached were lost.
logRejection(window.__TAURI__.event.emit("ui-ready"));

// Countdown ticks locally between updates.
setInterval(render, 30000);
render();

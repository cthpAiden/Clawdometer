// Default skin. Contract: `state-updated` (usage data) and `ui-prefs`
// (opacity/compact) events in; `ui-ready` and `toggle-compact` events out.

// If Tauri's bridge injection ever fails, show a hint instead of throwing
// mid-setup (an uncaught throw here would also kill render/setInterval below).
if (!window.__TAURI__) {
  document.getElementById("age").textContent = "tauri bridge missing — restart the HUD";
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
  footer: document.getElementById("footer"),
  age: document.getElementById("age"),
  reset: document.getElementById("reset"),
};

let current = null; // last payload
let compactMode = false; // mirrors the tray's "Compact size" toggle

// Usage colors live in usage-color.js — the orb skin reads the same table, and
// the working stripes read the same custom properties.
const { paint, num: numColor } = window.UsageColor;

// Header countdown to the 5h window reset. Account-wide limits, so this beats a
// model name. Leading ↻ marks it as a reset countdown so the number isn't read as
// usage; regular spells "resets in", compact drops the words to fit the pill.
// Past the reset the backend derives 0% (the window is gone until the next
// request opens one), so the label switches to a plain "reset".
function fmtCountdown(resetsAtEpochSec, nowMs, compact) {
  if (!Number.isFinite(resetsAtEpochSec)) return "—";
  const mins = Math.ceil((resetsAtEpochSec * 1000 - nowMs) / 60000);
  if (mins <= 0) return "↻ reset";
  const core = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
  return compact ? `↻ ${core}` : `↻ resets in ${core}`;
}

// The dominant 5h window: big number + bar to its right, both threshold-colored.
function renderPrimary(win) {
  if (!win || typeof win.used_percentage !== "number") {
    els.txt5h.textContent = "—";
    els.txt5h.style.color = "";
    els.bar5h.style.width = "0%";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  // textContent + createElement, never innerHTML: the state files are the
  // input here, and a template injection sink is one refactor away from XSS.
  els.txt5h.textContent = "";
  els.txt5h.append(String(win.used_percentage));
  const unit = document.createElement("span");
  unit.className = "u";
  unit.textContent = "%";
  els.txt5h.append(unit);
  els.txt5h.style.color = numColor(win.used_percentage);
  els.bar5h.style.width = pct + "%";
  paint(els.bar5h, win.used_percentage);
}

// A demoted window (weekly, Fable): thin bar (threshold-colored) + muted
// percentage.
// fable_week is absent until a /usage refresh has seen it (the statusline hook
// never carries it), and absent entirely if Fable hasn't been used this week —
// both render as "—", not as 0%, which would be a lie.
function renderSecondary(win, bar, txt) {
  if (!win || typeof win.used_percentage !== "number") {
    txt.textContent = "—";
    bar.style.width = "0%";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  txt.textContent = `${win.used_percentage}%`;
  bar.style.width = pct + "%";
  paint(bar, win.used_percentage);
}

// Weekly reset day + local time, from the seven_day.resets_at the statusline
// snapshot stores (epoch seconds) — the exact field Claude's own UI renders as
// "Resets Thu 10:00 AM". Regular size only; compact has no footer room.
function fmtResetDay(resetsAtEpochSec) {
  if (!Number.isFinite(resetsAtEpochSec)) return "";
  const d = new Date(resetsAtEpochSec * 1000);
  const day = d.toLocaleDateString([], { weekday: "short" });
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return `${day} ${time}`;
}

function fmtAge(capturedAtIso, nowMs) {
  const t = Date.parse(capturedAtIso);
  if (Number.isNaN(t)) return "";
  const mins = Math.floor((nowMs - t) / 60000);
  if (mins < -1) return ""; // future timestamp (clock stepped back); render() flags it stale
  if (mins < 1) return "as of just now";
  if (mins < 60) return `as of ${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `as of ${hours}h ${mins % 60}m ago`;
}

function render() {
  const nowMs = Date.now();
  // Animate while a Claude Code session is mid-turn; the backend derives this
  // flag from transcript turn state (see watcher::any_session_generating) so
  // idle live.json refreshes never trip it.
  document.body.classList.toggle("working", !!(current && current.working));
  const state = current && current.state;
  if (!state || !state.rate_limits) {
    els.countdown.textContent = "—";
    els.countdown.style.color = "";
    renderPrimary(null);
    renderSecondary(null, els.bar7d, els.txt7d);
    renderSecondary(null, els.barfb, els.txtfb);
    els.age.textContent = "waiting for data — open Claude Code";
    els.reset.textContent = "";
    els.footer.classList.remove("stale");
    document.body.classList.remove("critical");
    return;
  }
  const fh = state.rate_limits.five_hour;
  els.countdown.textContent = fmtCountdown(fh && fh.resets_at, nowMs, compactMode);
  renderPrimary(fh);
  renderSecondary(state.rate_limits.seven_day, els.bar7d, els.txt7d);
  renderSecondary(state.rate_limits.fable_week, els.barfb, els.txtfb);

  const fivePct = fh && typeof fh.used_percentage === "number" ? fh.used_percentage : 0;
  const critical = fivePct >= 90;
  document.body.classList.toggle("critical", critical);
  els.countdown.style.color = critical ? "#e5484d" : "";

  // The refresher normally keeps data under a minute old, so 30+ minutes
  // means it's failing (claude CLI missing/broken) — say how to retry
  // instead of silently aging. The backend zeroes any window whose reset
  // passed, which keeps an idle 5h number honest.
  const ageMs = nowMs - Date.parse(state.captured_at);
  // Unparseable or future-stamped captured_at (clock stepped back) is as
  // untrustworthy as old data — never render it as fresh.
  const stale = !Number.isFinite(ageMs) || ageMs > 30 * 60000 || ageMs < -60000;
  const hint = " — " + (
    Number.isFinite(ageMs) && ageMs < -60000
      ? "check system clock"
      : "tray → Refresh usage");
  els.age.textContent = stale
    ? (fmtAge(state.captured_at, nowMs) || "stale data") + hint
    : fmtAge(state.captured_at, nowMs);
  els.footer.classList.toggle("stale", stale);
  const sd = state.rate_limits.seven_day;
  // A weekly reset time in the past is no longer a schedule — drop the line
  // (the backend has already zeroed the percentage).
  const resetDay =
    sd && sd.resets_at * 1000 > nowMs ? fmtResetDay(sd.resets_at) : "";
  els.reset.textContent = "";
  if (resetDay) {
    els.reset.append("resets ");
    const day = document.createElement("b");
    day.textContent = resetDay;
    els.reset.append(day);
  }
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
  for (const id of ["bento", "orb"]) {
    const el = document.getElementById(id);
    if (el) el.style.opacity = opacity;
  }
  // Switch skins: Classic card, Bento Box, or Audiowave Orb. The orb's
  // animation loop only runs while it's the active skin (the backend likewise
  // gates audio capture on it), so the other two cost nothing extra.
  const rice = typeof p.rice === "string" ? p.rice : "classic";
  document.body.dataset.rice = rice;
  if (window.Orb) {
    // Two orb variants share the "audiowave_orb" prefix: plain "Bars" and
    // "audiowave_orb_peak" (bars + peak-hold caps).
    if (rice.startsWith("audiowave_orb")) {
      window.Orb.setPeak(rice === "audiowave_orb_peak");
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

// Age line ticks locally between updates.
setInterval(render, 30000);
render();

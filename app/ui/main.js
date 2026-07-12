// Default skin. Contract: `state-updated` (usage data) and `ui-prefs`
// (opacity/compact) events in; `ui-ready` and `toggle-compact` events out.

// The HUD is chrome, not a page — suppress WebView2's context menu
// (Back/Refresh/Save as/Print).
window.addEventListener("contextmenu", (e) => e.preventDefault());

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
    window.__TAURI__.window.getCurrentWindow().startDragging();
  }
});
window.addEventListener("mouseup", () => { dragOrigin = null; });
window.addEventListener("dblclick", (e) => {
  if (e.button === 0) window.__TAURI__.event.emit("toggle-compact");
});

const els = {
  card: document.getElementById("card"),
  countdown: document.getElementById("countdown"),
  bar5h: document.getElementById("bar5h"),
  bar7d: document.getElementById("bar7d"),
  txt5h: document.getElementById("txt5h"),
  txt7d: document.getElementById("txt7d"),
  footer: document.getElementById("footer"),
};

let current = null; // last payload
let compactMode = false; // mirrors the tray's "Compact size" toggle

// Usage colors: calm green when safe, amber past 70%, red past 90% — so a low
// number reads as safe instead of the old always-orange bar.
const barColor = (pct) => (pct >= 90 ? "#e5484d" : pct >= 70 ? "#f59e0b" : "#4a7c47");
const numColor = (pct) => (pct >= 90 ? "#e5484d" : pct >= 70 ? "#f0b429" : "#63b35f");

// Header countdown to the 5h window reset. Account-wide limits, so this beats a
// model name. Compact shows just the duration; the label supplies the context.
function fmtCountdown(resetsAtEpochSec, nowMs, compact) {
  if (!Number.isFinite(resetsAtEpochSec)) return "—";
  const mins = Math.ceil((resetsAtEpochSec * 1000 - nowMs) / 60000);
  if (mins <= 0) return compact ? "resetting…" : "resetting…";
  const core = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
  return compact ? core : `resets in ${core}`;
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
  els.txt5h.innerHTML = `${win.used_percentage}<span class="u">%</span>`;
  els.txt5h.style.color = numColor(win.used_percentage);
  els.bar5h.style.width = pct + "%";
  els.bar5h.style.background = barColor(win.used_percentage);
}

// The demoted weekly window: thin bar (threshold-colored) + muted percentage.
function renderSecondary(win) {
  if (!win || typeof win.used_percentage !== "number") {
    els.txt7d.textContent = "—";
    els.bar7d.style.width = "0%";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  els.txt7d.textContent = `${win.used_percentage}%`;
  els.bar7d.style.width = pct + "%";
  els.bar7d.style.background = barColor(win.used_percentage);
}

function fmtAge(capturedAtIso, nowMs) {
  const t = Date.parse(capturedAtIso);
  if (Number.isNaN(t)) return "";
  const mins = Math.floor((nowMs - t) / 60000);
  if (mins < 1) return "as of just now";
  if (mins < 60) return `as of ${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `as of ${hours}h ${mins % 60}m ago`;
}

function render() {
  const nowMs = Date.now();
  const state = current && current.state;
  if (!state || !state.rate_limits) {
    els.countdown.textContent = "—";
    els.countdown.style.color = "";
    renderPrimary(null);
    renderSecondary(null);
    els.footer.textContent = "waiting for usage data";
    els.footer.classList.remove("stale");
    document.body.classList.remove("critical");
    return;
  }
  const fh = state.rate_limits.five_hour;
  els.countdown.textContent = fmtCountdown(fh && fh.resets_at, nowMs, compactMode);
  renderPrimary(fh);
  renderSecondary(state.rate_limits.seven_day);

  const fivePct = fh && typeof fh.used_percentage === "number" ? fh.used_percentage : 0;
  const critical = fivePct >= 90;
  document.body.classList.toggle("critical", critical);
  els.countdown.style.color = critical ? "#e5484d" : "";

  // Data older than ~10 missed polls means the poller is failing (network
  // down or sign-in expired) — make that visible instead of silently aging.
  const ageMs = nowMs - Date.parse(state.captured_at);
  const stale = Number.isFinite(ageMs) && ageMs > 10 * 60000;
  els.footer.textContent =
    fmtAge(state.captured_at, nowMs) + (stale ? " — poll failing, open Claude Code" : "");
  els.footer.classList.toggle("stale", stale);
}

window.__TAURI__.event.listen("state-updated", (event) => {
  current = event.payload;
  render();
});

window.__TAURI__.event.listen("ui-prefs", (event) => {
  const p = event.payload || {};
  compactMode = !!p.compact;
  document.body.classList.toggle("compact", compactMode);
  els.card.style.opacity = typeof p.opacity === "number" ? p.opacity : 1;
  render();
});
// Ask the backend to (re)send prefs — emissions before this listener
// attached were lost.
window.__TAURI__.event.emit("ui-ready");

// Age line ticks locally between updates.
setInterval(render, 30000);
render();

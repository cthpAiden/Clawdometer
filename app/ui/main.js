// Default skin. Contract: the single `state-updated` event. No other IPC.
const els = {
  model: document.getElementById("model"),
  bar5h: document.getElementById("bar5h"),
  bar7d: document.getElementById("bar7d"),
  txt5h: document.getElementById("txt5h"),
  txt7d: document.getElementById("txt7d"),
  footer: document.getElementById("footer"),
};

let current = null; // last payload

function fmtReset(resetsAtEpochSec, nowMs) {
  if (!Number.isFinite(resetsAtEpochSec)) return "";
  if (resetsAtEpochSec * 1000 < nowMs) return "refresh pending";
  const d = new Date(resetsAtEpochSec * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `resets ${hh}:${mm}`;
}

function renderWindow(win, barEl, txtEl, nowMs) {
  if (!win || typeof win.used_percentage !== "number") {
    barEl.style.width = "0%";
    txtEl.textContent = "—";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  barEl.style.width = pct + "%";
  barEl.style.background = pct >= 90 ? "#dc2626" : pct >= 70 ? "#f59e0b" : "#d97706";
  txtEl.textContent = `${win.used_percentage}% · ${fmtReset(win.resets_at, nowMs)}`;
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
    els.model.textContent = (state && state.model && state.model.display_name) || "Clawdometer";
    renderWindow(null, els.bar5h, els.txt5h, nowMs);
    renderWindow(null, els.bar7d, els.txt7d, nowMs);
    els.footer.textContent = "waiting for usage data";
    return;
  }
  els.model.textContent = (state.model && state.model.display_name) || "Claude";
  renderWindow(state.rate_limits.five_hour, els.bar5h, els.txt5h, nowMs);
  renderWindow(state.rate_limits.seven_day, els.bar7d, els.txt7d, nowMs);
  els.footer.textContent = fmtAge(state.captured_at, nowMs);
}

window.__TAURI__.event.listen("state-updated", (event) => {
  current = event.payload;
  render();
});

// Age line ticks locally between updates.
setInterval(render, 30000);
render();

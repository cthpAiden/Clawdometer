// Bento Box skin (U10 "Mini Bento"). The same snapshot Classic renders, laid
// out as a compact grid: Current is the hero cell across the top, Weekly and
// Fable share the bottom two. Fed by "state-updated"; no animation loop of its
// own, so it costs nothing but a few DOM writes per payload (the mascot's
// working animation is pure CSS in style.css, shared with Classic).
(function () {
  if (!window.__TAURI__) return;

  // Shared threshold table, same as the card and the orb: the three skins must
  // never disagree about what 76% looks like. Only the bars are colored (the
  // numbers stay off-white), so we take paint() but not num().
  const { paint } = window.UsageColor;

  const els = {
    brtop: document.getElementById("brtop"),
    b5v: document.getElementById("b5v"), b5b: document.getElementById("b5b"),
    b7v: document.getElementById("b7v"), b7b: document.getElementById("b7b"),
    bfv: document.getElementById("bfv"), bfb: document.getElementById("bfb"),
  };

  let lastPayload = null;

  const clampPct = (v) => Math.max(0, Math.min(100, v));
  const pctText = (win) =>
    win && typeof win.used_percentage === "number" ? `${win.used_percentage}%` : "—";

  // A usage cell with a bar. Absent → "—", never 0% (fable_week is missing
  // until a /usage refresh has seen it, and until Fable is used this week —
  // neither of which means "none used").
  function renderCell(win, val, bar) {
    val.textContent = pctText(win);
    if (win && typeof win.used_percentage === "number") {
      bar.style.width = clampPct(win.used_percentage) + "%";
      paint(bar, win.used_percentage);
    } else {
      bar.style.width = "0%";
    }
  }

  // 5h reset countdown, top-right — mirrors Classic's fmtCountdown. Compact
  // drops the word to fit; past the reset the window is gone (the backend has
  // derived 0%), so it reads a plain "reset".
  function renderReset(fh, nowMs) {
    const at = fh && fh.resets_at;
    if (!Number.isFinite(at)) { els.brtop.textContent = "—"; return; }
    const mins = Math.ceil((at * 1000 - nowMs) / 60000);
    if (mins <= 0) { els.brtop.textContent = "reset"; return; }
    const core = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
    els.brtop.textContent = document.body.classList.contains("compact") ? core : `Resets ${core}`;
  }

  function render() {
    const rl = lastPayload && lastPayload.state && lastPayload.state.rate_limits;
    renderCell(rl && rl.five_hour, els.b5v, els.b5b);
    renderCell(rl && rl.seven_day, els.b7v, els.b7b);
    renderCell(rl && rl.fable_week, els.bfv, els.bfb);
    renderReset(rl && rl.five_hour, Date.now());
  }

  window.__TAURI__.event.listen("state-updated", (e) => {
    lastPayload = e.payload;
    render();
  }).catch(console.error);

  // Re-render on prefs changes too, so the reset line switches between its full
  // and compact wording the moment the size toggles (main.js registers first,
  // so body.compact is already updated when this fires).
  window.__TAURI__.event.listen("ui-prefs", () => render()).catch(console.error);

  // The reset countdown ticks locally between snapshots (minute granularity).
  setInterval(render, 30000);
})();

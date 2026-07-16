// Bento Box skin. The same snapshot Classic renders, laid out as a 2×2 of
// self-contained cells: 5H (hero), 7D, Fable, RESET. Fed by "state-updated";
// no animation loop of its own, so it costs nothing but four DOM writes per
// payload (the working stripes ride Classic's .bar.main rule in style.css).
(function () {
  if (!window.__TAURI__) return;

  // The 5h window's full length. resets_at marks its end, so the elapsed
  // fraction — what the RESET bar draws — is whatever isn't still remaining.
  const WINDOW_SECS = 5 * 3600;

  // Shared threshold table, same as the card and the orb: the three skins must
  // never disagree about what 76% looks like.
  const { paint, num: numColor } = window.UsageColor;

  const els = {
    b5v: document.getElementById("b5v"), b5b: document.getElementById("b5b"),
    b7v: document.getElementById("b7v"), b7b: document.getElementById("b7b"),
    bfv: document.getElementById("bfv"), bfb: document.getElementById("bfb"),
    brv: document.getElementById("brv"), brb: document.getElementById("brb"),
  };

  let lastPayload = null;

  const clampPct = (v) => Math.max(0, Math.min(100, v));

  // A usage cell. An absent window renders "—", never 0% — fable_week is
  // missing until a /usage refresh has seen it, and missing entirely until
  // Fable is used this week, neither of which means "none used".
  function renderCell(win, val, bar) {
    const pct = win && typeof win.used_percentage === "number" ? win.used_percentage : null;
    if (pct === null) {
      val.textContent = "—";
      val.style.color = "";
      bar.style.width = "0%";
      return;
    }
    // textContent + createElement, never innerHTML: the state files are the
    // input here, and a template injection sink is one refactor away from XSS.
    val.textContent = "";
    val.append(String(pct));
    const unit = document.createElement("span");
    unit.className = "u";
    unit.textContent = "%";
    val.append(unit);
    val.style.color = numColor(pct);
    bar.style.width = clampPct(pct) + "%";
    paint(bar, pct);
  }

  // The RESET cell: time left as the value, elapsed share of the window as the
  // bar. Past the reset the window is gone until the next request opens one
  // (the backend has already derived 0%), so an empty bar is the honest draw —
  // a full one would claim the window is about to turn over.
  function renderReset(fh, nowMs) {
    const at = fh && fh.resets_at;
    if (!Number.isFinite(at)) {
      els.brv.textContent = "—";
      els.brb.style.width = "0%";
      return;
    }
    const mins = Math.ceil((at * 1000 - nowMs) / 60000);
    if (mins <= 0) {
      els.brv.textContent = "reset";
      els.brb.style.width = "0%";
      return;
    }
    els.brv.textContent = mins < 60 ? `${mins}m` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
    const remaining = (at * 1000 - nowMs) / 1000;
    els.brb.style.width = clampPct((1 - remaining / WINDOW_SECS) * 100) + "%";
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

  // The reset countdown ticks locally between snapshots (minute granularity).
  setInterval(render, 30000);
})();

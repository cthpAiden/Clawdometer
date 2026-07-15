// Usage threshold colors, shared by every skin. Single source of truth: the
// card and the orb must never disagree about what 76% looks like, and the
// working stripes have to track the same level or they mask it (they used to
// paint the main bar green regardless of state).
//
//   blue   < 75%   normal
//   yellow 75-90%  approaching limit
//   red    >= 90%  critical / limit reached
//
// `lit` is the stripe highlight band — the same hue lifted, so a working bar
// still reads at its own level instead of falling back to one fixed color.
window.UsageColor = (() => {
  const LEVELS = {
    ok:   { bar: "#3f7fbf", lit: "#5b98d4", num: "#6aa9e8" },
    warn: { bar: "#f59e0b", lit: "#ffb733", num: "#f0b429" },
    crit: { bar: "#e5484d", lit: "#f26a6e", num: "#e5484d" },
  };

  const level = (pct) => (pct >= 90 ? "crit" : pct >= 75 ? "warn" : "ok");

  // Paint a `.fill` element: --u drives its own background, --u-lit the
  // working stripes drawn over it (see .fill::after in style.css).
  const paint = (el, pct) => {
    const l = LEVELS[level(pct)];
    el.style.setProperty("--u", l.bar);
    el.style.setProperty("--u-lit", l.lit);
  };

  const num = (pct) => LEVELS[level(pct)].num;

  return { LEVELS, level, paint, num };
})();

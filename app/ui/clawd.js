// Clawd mascot injector, shared by Classic and Bento Box. Fills every
// `.clawd-i` holder with the LOCKED sprite (geometry from the mascot spec —
// never redraw by eye). Body rects are `currentColor` (the skin paints the
// orange); the two eyes are a separate group so they can blink and scan.
//
// The markup is a compile-time constant with no state-derived input, so
// innerHTML here is not an injection sink — unlike the usage numbers, which
// main.js/bento.js still write via textContent. All motion (idle eye-blink,
// the isWorking "Coder" bob + eye-scan) is pure CSS in style.css, keyed off
// body.working — nothing to animate from JS.
(function () {
  // [x0, y0, x1, y1] in the 100×70 viewBox. Inner edges overlap ~1u UNDER the
  // body so the per-part groups (needed for the bob/scan) don't reveal
  // anti-alias seams; the outer silhouette stays pixel-exact. Arm reach is
  // 17u past each body edge, spilling 1.2u outside the viewBox — the svg is
  // overflow:visible, so it renders; widening the viewBox instead would shift
  // the animation transform-origins (they're view-box relative).
  const BODY = [15.8, 0, 84.2, 47.6];
  const ARM_L = [-1.2, 16.9, 19.0, 35.8], ARM_R = [81.0, 16.9, 101.2, 35.8];
  const LEGS = [
    [15.8, 42.0, 25.5, 64.8], [33.4, 42.0, 43.1, 64.8],
    [56.9, 42.0, 66.6, 64.8], [74.5, 42.0, 84.2, 64.8],
  ];
  const EYES = [[24.9, 7.8, 32.7, 16.3], [67.3, 7.8, 75.1, 16.3]];

  const r = (a) =>
    `<rect x="${a[0]}" y="${a[1]}" width="${(a[2] - a[0]).toFixed(2)}" height="${(a[3] - a[1]).toFixed(2)}"/>`;

  function sprite(px) {
    const w = (17 * px).toFixed(1), h = (11.9 * px).toFixed(1);
    // Legs and arms are wrapped in their own groups so the random per-turn
    // working animations can move them independently (leg-tap, arm-knead).
    // The body silhouette is unchanged; groups only add transform hooks.
    return `<svg width="${w}" height="${h}" viewBox="0 0 100 70" xmlns="http://www.w3.org/2000/svg">` +
      `<g class="g-all"><g fill="currentColor">${r(BODY)}` +
      `<g class="g-armL">${r(ARM_L)}</g><g class="g-armR">${r(ARM_R)}</g>` +
      `<g class="g-leg1">${r(LEGS[0])}</g><g class="g-leg2">${r(LEGS[1])}</g>` +
      `<g class="g-leg3">${r(LEGS[2])}</g><g class="g-leg4">${r(LEGS[3])}</g></g>` +
      `<g class="g-eyes"><g class="eyes" fill="#111">${EYES.map(r).join("")}</g></g></g></svg>`;
  }

  document.querySelectorAll(".clawd-i").forEach((el) => {
    el.classList.add("clawd");
    el.innerHTML = sprite(parseFloat(el.dataset.px || "1.15"));
  });
})();

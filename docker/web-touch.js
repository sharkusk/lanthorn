// lanthorn's browser touch-scroll shim, injected into ttyd's page by the
// entrypoint (docker/entrypoint.sh's build_index(), unconditionally unless
// LANTHORN_WEB_TOUCH=off — unlike web-audio.js this needs no session id or
// websocket, so there's no port to gate it on).
//
// lanthorn runs on the alternate screen with mouse tracking on, so xterm.js
// already converts wheel events into mouse-wheel reports lanthorn understands
// (crates/app/src/input.rs maps MouseEventKind::ScrollUp/Down onto the
// transcript and map) — but xterm.js only wires touchstart/touchmove to ITS
// OWN viewport scroll (browser/Viewport.ts's handleTouchStart/handleTouchMove),
// which scrolls xterm's scrollback and is a no-op on the alternate screen. A
// drag on an iPad reaches nothing. This turns a vertical touch drag into
// synthetic wheel events dispatched on the touched element, which bubble up
// to the same element xterm.js's own wheel listener is bound to (`.xterm`,
// `this.element` in browser/Terminal.ts) — so xterm's existing
// wheel-to-mouse-report path carries it the rest of the way and nothing
// downstream needs to know this isn't a real mouse.
//
// Sign: xterm.js's own handleTouchMove treats an upward drag (content should
// move up, i.e. scroll further into the transcript) as `lastY - clientY`;
// matched here so a finger-up drag reads the same as it would on xterm's own
// viewport.
//
// Row-height quantisation: xterm.js 5.5's Viewport.getLinesScrolled() divides
// a DOM_DELTA_PIXEL deltaY by its own private `_currentRowHeight` and floors
// the result, carrying any fractional remainder in `_wheelPartialScroll` for
// the next wheel event. Rather than lean on that private accumulator (whose
// starting remainder we don't control and can't read), this accumulates the
// drag itself and only ever dispatches deltas that are exact multiples of one
// row height — so getLinesScrolled's floor divides evenly, produces exactly
// the row count intended, and leaves no fractional residue behind for the
// next real mouse-wheel event to inherit. xterm.js 5.x has no `.xterm-rows`
// DOM row to measure (it renders to canvas, not a DOM grid); the row height
// it actually computes and uses internally is read here from
// `.xterm-char-measure-element`, the hidden probe span
// browser/services/CharSizeService.ts keeps permanently in the DOM for
// exactly this measurement — the same number Viewport derives its row height
// from, rather than a guess at the font's line height.
(function () {
  var TOUCH_SLOP = 4; // px of near-horizontal wobble tolerated before a drag is treated as horizontal
  var active = false;
  var lastX = 0, lastY = 0, accum = 0;

  function target() {
    return document.querySelector(".xterm") ||
      document.getElementById("terminal-container") ||
      document.body;
  }

  function rowHeight() {
    var probe = document.querySelector(".xterm-char-measure-element");
    var h = probe && probe.offsetHeight;
    return (h && h > 0) ? h : 16;
  }

  function onTouchStart(ev) {
    if (ev.touches.length !== 1) { active = false; return; }
    var host = target();
    if (!host.contains(ev.target)) { active = false; return; }
    active = true;
    accum = 0;
    lastX = ev.touches[0].clientX;
    lastY = ev.touches[0].clientY;
    // touchstart itself is left untouched (no preventDefault) so tap-to-focus
    // and the on-screen keyboard still work — only a subsequent touchmove
    // that turns out to be a vertical drag gets intercepted.
  }

  function onTouchMove(ev) {
    if (!active || ev.touches.length !== 1) { return; }
    var t = ev.touches[0];
    var dx = t.clientX - lastX;
    var dy = lastY - t.clientY;
    if (Math.abs(dx) > Math.abs(dy) + TOUCH_SLOP) {
      // Mostly-horizontal drag: not ours, stop tracking and let the page
      // handle it (e.g. text selection) as it normally would.
      active = false;
      return;
    }
    ev.preventDefault(); // a real vertical drag: keep the page from rubber-banding
    lastX = t.clientX;
    lastY = t.clientY;
    accum += dy;

    var rh = rowHeight();
    var rows = Math.trunc(accum / rh);
    if (rows === 0) { return; }
    accum -= rows * rh; // keep the sub-row remainder for the next move

    ev.target.dispatchEvent(new WheelEvent("wheel", {
      deltaY: rows * rh,
      deltaMode: WheelEvent.DOM_DELTA_PIXEL,
      clientX: t.clientX,
      clientY: t.clientY,
      bubbles: true,
      cancelable: true
    }));
  }

  function onTouchEnd() {
    active = false;
  }

  document.addEventListener("touchstart", onTouchStart, { passive: true });
  document.addEventListener("touchmove", onTouchMove, { passive: false });
  document.addEventListener("touchend", onTouchEnd, { passive: true });
  document.addEventListener("touchcancel", onTouchEnd, { passive: true });
})();

// lanthorn's browser touch shim, injected into ttyd's page by the entrypoint
// (docker/entrypoint.sh's build_index(), unconditionally unless
// LANTHORN_WEB_TOUCH=off — unlike web-audio.js this needs no session id or
// websocket, so there's no port to gate it on).
//
// lanthorn runs on the alternate screen with mouse tracking on, so xterm.js
// already converts real mouse events into the reports lanthorn understands
// (crates/app/src/input.rs maps MouseEventKind::ScrollUp/Down/Drag/Down/Up
// onto the transcript, map pan (SQ-1325) and the pane-boundary drags in
// crates/app/src/main.rs's PaneRects) — but xterm.js only wires
// touchstart/touchmove to ITS OWN viewport scroll (browser/Viewport.ts's
// handleTouchStart/handleTouchMove), which scrolls xterm's scrollback and is
// a no-op on the alternate screen, and it never synthesises a touch into a
// mouse DRAG at all. A drag on an iPad reaches nothing unless this file
// turns the touch into the mouse events xterm.js already knows how to
// forward.
//
// Gesture table (SQ-1324), classified once at touchstart/first touchmove and
// then held for the rest of that touch contact — see createGestureTracker
// below:
//
//   fingers   direction              becomes
//   -------   ---------------------  ------------------------------------
//   1         vertical (or still)    synthetic wheel events (unchanged)
//   1         horizontal             synthetic mouse drag (down/move/up)
//   2         either                 synthetic mouse drag (down/move/up)
//   1         none (a tap)           nothing — untouched, so tap-to-focus
//                                    and the on-screen keyboard still work
//
// One-finger horizontal was dead on the story/map panes before this (xterm's
// own viewport only scrolls vertically), so claiming it for a drag costs
// nothing there; two fingers is free for the same reason and gives the map a
// way to pan vertically too without touching one-finger vertical scroll,
// which must and does keep working exactly as before.
//
// Why a drag reaches the app at all: xterm.js's Terminal.bindMouse() (see
// browser/Terminal.ts in the version ttyd 1.7.7 bundles — @xterm/xterm
// ^5.4.0 per its html/package.json; checked here against the 5.5.0 tag,
// which is also what the row-height comment below already assumes) attaches
// an "always on" 'mousedown' listener directly to `this.element` (the
// `.xterm` host — the same element the wheel synthesis below dispatches on,
// so it bubbles to the same place). On mousedown it calls
// `sendEvent(ev)` (reads `ev.button`) and then, only if the terminal's
// active mouse-report protocol has requested them, adds 'mousemove' and
// 'mouseup' listeners on `document` for the rest of the drag — 'mousemove'
// only counts if `ev.buttons` has the left-button bit set
// (`eventListeners.mousedrag`), and 'mouseup' clears the listeners once
// `ev.buttons` reads empty again. Coordinates come from
// `MouseService.getMouseReportCoords` → `browser/input/Mouse.ts`'s
// `getCoordsRelativeToElement`, which reads only `event.clientX`/`clientY`
// against `getBoundingClientRect()` — so a synthetic `MouseEvent` built with
// real `clientX`/`clientY` (plus `button`/`buttons`) is indistinguishable
// from a native one at every step of that path.
//
// What puts the terminal in a protocol that requests DRAG/MOVE at all:
// crossterm's `EnableMouseCapture` (crossterm 0.29's `src/event.rs`) writes
// `CSI ?1000h CSI ?1002h CSI ?1003h CSI ?1006h` — X10 mouse reporting,
// button-event tracking, any-motion tracking, and SGR extended coordinates —
// as ordinary output bytes that flow through ttyd's pty exactly like any
// other lanthorn output, so `CoreMouseService` sees the same escape codes it
// would from a native terminal and requests the DRAG/MOVE/UP event classes
// from `bindMouse()` above. The resulting reports flow back through the pty
// the same way a real drag's would, and land as
// `MouseEventKind::Down`/`Drag(Left)`/`Up(Left)` on lanthorn's crossterm
// reader — nothing downstream needs to know this isn't a real mouse.
//
// Sign, for the wheel path: xterm.js's own handleTouchMove treats an upward
// drag (content should move up, i.e. scroll further into the transcript) as
// `lastY - clientY`; matched here so a finger-up drag reads the same as it
// would on xterm's own viewport.
//
// Row-height quantisation, for the wheel path: xterm.js 5.5's
// Viewport.getLinesScrolled() divides a DOM_DELTA_PIXEL deltaY by its own
// private `_currentRowHeight` and floors the result, carrying any fractional
// remainder in `_wheelPartialScroll` for the next wheel event. Rather than
// lean on that private accumulator (whose starting remainder we don't
// control and can't read), this accumulates the drag itself and only ever
// dispatches deltas that are exact multiples of one row height — so
// getLinesScrolled's floor divides evenly, produces exactly the row count
// intended, and leaves no fractional residue behind for the next real
// mouse-wheel event to inherit. xterm.js 5.x has no `.xterm-rows` DOM row to
// measure (it renders to canvas, not a DOM grid); the row height it actually
// computes and uses internally is read here from
// `.xterm-char-measure-element`, the hidden probe span
// browser/services/CharSizeService.ts keeps permanently in the DOM for
// exactly this measurement — the same number Viewport derives its row
// height from, rather than a guess at the font's line height. The same probe
// gives a cell width too: CharSizeService measures a run of 32 'W's and
// divides `offsetWidth` by 32 (`DomMeasureStrategyConstants.REPEAT`), so
// this reads `offsetWidth / 32` for one cell's width.
//
// Drag throttling: xterm.js resolves a mouse position to a col/row (or
// device pixel, under SGR-pixel mode — not used here) and de-duplicates
// against its last-reported event (`CoreMouseService._equalEvents`), so a
// flood of same-cell mousemoves is harmless but wasted. This dispatches a
// mousemove only once the finger has moved a full cell width or height since
// the last dispatched point, which is enough to report every cell the drag
// crosses without spamming a mousemove per pixel of touch movement.
//
// createGestureTracker is a pure state machine (touch points and cell
// measurements in, {actions, preventDefault} out) with no DOM dependency, so
// it is exercised directly by docker/web-touch.test.js under plain Node;
// see that file to run it (`node docker/web-touch.test.js`). The IIFE below
// wraps it with the actual DOM glue and only runs where `document` exists.
(function () {
  var TOUCH_SLOP = 4; // px of near-horizontal wobble tolerated before a 1-finger drag commits to horizontal

  function centroid(touches) {
    var sx = 0, sy = 0;
    for (var i = 0; i < touches.length; i++) {
      sx += touches[i].x;
      sy += touches[i].y;
    }
    return { x: sx / touches.length, y: sy / touches.length };
  }

  // measure() => { rowHeight: number, cellWidth: number }, read fresh on
  // every move since a font-ready re-measure (web-font.js) can change it
  // mid-session.
  function createGestureTracker(measure) {
    var pendingStart = null; // {x,y} — one finger down, direction not yet classified
    // {type:'wheel', accum, lastX, lastY} or
    // {type:'drag', fingerCount, lastDispatchX, lastDispatchY}
    var gesture = null;

    function endDrag() {
      var actions = [];
      if (gesture && gesture.type === "drag") {
        actions.push({ type: "mouseup", x: gesture.lastDispatchX, y: gesture.lastDispatchY });
      }
      gesture = null;
      return actions;
    }

    function wheelMove(x, y) {
      var dy = gesture.lastY - y;
      gesture.lastX = x;
      gesture.lastY = y;
      gesture.accum += dy;

      var rh = measure().rowHeight;
      var rows = Math.trunc(gesture.accum / rh);
      if (rows === 0) {
        return { actions: [], preventDefault: true };
      }
      gesture.accum -= rows * rh; // keep the sub-row remainder for the next move

      return {
        actions: [{ type: "wheel", deltaY: rows * rh, x: x, y: y }],
        preventDefault: true
      };
    }

    function dragMove(x, y) {
      var m = measure();
      if (
        Math.abs(x - gesture.lastDispatchX) >= m.cellWidth ||
        Math.abs(y - gesture.lastDispatchY) >= m.rowHeight
      ) {
        gesture.lastDispatchX = x;
        gesture.lastDispatchY = y;
        return { actions: [{ type: "mousemove", x: x, y: y }], preventDefault: true };
      }
      return { actions: [], preventDefault: true };
    }

    return {
      onTouchStart: function (touches) {
        if (touches.length === 1) {
          var endedActions = endDrag();
          pendingStart = { x: touches[0].x, y: touches[0].y };
          return { actions: endedActions, preventDefault: false };
        }
        if (touches.length === 2) {
          var preActions = endDrag();
          var c = centroid(touches);
          gesture = { type: "drag", fingerCount: 2, lastDispatchX: c.x, lastDispatchY: c.y };
          pendingStart = null;
          preActions.push({ type: "mousedown", x: c.x, y: c.y });
          return { actions: preActions, preventDefault: false };
        }
        // 3+ fingers: not a gesture we handle.
        var abortActions = endDrag();
        pendingStart = null;
        return { actions: abortActions, preventDefault: false };
      },

      onTouchMove: function (touches) {
        if (touches.length === 0 || touches.length >= 3) {
          var actions = endDrag();
          pendingStart = null;
          return { actions: actions, preventDefault: false };
        }

        if (touches.length === 2) {
          if (!gesture || gesture.type !== "drag" || gesture.fingerCount !== 2) {
            // Defensive: a touchstart should already have committed this,
            // but if event order ever surprises us, start the drag here.
            var c2 = centroid(touches);
            gesture = { type: "drag", fingerCount: 2, lastDispatchX: c2.x, lastDispatchY: c2.y };
            return { actions: [{ type: "mousedown", x: c2.x, y: c2.y }], preventDefault: true };
          }
          var c = centroid(touches);
          return dragMove(c.x, c.y);
        }

        // touches.length === 1
        var t = touches[0];
        if (!gesture) {
          if (!pendingStart) {
            pendingStart = { x: t.x, y: t.y };
            return { actions: [], preventDefault: false };
          }
          var dx = t.x - pendingStart.x;
          var dy = pendingStart.y - t.y;
          if (dx === 0 && dy === 0) {
            return { actions: [], preventDefault: false };
          }
          if (Math.abs(dx) > Math.abs(dy) + TOUCH_SLOP) {
            // Commit: one-finger horizontal drag.
            gesture = { type: "drag", fingerCount: 1, lastDispatchX: pendingStart.x, lastDispatchY: pendingStart.y };
            var down = { type: "mousedown", x: pendingStart.x, y: pendingStart.y };
            var moved = dragMove(t.x, t.y);
            return { actions: [down].concat(moved.actions), preventDefault: true };
          }
          // Commit: vertical wheel (unchanged behaviour).
          gesture = { type: "wheel", accum: 0, lastX: pendingStart.x, lastY: pendingStart.y };
          return wheelMove(t.x, t.y);
        }
        if (gesture.type === "wheel") {
          return wheelMove(t.x, t.y);
        }
        if (gesture.type === "drag" && gesture.fingerCount === 1) {
          return dragMove(t.x, t.y);
        }
        return { actions: [], preventDefault: false };
      },

      onTouchEnd: function () {
        var actions = endDrag();
        pendingStart = null;
        return { actions: actions };
      }
    };
  }

  if (typeof module !== "undefined" && module.exports) {
    module.exports = { createGestureTracker: createGestureTracker, TOUCH_SLOP: TOUCH_SLOP };
  }

  if (typeof document === "undefined") {
    return; // under Node (docker/web-touch.test.js): export only, no DOM glue
  }

  function target() {
    return document.querySelector(".xterm") ||
      document.getElementById("terminal-container") ||
      document.body;
  }

  function probe() {
    return document.querySelector(".xterm-char-measure-element");
  }

  function rowHeight() {
    var p = probe();
    var h = p && p.offsetHeight;
    return (h && h > 0) ? h : 16;
  }

  function cellWidth() {
    var p = probe();
    var w = p && p.offsetWidth;
    return (w && w > 0) ? w / 32 : 8; // CharSizeService.REPEAT is 32 'W's
  }

  var tracker = createGestureTracker(function () {
    return { rowHeight: rowHeight(), cellWidth: cellWidth() };
  });

  // The element a gesture's synthetic events are dispatched on, fixed for
  // the life of that touch contact (Touch.target doesn't change as a finger
  // moves, so re-deriving it from ev.target on every move would be
  // equivalent, but caching it also means onTouchEnd/onTouchCancel — which
  // carry no touches of their own for a lifted finger — still have somewhere
  // to dispatch the closing mouseup).
  var trackingTarget = null;

  function touchesOf(ev) {
    var out = [];
    for (var i = 0; i < ev.touches.length; i++) {
      out.push({ x: ev.touches[i].clientX, y: ev.touches[i].clientY });
    }
    return out;
  }

  function dispatch(actions) {
    if (!trackingTarget) {
      return;
    }
    for (var i = 0; i < actions.length; i++) {
      var a = actions[i];
      if (a.type === "wheel") {
        trackingTarget.dispatchEvent(new WheelEvent("wheel", {
          deltaY: a.deltaY,
          deltaMode: WheelEvent.DOM_DELTA_PIXEL,
          clientX: a.x,
          clientY: a.y,
          bubbles: true,
          cancelable: true
        }));
      } else if (a.type === "mousedown") {
        trackingTarget.dispatchEvent(new MouseEvent("mousedown", {
          button: 0, buttons: 1, clientX: a.x, clientY: a.y, bubbles: true, cancelable: true
        }));
      } else if (a.type === "mousemove") {
        trackingTarget.dispatchEvent(new MouseEvent("mousemove", {
          button: 0, buttons: 1, clientX: a.x, clientY: a.y, bubbles: true, cancelable: true
        }));
      } else if (a.type === "mouseup") {
        trackingTarget.dispatchEvent(new MouseEvent("mouseup", {
          button: 0, buttons: 0, clientX: a.x, clientY: a.y, bubbles: true, cancelable: true
        }));
      }
    }
  }

  function onTouchStart(ev) {
    var host = target();
    if (!host.contains(ev.target)) {
      trackingTarget = null;
      return;
    }
    trackingTarget = ev.target;
    // touchstart itself is left untouched (no preventDefault) so tap-to-focus
    // and the on-screen keyboard still work — only a subsequent touchmove
    // that commits to a drag or a scroll gets intercepted.
    dispatch(tracker.onTouchStart(touchesOf(ev)).actions);
  }

  function onTouchMove(ev) {
    if (!trackingTarget) {
      return;
    }
    var r = tracker.onTouchMove(touchesOf(ev));
    if (r.preventDefault) {
      ev.preventDefault(); // keep the page from scrolling/rubber-banding/pinch-zooming
    }
    dispatch(r.actions);
  }

  function onTouchEnd() {
    if (!trackingTarget) {
      return;
    }
    dispatch(tracker.onTouchEnd().actions);
    trackingTarget = null;
  }

  document.addEventListener("touchstart", onTouchStart, { passive: true });
  document.addEventListener("touchmove", onTouchMove, { passive: false });
  document.addEventListener("touchend", onTouchEnd, { passive: true });
  document.addEventListener("touchcancel", onTouchEnd, { passive: true });
})();

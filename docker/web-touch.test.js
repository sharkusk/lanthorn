// Pure-function tests for docker/web-touch.js's gesture classifier
// (SQ-1324). No DOM: exercises createGestureTracker directly with synthetic
// touch-point sequences and asserts the {actions, preventDefault} it
// returns, which is everything the DOM glue in web-touch.js turns into real
// dispatchEvent calls.
//
// Run with: node docker/web-touch.test.js
// There is no test runner wired into CI for this (no Node setup step in
// .github/workflows/*.yml as of SQ-1324) — run it locally when touching the
// gesture classifier, the same way docker/test-entrypoint.sh is run by hand
// for entrypoint.sh changes.

var assert = require("assert");
var path = require("path");
var { createGestureTracker } = require(path.join(__dirname, "web-touch.js"));

var CELL_W = 8;
var CELL_H = 16;
function measure() {
  return { rowHeight: CELL_H, cellWidth: CELL_W };
}

var failures = 0;
var passed = 0;

function test(name, fn) {
  try {
    fn();
    passed++;
  } catch (e) {
    failures++;
    console.error("FAIL: " + name);
    console.error("  " + (e && e.stack ? e.stack : e));
  }
}

function pt(x, y) {
  return { x: x, y: y };
}

test("vertical one-finger drag produces wheel deltas quantised to row height, no drag events", function () {
  var t = createGestureTracker(measure);
  var r0 = t.onTouchStart([pt(100, 200)]);
  assert.deepStrictEqual(r0.actions, []);
  assert.strictEqual(r0.preventDefault, false);

  // Move up by 40px (three row-heights of 16px = 48, so 2 rows dispatch, 8px remainder held).
  var r1 = t.onTouchMove([pt(100, 160)]); // dy = 200-160 = 40
  assert.strictEqual(r1.preventDefault, true);
  assert.strictEqual(r1.actions.length, 1);
  assert.strictEqual(r1.actions[0].type, "wheel");
  assert.strictEqual(r1.actions[0].deltaY, 32); // 2 rows * 16px, exact multiple
  assert.strictEqual(r1.actions[0].x, 100);
  assert.strictEqual(r1.actions[0].y, 160);

  // Remaining 8px + another 8px move up = 16px = exactly one more row.
  var r2 = t.onTouchMove([pt(100, 152)]); // dy = 160-152 = 8, accum was 8 -> 16
  assert.strictEqual(r2.actions.length, 1);
  assert.strictEqual(r2.actions[0].deltaY, 16);

  var r3 = t.onTouchEnd();
  assert.deepStrictEqual(r3.actions, []); // wheel gestures never open a mousedown, so nothing to release
});

test("horizontal one-finger drag becomes mousedown/mousemove.../mouseup, never a wheel event", function () {
  var t = createGestureTracker(measure);
  t.onTouchStart([pt(50, 50)]);

  // Move mostly sideways: dx=20, dy=2 — comfortably past TOUCH_SLOP (4).
  var r1 = t.onTouchMove([pt(70, 52)]);
  assert.strictEqual(r1.preventDefault, true);
  assert.ok(r1.actions.length >= 1, "expected at least the mousedown");
  assert.strictEqual(r1.actions[0].type, "mousedown");
  assert.strictEqual(r1.actions[0].x, 50); // dispatched at the ORIGINAL touch-start point
  assert.strictEqual(r1.actions[0].y, 50);
  var rest = r1.actions.slice(1);
  rest.forEach(function (a) { assert.strictEqual(a.type, "mousemove"); });
  // Moved 20px in x (>= CELL_W of 8), so the catch-up mousemove should fire.
  assert.strictEqual(rest.length, 1);
  assert.strictEqual(rest[0].x, 70);
  assert.strictEqual(rest[0].y, 52);

  // Continue the drag: another cell-sized step should produce exactly one more mousemove.
  var r2 = t.onTouchMove([pt(80, 52)]); // dx since last dispatch = 10 >= CELL_W
  assert.strictEqual(r2.actions.length, 1);
  assert.strictEqual(r2.actions[0].type, "mousemove");
  assert.strictEqual(r2.actions[0].x, 80);

  // Sub-cell jitter dispatches nothing.
  var r3 = t.onTouchMove([pt(82, 53)]); // dx=2, dy=1 since last dispatch — below both thresholds
  assert.deepStrictEqual(r3.actions, []);
  assert.strictEqual(r3.preventDefault, true); // still suppress page scrolling mid-drag

  var r4 = t.onTouchEnd();
  assert.strictEqual(r4.actions.length, 1);
  assert.strictEqual(r4.actions[0].type, "mouseup");
  assert.strictEqual(r4.actions[0].x, 80);
  assert.strictEqual(r4.actions[0].y, 52);

  // No wheel events anywhere in this gesture.
  [r1, r2, r3, r4].forEach(function (r) {
    r.actions.forEach(function (a) { assert.notStrictEqual(a.type, "wheel"); });
  });
});

test("two-finger touch (any direction) becomes a mouse drag immediately at touchstart", function () {
  var t = createGestureTracker(measure);
  var r0 = t.onTouchStart([pt(100, 100), pt(120, 100)]); // centroid (110, 100)
  assert.strictEqual(r0.actions.length, 1);
  assert.strictEqual(r0.actions[0].type, "mousedown");
  assert.strictEqual(r0.actions[0].x, 110);
  assert.strictEqual(r0.actions[0].y, 100);

  // Drag both fingers straight down (a direction a 1-finger touch would have
  // classified as a wheel-scroll) — must stay a drag.
  var r1 = t.onTouchMove([pt(100, 120), pt(120, 120)]); // centroid (110, 120), dy=20 >= CELL_H? no, 20>=16 yes
  assert.strictEqual(r1.actions.length, 1);
  assert.strictEqual(r1.actions[0].type, "mousemove");
  assert.strictEqual(r1.actions[0].y, 120);

  var r2 = t.onTouchEnd();
  assert.strictEqual(r2.actions.length, 1);
  assert.strictEqual(r2.actions[0].type, "mouseup");

  [r0, r1, r2].forEach(function (r) {
    r.actions.forEach(function (a) { assert.notStrictEqual(a.type, "wheel"); });
  });
});

test("a tap (touchstart then touchend with no touchmove) dispatches nothing", function () {
  var t = createGestureTracker(measure);
  var r0 = t.onTouchStart([pt(300, 300)]);
  assert.deepStrictEqual(r0.actions, []);
  var r1 = t.onTouchEnd();
  assert.deepStrictEqual(r1.actions, []);
});

test("a vertical gesture that later swerves horizontal stays committed to wheel (classified once)", function () {
  var t = createGestureTracker(measure);
  t.onTouchStart([pt(100, 100)]);
  var r1 = t.onTouchMove([pt(100, 84)]); // pure vertical -> commits to wheel
  assert.strictEqual(r1.actions[0].type, "wheel");

  // Now swerve hard sideways; a fresh classification would call this
  // horizontal, but the gesture is already committed.
  var r2 = t.onTouchMove([pt(140, 84)]); // big dx, dy=0 since last point
  r2.actions.forEach(function (a) { assert.notStrictEqual(a.type, "mousedown"); });
  r2.actions.forEach(function (a) { assert.notStrictEqual(a.type, "mousemove"); });

  var r3 = t.onTouchEnd();
  assert.deepStrictEqual(r3.actions, []); // still no mousedown was ever opened
});

test("a third finger aborts an in-progress two-finger drag with a mouseup", function () {
  var t = createGestureTracker(measure);
  t.onTouchStart([pt(0, 0), pt(20, 0)]);
  var r = t.onTouchMove([pt(0, 0), pt(20, 0), pt(40, 0)]);
  assert.strictEqual(r.actions.length, 1);
  assert.strictEqual(r.actions[0].type, "mouseup");
  assert.strictEqual(r.preventDefault, false);
});

console.log(passed + " passed, " + failures + " failed");
process.exit(failures === 0 ? 0 : 1);

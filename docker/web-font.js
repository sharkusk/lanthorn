// lanthorn's browser font-ready fix, injected into ttyd's page by the
// entrypoint (which sets window.LANTHORN_WEB_FONT and
// window.LANTHORN_WEB_FONT_SIZE first — the same family/size string ttyd's
// own `-t fontFamily=…` / `-t fontSize=…` options got, per
// docker/entrypoint.sh's build_index()).
//
// The bug (SQ-1263): xterm.js measures its character cell exactly once, in
// Terminal.open() (xterm.js src/browser/Terminal.ts:570,
// this._charSizeService.measure()), and re-measures only when the
// `fontFamily` or `fontSize` OPTION changes
// (src/browser/services/CharSizeService.ts:34,
// onMultipleOptionChange(['fontFamily', 'fontSize'], measure)). ttyd's page
// opens the terminal immediately on load. On a cold visit the embedded
// IosevkaTerm Nerd Font Mono webfont (base64, font-display:swap, the
// font-face rule build_index() builds) hasn't finished decoding yet, so
// xterm measures the browser's fallback monospace instead — wider than
// Iosevka — and locks the cell to that size. When the real face swaps in a
// moment later, its narrower glyphs sit inside cells sized for the
// fallback: letters read as spaced far apart. A refresh doesn't reproduce
// it because the face is then already in the browser's font cache and
// ready before open() runs.
//
// Fix: wait for the font to actually be ready (the Font Loading API), then
// force xterm to re-measure. xterm's options service diffs a set against the
// option's current value and skips the change — and the re-measure that
// change would trigger — when the new value is identical to the old one, so
// setting fontFamily back to the same string it already holds is a no-op.
// This toggles through a harmlessly different value first (the family list
// plus ", monospace") and immediately back to the exact original, which
// crosses the diff both times and re-measures for real. window.term.fit()
// (ttyd 1.7.7's own helper, html/src/components/terminal/xterm/index.ts:153)
// then resizes the terminal to the freshly measured cell.
(function () {
  if (window.__lanthornFontFixRan) { return; }
  window.__lanthornFontFixRan = true;

  var family = window.LANTHORN_WEB_FONT;
  var size = window.LANTHORN_WEB_FONT_SIZE;
  if (!family || !size) { return; }

  var TERM_POLL_MS = 100;
  var TERM_POLL_TRIES = 50; // ~5s total

  function remeasure() {
    var term = window.term;
    if (!term || !term.options) { return; }
    var current = term.options.fontFamily;
    term.options.fontFamily = current + ", monospace";
    term.options.fontFamily = current;
    if (typeof term.fit === "function") { term.fit(); }
  }

  // ttyd's own bundle may not have run (and set window.term) yet by the
  // time the fonts resolve — poll briefly rather than assume it has.
  function waitForTermThenRemeasure(triesLeft) {
    if (window.term) { remeasure(); return; }
    if (triesLeft <= 0) { return; }
    setTimeout(function () { waitForTermThenRemeasure(triesLeft - 1); }, TERM_POLL_MS);
  }

  function onFontsSettled() {
    waitForTermThenRemeasure(TERM_POLL_TRIES);
  }

  if (window.document && document.fonts && document.fonts.load) {
    var spec = size + 'px "' + family + '"';
    Promise.all([
      document.fonts.load(spec),
      document.fonts.load("bold " + spec)
    ]).then(onFontsSettled, onFontsSettled);
  } else {
    // Old WebKit with no Font Loading API: nothing to await, so
    // approximate — re-measure once the page finishes loading and once
    // more shortly after, by which point a swapped-in webfont has usually
    // settled.
    window.addEventListener("load", function () {
      onFontsSettled();
      setTimeout(onFontsSettled, 1000);
    });
  }
})();

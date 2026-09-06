// lanthorn's browser audio, injected into ttyd's page by the entrypoint
// (which sets window.LANTHORN_WEB_AUDIO_PORT first). See
// crates/audio-relay/src/lib.rs for the other end.
//
// Order is the whole trick: the audio socket is opened BEFORE ttyd's script
// opens the terminal socket, because the relay creates the session's FIFO on
// connect and the per-connection wrapper looks for it when lanthorn starts.
//
// The session id is not this file's any more — docker/web-session.js owns it,
// runs first, persists it in localStorage and puts it in the URL as
// `?arg=--web-session=ID` (SQ-1323). Minting one here as well would have given
// the audio socket a different name from the one the game session is filed
// under. With no id there is no FIFO to name, and the page is simply silent.
//
// AND THE SOCKET COMES BACK (SQ-1328). The game outlives a dropped connection
// and so does its FIFO, so the audio socket is reopened with the SAME id after
// a drop and the sound picks up where the game is now — not where it was. Each
// attach opens with a fresh header frame, and the queue is emptied when one
// arrives: whatever was still buffered belongs to a connection that is over.
(function () {
  var session = window.LANTHORN_SESSION_ID;
  if (typeof session !== "string" || !/^[A-Za-z0-9_-]{8,64}$/.test(session)) {
    return;
  }
  var port = window.LANTHORN_WEB_AUDIO_PORT || 7682;
  var scheme = window.location.protocol === "https:" ? "wss" : "ws";
  var url = scheme + "://" + window.location.hostname + ":" + port + "/audio/" + session;
  var ws = null;
  var retry = 500;                      // ms, doubling to RETRY_MAX
  var RETRY_MAX = 8000;
  var closing = false;

  var rate = 44100, channels = 2;
  var ctx = null, node = null;
  var queue = [], queued = 0;           // Int16Array chunks, interleaved
  var CAP_SECONDS = 1;                  // drop old audio past this much backlog
  var PRIME_SECONDS = 0.1;              // gather this much before playing, and after running dry
  var primed = false;

  function ensureContext() {
    if (!ctx) {
      var AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) { return; }
      try { ctx = new AC({ sampleRate: rate }); } catch (e) { ctx = new AC(); }
      node = ctx.createScriptProcessor(4096, 0, channels);
      node.onaudioprocess = fill;
      node.connect(ctx.destination);
    }
    if (ctx.state === "suspended") { ctx.resume(); }
  }
  // Browsers only start audio after a gesture; the first key or click the
  // terminal gets is that gesture.
  ["keydown", "mousedown", "touchstart"].forEach(function (ev) {
    window.addEventListener(ev, ensureContext, { capture: true, passive: true });
  });

  function onMessage(e) {
    if (typeof e.data === "string") {
      try {
        var h = JSON.parse(e.data);
        rate = h.rate || rate;
        channels = h.channels || channels;
      } catch (err) { /* not ours */ }
      // A header opens every attachment, this one included: start the decoder
      // clean rather than playing out a queue from a connection that is over.
      queue = [];
      queued = 0;
      primed = false;
      return;
    }
    var pcm = new Int16Array(e.data);
    queue.push(pcm);
    queued += pcm.length;
    var cap = rate * channels * CAP_SECONDS;
    while (queued > cap && queue.length > 1) { queued -= queue.shift().length; }
  }

  function connect() {
    try {
      ws = new WebSocket(url);
    } catch (e) {
      ws = null;
      reconnect();
      return;
    }
    ws.binaryType = "arraybuffer";
    ws.onmessage = onMessage;
    ws.onopen = function () { retry = 500; };
    // Both ends of a dropped connection: the relay is still draining the game's
    // FIFO for this id, so coming back is a matter of asking again.
    ws.onclose = reconnect;
    ws.onerror = function () { /* onclose follows */ };
  }

  function reconnect() {
    if (closing) { return; }
    window.setTimeout(connect, retry);
    retry = Math.min(retry * 2, RETRY_MAX);
  }

  // A page that is going away must not spend its last moments reconnecting —
  // but a page merely FROZEN into the back/forward cache (`persisted`, which is
  // what a backgrounded tab on a phone does) is coming back, and its timers
  // come back with it, so that one is left alone to reconnect on its own.
  window.addEventListener("pagehide", function (e) {
    if (e.persisted) { return; }
    closing = true;
    if (ws) { try { ws.close(); } catch (err) { /* already gone */ } }
  });

  connect();

  function fill(ev) {
    var out = [];
    for (var c = 0; c < channels; c++) { out.push(ev.outputBuffer.getChannelData(c)); }
    var n = ev.outputBuffer.length;
    var pos = 0;
    if (!primed && queued >= rate * channels * PRIME_SECONDS) { primed = true; }
    if (!queue.length) { primed = false; }
    while (primed && pos < n && queue.length) {
      var head = queue[0];
      var frames = Math.min((head.length / channels) | 0, n - pos);
      for (var f = 0; f < frames; f++) {
        for (var ch = 0; ch < channels; ch++) { out[ch][pos + f] = head[f * channels + ch] / 32768; }
      }
      pos += frames;
      if (frames * channels >= head.length) {
        queue.shift();
        queued -= head.length;
      } else {
        queue[0] = head.subarray(frames * channels);
        queued -= frames * channels;
      }
    }
    for (; pos < n; pos++) {
      for (var z = 0; z < channels; z++) { out[z][pos] = 0; }
    }
  }
})();

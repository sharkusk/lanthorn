// lanthorn's browser audio, injected into ttyd's page by the entrypoint
// (which sets window.LANTHORN_WEB_AUDIO_PORT first). See
// crates/audio-relay/src/lib.rs for the other end.
//
// Order is the whole trick: the audio socket is opened BEFORE ttyd's script
// opens the terminal socket, because the relay creates the session's FIFO on
// connect and the per-connection wrapper looks for it when lanthorn starts.
// The session id travels to that wrapper in the URL, as `?arg=--web-audio=ID`,
// which ttyd (--url-arg) appends to the command line; a page without the id
// rewrites its own URL and reloads once.
(function () {
  var TAG = "--web-audio=";
  var params = new URLSearchParams(window.location.search);
  var have = params.getAll("arg").filter(function (a) { return a.indexOf(TAG) === 0; })[0];
  if (!have) {
    var alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    var id = "";
    var rnd = new Uint8Array(16);
    (window.crypto || window.msCrypto).getRandomValues(rnd);
    for (var i = 0; i < 16; i++) { id += alphabet[rnd[i] % alphabet.length]; }
    params.append("arg", TAG + id);
    window.location.replace(window.location.pathname + "?" + params.toString() + window.location.hash);
    return;
  }
  var session = have.slice(TAG.length);
  var port = window.LANTHORN_WEB_AUDIO_PORT || 7682;
  var scheme = window.location.protocol === "https:" ? "wss" : "ws";
  var ws;
  try {
    ws = new WebSocket(scheme + "://" + window.location.hostname + ":" + port + "/audio/" + session);
  } catch (e) {
    return;
  }
  ws.binaryType = "arraybuffer";

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

  ws.onmessage = function (e) {
    if (typeof e.data === "string") {
      try {
        var h = JSON.parse(e.data);
        rate = h.rate || rate;
        channels = h.channels || channels;
      } catch (err) { /* not ours */ }
      return;
    }
    var pcm = new Int16Array(e.data);
    queue.push(pcm);
    queued += pcm.length;
    var cap = rate * channels * CAP_SECONDS;
    while (queued > cap && queue.length > 1) { queued -= queue.shift().length; }
  };

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

// The browser's lanthorn session id, injected into ttyd's page by the
// entrypoint. Everything that has to survive a dropped connection hangs off
// this one string (SQ-1323).
//
// WHY IT IS PERSISTED. ttyd spawns one process per websocket, so without a
// stable name a reconnect is a stranger: a new process, a new game, and the
// half hour you just played is somewhere in a save file you have to go and
// find. With one, `lanthorn-serve-session` can hand the reconnect back to the
// game that is STILL RUNNING under the same name (see `dtach -A` there), and a
// tab that dropped on a train picks up mid-sentence.
//
// localStorage, keyed per origin, because that is the only store that outlives
// a reload, a crashed tab and a closed laptop while staying private to the one
// browser. A private window, a cleared site list or a browser that refuses
// storage outright all throw here; that is not an error, it is simply a visitor
// who gets a fresh session each time, which is exactly what everyone got before
// this file existed.
//
// This runs BEFORE web-audio.js, which needs the id to open ws://…/audio/<id>
// and would otherwise mint a second one. It travels to the per-connection
// wrapper in the URL as `?arg=--web-session=ID`, which ttyd (--url-arg) appends
// to the command line; a page without the id rewrites its own URL and reloads
// once.
(function () {
  var TAG = "--web-session=";
  var KEY = "lanthorn.session";

  // The same rule the wrapper and the audio relay apply, because the id names a
  // FILE at both ends — a FIFO and a unix socket. A page that put anything else
  // in the URL gets it thrown away here rather than at the far end.
  function valid(id) {
    return typeof id === "string" && /^[A-Za-z0-9_-]{8,64}$/.test(id);
  }

  function stored() {
    try {
      return window.localStorage.getItem(KEY);
    } catch (e) {
      return null;
    }
  }

  function store(id) {
    try {
      window.localStorage.setItem(KEY, id);
    } catch (e) {
      /* private mode, or storage refused: this visit is simply not resumable */
    }
  }

  function mint() {
    var alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    var rnd = new Uint8Array(16);
    var id = "";
    (window.crypto || window.msCrypto).getRandomValues(rnd);
    for (var i = 0; i < 16; i++) {
      id += alphabet[rnd[i] % alphabet.length];
    }
    return id;
  }

  var params = new URLSearchParams(window.location.search);
  var args = params.getAll("arg");
  var have = args.filter(function (a) { return a.indexOf(TAG) === 0; })[0];

  if (have && valid(have.slice(TAG.length))) {
    // The URL already names a session. It wins over the store — a link someone
    // pasted, or the reload this file itself just did — and becomes this
    // browser's remembered id, so the next cold open comes back to it.
    window.LANTHORN_SESSION_ID = have.slice(TAG.length);
    store(window.LANTHORN_SESSION_ID);
    return;
  }

  var id = stored();
  if (!valid(id)) {
    id = mint();
  }
  store(id);

  // Rebuild the query with every other `arg` intact and exactly one session
  // argument — a malformed one that was already there is dropped rather than
  // carried, or the far end would see two and take the wrong one.
  var next = new URLSearchParams();
  params.forEach(function (value, key) {
    if (key !== "arg") {
      next.append(key, value);
    }
  });
  args.forEach(function (a) {
    if (a.indexOf(TAG) !== 0) {
      next.append("arg", a);
    }
  });
  next.append("arg", TAG + id);
  window.location.replace(window.location.pathname + "?" + next.toString() + window.location.hash);
})();

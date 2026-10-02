// Tests for docker/web-audio.js's choice of audio socket URL (SQ-1318).
// Run with: node docker/web-audio.test.js
var assert = require("assert");
var path = require("path");
var { audioUrl } = require(path.join(__dirname, "web-audio.js"));

var id = "abcdefgh12345678";
var http = { protocol: "http:", hostname: "nas.lan", host: "nas.lan:8080" };
var https = { protocol: "https:", hostname: "play.example.com", host: "play.example.com" };

// Normal mode: the relay's own port on the page's host.
assert.strictEqual(audioUrl(http, 7682, id, undefined), "ws://nas.lan:7682/audio/" + id);
assert.strictEqual(audioUrl(https, 9000, id, undefined), "wss://play.example.com:9000/audio/" + id);
assert.strictEqual(audioUrl(http, 7682, id, ""), "ws://nas.lan:7682/audio/" + id);

// Proxy mode: the page's own origin (host includes any port), at the path.
assert.strictEqual(audioUrl(https, 7682, id, "/lanthorn-audio/"), "wss://play.example.com/lanthorn-audio/" + id);
assert.strictEqual(audioUrl(http, 7682, id, "/lanthorn-audio/"), "ws://nas.lan:8080/lanthorn-audio/" + id);

console.log("web-audio.test.js: all checks passed");

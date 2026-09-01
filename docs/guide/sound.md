# Sound

For anyone wondering what makes noise in lanthorn, where it comes from, and
what to do when you're playing over SSH and hear nothing at all.

## Z-machine bleeps and samples

The Z-machine has two sounds built into it — a high bleep and a low one —
which lanthorn synthesizes for real, no sample needed. Above those, a Blorb's
`Snd ` resources play as sampled audio (AIFF, Ogg, or ProTracker MOD). Sound resources come from the story file itself if it's a
Blorb, or from a sibling `.blb`/`.blorb` sitting beside it. On every bleep the
story pane's border also flashes in a themeable colour — a nice touch when
sound is on, and the only cue you get when it's off.

## Straight off the original floppy

*The Lurking Horror* and *Sherlock* shipped sampled sound effects on their
release disks years before Blorb existed. Mount one of those disks and
lanthorn plays them straight off the media — no `.blb`, no conversion. **A
release disk's own sound always wins over a `.blb` filed beside it**, the
same rule artwork follows and for the same reason: the disk is what Infocom
actually pressed. The pitch travels with it too — *Sherlock*'s heartbeat
genuinely beats at three different speeds from one recording, because
lanthorn reproduces the bend the original interpreter applied.

## Glulx

Glulx games use Glk sound channels to play a Blorb's AIFF/Ogg/MOD resources,
with per-channel volume, gradual volume ramps, and sound-finished
notifications — so music and effects behave exactly as the author wired
them.

## Turning it on and off

Sound is on by default. `enable_sound` and `volume` (0–100) live in
`config.toml`; toggle sound mid-game with `/toggle-sound` or from the
settings screen (`/open-settings`), and adjust volume with `/volume <0-100>`.
`--sound off` mutes a single run without touching your saved setting.
`/play-sound [n]` lists the Blorb sound resources, or fires one on demand,
which is handy for checking that you can hear anything at all.

## Over SSH

Sound always plays on the local device lanthorn is running on. If you SSH
into a remote box and run lanthorn there, the sound comes out *that*
machine's speakers, not yours — lanthorn has no built-in network-audio
feature. Two options: forward audio yourself with PulseAudio/PipeWire or a
similar tool, or run lanthorn in Docker's browser mode instead, where sound
reaches the browser tab over its own channel. Both are covered in full in
[remote sound](../internals/remote-sound.md); the browser route is also
covered from the player's side in [play in a browser](play-in-a-browser.md).

## Going deeper

- [Remote sound](../internals/remote-sound.md) — routing audio off a remote or headless box
- [Play in a browser](play-in-a-browser.md) — sound reaches the browser too
- [Interpreter](../internals/interpreter.md) — the full sound implementation, format by format

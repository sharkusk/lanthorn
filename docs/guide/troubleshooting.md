# Troubleshooting

Answers to the things players actually run into.

### The map (or a border icon) shows boxes or question marks instead of glyphs

Your terminal's font is missing some of the characters lanthorn draws with.
Any mono-space [Nerd Font](https://www.nerdfonts.com) carries the full set.
On first launch lanthorn shows you two sample rows and asks which one draws
correctly, then sets everything — arrows, portals, pane icons — from your
answer; run it again any time with `/run-font-check` after changing fonts.
See [missing or corrupted glyphs](../internals/glyphs.md) for exactly which Unicode
blocks are involved, and for the one modern ask (diagonal corner stubs) that
even some otherwise-good fonts skip.

### I get no in-game pictures over SSH, or inside tmux/screen

Pixel graphics need the terminal itself to speak a graphics protocol, and
that request has to survive whatever sits between lanthorn and your screen.
tmux and GNU screen don't pass a graphics protocol through unless you've
turned on passthrough for it, so lanthorn falls back to Unicode half-blocks
— a real renderer in its own right, not a broken image, and it's what an SSH
session with no protocol support gets too. `/dump-terminal` shows exactly
what lanthorn detected versus assumed about the terminal you're on, and is
the thing to attach to a bug report if art still looks wrong.

### A story seems to have hung

It probably hasn't. If a turn runs for ten seconds with no sign of asking
for input, lanthorn assumes it's caught in a runaway loop, aborts that turn
as a recoverable fault rather than freezing the app, and keeps the map,
scrollback and your ability to quit working throughout. A record lands in
`~/.lanthorn/crash.log` either way.

### On Windows: I closed the console and lost my progress

Closing the window (rather than quitting from inside lanthorn) kills the
process before it can save anything — save first with `Ctrl+S`, or quit
through the app itself. Quitting normally is unaffected. Also on Windows
only: changing your terminal's font size mid-session isn't picked up until
you restart lanthorn, because Windows has no way for lanthorn to re-measure
the terminal on the fly. macOS and Linux pick the change up immediately.

### My story file disappeared after I opened it in Frotz

Frotz relocates the story file it opens rather than leaving it in place. If
a game vanishes from where you expect it, check wherever Frotz moved it to
— lanthorn didn't touch it.

### Where does lanthorn keep everything?

Config and styles: `~/.lanthorn/config.toml` and `~/.lanthorn/style.toml`.
Saves and per-game sidecars: `~/.lanthorn/saves/<story>.save/`, one
directory per game (a disk image keys on its release and serial rather than
a filename, so different builds of the same story never collide). Crash
records: `~/.lanthorn/crash.log`. Terminal diagnostics:
`~/.lanthorn/dump-terminal.log`. `--user-dir` moves the whole `.lanthorn`
tree; `--data-dir` moves just the saves. See
[the persistence model](../internals/persistence.md) for the full layout.

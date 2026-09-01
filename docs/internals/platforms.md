# Platform notes

> For players, the short version is in [the guide](../guide/getting-started.md).

lanthorn runs on macOS, Linux and Windows, and the same story plays the same way
on all three. What differs is what the *terminal* can be asked and what the *OS*
lets a process do on the way out — and those two axes fail independently, which
is why a better terminal fixes some of the entries below and none of the others.

Everything here is a limitation a user can observe. Internals live in
[architecture.md](architecture.md).

## Windows

Windows is the platform with the most entries, for one structural reason: three
of the mechanisms lanthorn leans on elsewhere — POSIX signals, the `TIOCGWINSZ`
ioctl, and a controlling terminal that outlives the process — either do not exist
or do not mean the same thing.

### Closing the console window loses unsaved progress

`install_termination_handlers` is Unix-only. On macOS and Linux, SIGTERM/SIGHUP/
SIGINT/SIGQUIT set a flag that the main loop observes at a safe point, so an
out-of-band kill or a closing controlling terminal still restores the screen and
runs the normal shutdown path.

Windows has no equivalent registered. Closing the console window terminates the
process with no chance to flush, so anything since the last save is lost. The
console resets itself on exit, so the *terminal* is left clean — it is the
*state* that is not.

**Workaround:** save before closing the window (Ctrl-S for a host Save State),
or quit through the app rather than the window's close button. Quitting normally
is unaffected.

This is not terminal-dependent: it behaves the same in Windows Terminal, conhost,
WezTerm and everything else.

### A font-size change mid-session is not noticed

**Fixed on macOS and Linux; still open on Windows.**

lanthorn asks the terminal for its cell size at startup, with a query that writes
an escape and reads the reply — genuinely delicate to repeat once the app is in
raw mode and owns the keyboard. So the answer used to be held for the whole
session: change the font size while a story was open and every graphics fit kept
using the launch cell.

The absolute size does not matter — the geometry multiplies and divides by it, so
a uniform error cancels. What survives is the *aspect ratio*, and that genuinely
moves between adjacent font sizes: a cell is `round(advance x px)` by
`round(line_height x px)`, and the two round at different rates, so even a font
whose design ratio is exactly 1:2 produces real cells ranging from 1.75 to 2.25.
The visible result is artwork that looks slightly stretched until you relaunch.

On macOS and Linux the cell is now re-derived from a `TIOCGWINSZ` syscall on
every resize — one ioctl, no escape written, nothing for the input loop to race
with — and everything fitted against the old cell is thrown away when it moves.
Changing your terminal's font size is a resize, so this happens on the same
keystroke that causes it.

Windows has no such fallback: the ioctl does not exist and the console API
reports no per-cell pixel geometry, so the launch value stands for the session
there and a fix needs a re-issued terminal query.

**Workaround (Windows only):** restart lanthorn after changing your terminal font
size.

### Without a terminal that answers, the cell size is a guess

lanthorn asks with `CSI 16 t`. If the terminal answers, all is well. If it does
not, macOS and Linux fall back to a `TIOCGWINSZ` syscall; **Windows has no
fallback at all** and lands on a hardcoded 10x20.

A 10x20 guess is not obviously wrong — it is a plausible cell, so nothing looks
broken — but every graphics geometry decision is then computed against a cell
your terminal may not have.

**This one a better terminal does fix.** Windows Terminal and WezTerm both answer
the query, so the fallback never runs. It matters on bare consoles and on older
terminals that ignore `CSI 16 t`.

## Graphics protocols by platform

| Protocol | Terminals | Platforms |
|---|---|---|
| Kitty graphics | kitty, Ghostty, WezTerm | Linux, macOS, Windows (WezTerm) |
| iTerm2 inline images | iTerm2 | macOS |
| Sixel | Windows Terminal 1.22+, foot, xterm | Windows 11, Linux, macOS |
| Unicode half-blocks | anything, including SSH and tmux | everywhere |

kitty the terminal is Linux and macOS only; on Windows the kitty *protocol* is
reached through WezTerm. Sixel on Windows needs Windows Terminal 1.22 or newer,
which in practice means Windows 11.

Half-blocks always works, needs nothing from the terminal beyond colour, and is
what everything degrades to. It is also, at a small enough font, a genuinely
high-resolution renderer rather than a consolation prize — see
[v6-graphics.md](v6-graphics.md).

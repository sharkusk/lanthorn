# Playing

For anyone with a story open who wants to know what lanthorn adds on top of
typing commands.

## Typing, and what fills in around it

The prompt is the story's own — type a command and press `Enter`, exactly as
in any interpreter. What lanthorn adds sits *around* that line and never
steals it: **Tab** completes from the things actually in front of you first
(standing in the Living Room, `lan` offers `lantern`), then words the story
has just used, then its whole dictionary — checked against the story's own
parser, so you're never offered a word it would refuse. The rest of the
match ghosts in dim text right after the caret; `Tab`/`Shift-Tab` cycle
candidates, `→` at the end of the line takes the one on offer.

Type `/` on an empty line and a fuzzy palette opens over every command
lanthorn knows — the fastest way to find out what's there without
memorizing anything. `↑`/`↓` at the prompt recall earlier commands,
shell-style. See [keys](../reference/keys.md) and
[commands](../reference/commands.md) for the full lists.

## The command band

Press the `≡` control on the story pane's border (or `/open-command-band`)
and a dock opens along the bottom that builds a command by pointing instead
of typing. It reads the running story's own grammar — every verb it
actually accepts, alphabetically — and fills in object columns for what
you can see and what you're carrying, live, as you play. Click a verb, then
an object, and the words land on your prompt; **Enter** still sends
whatever's actually written there, so nothing fires on its own except the
one-click quick actions.

Those quick actions draw as a compass rose when the band is wide enough — 
`↑` `◉` `◎` `↓` for up, in, out and down alongside the eight compass
points — and each is a single click that submits at once, no `Enter`
needed. Typing always wins over the band: letters and Backspace go straight
to the prompt whether the band is open or not, and the band only claims
column navigation (`Tab`/`Shift-Tab` to move between columns, `↑`/`↓` to
highlight a row).

## The word reveal

Press the `◈` control (or `/reveal-words`) and every word already on screen
that this story's parser would accept lights up for a few seconds, right
over the prose, without moving a line of it. It's the answer to the oldest
frustration in the genre — a room description names a dozen things and the
game only implements two — and it's telling you what the *dictionary*
knows, not a promise that any of it is within reach.

The command band carries the same idea as a running list: under what's
actually here, dimmed, sit the nouns the story has *printed* this session —
things a room describes rather than things it hands you directly. Newest
first, and it keeps accumulating, so something named forty turns ago is
still one click away.

## Reading back

**Left-drag** across the story pane selects transcript text; let go and it
lands on your system clipboard, even over SSH — lanthorn copies through the
terminal's own OSC 52 escape rather than a clipboard library, so it works
wherever your terminal does.

`/search-transcript <query>` highlights every match and jumps to the most
recent; `n`/`N` step through the rest. `/filter-transcript` narrows the view
to just the game's own output, just lanthorn's, or both. `/export-transcript`
writes what's on screen out to a text file in the story's own save
directory.

When a turn prints more than fits the pane — a long room description, a
hint page — lanthorn stops at the first full screen with a `[MORE]` bar
instead of scrolling straight past it, exactly like the original Infocom
interpreters. Any key pages onward, and nothing reaches the game until
you've caught up.

## Hints

`/open-hints` lays a companion *InvisiClues* file over the story pane — its
own topic menu on top, the clue text below, driven with the arrow keys and
whatever it prompts for. lanthorn finds a hint file sitting beside the
story automatically, or you can fetch one for free from the story picker
before you even start playing (see [Getting started](getting-started.md)).

## Going deeper

The full key bindings and command list are in
[keys](../reference/keys.md) and [commands](../reference/commands.md).
Everything on the pane borders — what each glyph means, how the toggles
remember your choices per game — is in
[the interface notes](../internals/interface.md), and the guiding-light
suggestions and font-icon setup are in
[customization](../internals/customization.md).

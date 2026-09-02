<!-- generated from crates/app/src/slash.rs (slash::COMMANDS) by docs_reference; do not edit by hand -->
# Command reference

Every slash command, grouped the way `/help` groups them. Type any of these after the command prefix (`/` by default); a key bound to one is in [keys.md](keys.md).

| Category | Command | Description |
|---|---|---|
| Game | `save-state [name]` | save an emulator Save State, optionally to a named slot |
| Game | `restore-state [name]` | restore an emulator Save State — bare opens the saves dialog to pick one; a name restores that slot directly |
| Game | `reset-game [map] [data]` | restart the game — bare opens the options dialog; 'map' also clears the map, 'data' deletes the game's saved progress/cache so it starts fresh |
| Game | `quit` | exit lanthorn |
| Game | `quit-to-library` | exit the current story and return to the story library |
| Game | `open-hints` | open the hints panel |
| Game | `open-history` | open the rewind/replay history |
| Game | `toggle-command-panel` | open or close the command panel; remembered per story |
| Game | `cycle-panel` | cycle command panel → inventory panel → none |
| Game | `toggle-timed-input` | toggle honoring the game's timed-input timers |
| Game | `toggle-sound` | toggle audio playback (bleeps + sampled sounds) |
| Game | `volume <0-100>` | set the master audio volume (0-100) |
| Game | `play-sound [n]` | diagnostic: list Snd resources, or play resource n |
| Map | `pan-map <dx> <dy>` | pan the map by dx columns and dy rows |
| Map | `zoom-map in|out|reset|<n>` | zoom the map in/out, reset, or step by signed n |
| Map | `center-map` | re-center the map on the selected room, or the current one |
| Map | `tidy-map` | re-run the layout tidy |
| Map | `cycle-layer next|prev|<n>` | switch map layer; n is a signed delta |
| Map | `select-room next|prev` | move the room selection |
| Map | `rename-room` | rename the selected room |
| Map | `rename-layer` | rename the current layer |
| Map | `edit-notes` | edit the selected room's notes |
| Map | `delete-connection` | delete the selected connection |
| Map | `relabel-edge` | relabel the selected edge |
| Map | `move-region [new|parent|layer] [direction]` | re-home the selected room's region onto a fresh layer, its parent, or any named layer; bare picks both when only one choice is possible |
| Map | `toggle-room-panel` | open or close the room panel under the map |
| Map | `toggle-inspector` | show the room panel's diagnostics view (flips back to info when open) |
| Map | `load-map <path>` | load a standalone map file into the current session |
| Map | `toggle-room-numbers` | toggle room-number labels |
| Map | `view-map [drawn|matrix]` | how the active layer draws: bare cycles, a name sets it |
| Map | `mark-maze-layer` | flag the active layer as a maze (defaults it to the matrix view) |
| Map | `toggle-alignment` | toggle alignment guides |
| Map | `toggle-portal-labels` | toggle portal labels |
| View | `toggle-map` | show or hide the map panel; persisted per-game |
| View | `toggle-focus` | switch focus between panes |
| View | `toggle-inventory-panel` | open or close the inventory panel; remembered per story |
| View | `toggle-status-bar` | toggle the status/score bar |
| View | `resize-panes` | enter interactive pane-resize mode |
| View | `reset-pane-size` | reset all pane sizes to their defaults |
| Transcript | `search-transcript [query]` | search the transcript; no query repeats the last search |
| Transcript | `filter-transcript story|meta|both` | filter the transcript by category |
| Transcript | `export-transcript [file]` | export the visible transcript; default path when omitted |
| Style | `open-settings` | open the global settings screen |
| Style | `reload-style` | reload style.toml from disk |
| Style | `toggle-watch` | toggle live style-file watching |
| Style | `print-colors [color]` | print the current color scheme (color = actual colors) |
| Style | `set-game-colours on|off|auto` | force this game's own colours on/off (auto follows garglk.ini/global); persisted per-game |
| Style | `set-v6-render [hybrid|raster|extended|auto]` | switch this game's v6 render mode — bare cycles hybrid → raster → extended, auto inherits the global setting; persisted per-game |
| Style | `set-v6-pixel-lock [on|off|auto]` | lock v6 art to a whole number of device pixels per art pixel — bare toggles, auto inherits the global setting; persisted per-game |
| Style | `set-guidance [on|off|auto]` | Lanthorn's Guiding Light: help while you play, marked in the margin — bare toggles, auto inherits the global setting; persisted per-game |
| Style | `set-return-probe [on|off|auto]` | after a move, look for the way back in a silent copy of the game and put it on the map — bare toggles, auto inherits the global setting; persisted per-game |
| Style | `reveal-words` | light the nouns and named things on screen this story knows, for a few seconds — under the Guiding Light's switch |
| Style | `run-font-check` | ask which of two glyph rows your terminal's font draws properly, and set the map's arrow, portal and Guiding Light icons from the answer (writes style.toml) |
| Style | `set-game-borders on|off|auto` | show this game's Glk window borders (on), or render borderless/abutting (off); auto = default (on); persisted per-game |
| Export | `export-svg [file]` | export the map as SVG; default path when omitted |
| Export | `export-dot [file]` | export the map as Graphviz DOT; default path when omitted |
| Export | `export-map [file]` | dump the map structure; default path when omitted |
| Animation | `animate-tidy` | animate a tidy pass |
| Animation | `anim-step forward|back` | step the animation one frame |
| Animation | `anim-play` | toggle animation play/pause |
| Animation | `anim-exit` | exit the animation view |
| Help | `dump-windows` | dump the last game frame's window layout, here and to ~/.lanthorn/dump-windows.log |
| Help | `dump-cells` | write the last frame's cells — glyphs, colours and attributes — to ~/.lanthorn/dump-cells.log |
| Help | `dump-terminal` | dump this terminal's detected protocol, cell size, capabilities and traffic — here and to ~/.lanthorn/dump-terminal.log |
| Help | `debug` | toggle the Z-machine debug inspector pane |
| Help | `trace [sections|all|none]` | toggle debug-trace sections (screen, map, hostio, v6) written to trace.log; no arg shows current state |
| Help | `dump-notifications` | print the notification history to the transcript, in case a toast was missed |
| Help | `help [command]` | list all commands by category; with a name, show one command's detail |
| Library | `move-selection <dx> <dy>` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Library | `page-selection <n>` | move the browser's selection by n pages |
| Library | `half-page-selection <n>` | move the browser's selection by half a page (vim Ctrl-U/Ctrl-D) |
| Library | `select-edge first|last` | jump the browser's selection to the first or last story |
| Library | `play-story` | launch the selected story |
| Library | `open-launch-options` | open the launch-options dialog for the selected story |
| Library | `open-story-menu` | open the per-story menu beside the selected story |
| Library | `show-browser-keys` | show the story browser's key reference |
| Library | `toggle-info-panel` | open or close the browser's story info panel |
| Library | `toggle-gallery` | switch the browser between the story list and the cover gallery |
| Library | `fetch-story` | re-fetch the selected story's IFDB metadata, ignoring the cache |
| Library | `refresh-library` | fetch IFDB metadata for every story that is missing or stale |
| Library | `set-ifdb-url` | point the selected story at an IFDB page by hand |
| Library | `open-url` | download a story from a URL into this library and open it |
| Library | `search-ifdb` | search IFDB by title or author and download a story into this directory |
| Library | `download-hints` | download a matching InvisiClues hint file for the selected story |
| Library | `sort-library` | cycle the browser's sort column, keeping the direction |
| Library | `reverse-sort` | reverse the browser's sort direction, keeping the column |
| Library | `find-story` | type to filter the whole library by title, author, filename or folder |
| Library | `parent-folder` | leave the current library folder for the one above it |
| Library | `quit-browser` | leave the story browser |
| Library | `cancel-browser` | cancel a running fetch, or leave the browser when nothing is in flight |

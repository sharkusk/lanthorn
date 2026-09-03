<!-- generated from crates/app/src/keymap.rs (keymap::KeyMap::default) by docs_reference; do not edit by hand -->
# Key reference

The built-in key bindings, one row per binding — several keys may reach the same command. Rebind any of these under `[keymap.*]` in `config.toml`; see [config.md](config.md).

| Context | Key | Command | Description |
|---|---|---|---|
| Global | `Tab` | `toggle-focus` | switch focus between panes |
| Global | `Ctrl+S` | `save-state` | save an emulator Save State, optionally to a named slot |
| Global | `Ctrl+R` | `restore-state` | restore an emulator Save State — bare opens the saves dialog to pick one; a name restores that slot directly |
| Anim | `H` | `pan-map -1 0` | pan the map by dx columns and dy rows |
| Anim | `L` | `pan-map 1 0` | pan the map by dx columns and dy rows |
| Anim | `K` | `pan-map 0 -1` | pan the map by dx columns and dy rows |
| Anim | `J` | `pan-map 0 1` | pan the map by dx columns and dy rows |
| Anim | `Shift+←` | `pan-map -1 0` | pan the map by dx columns and dy rows |
| Anim | `Shift+→` | `pan-map 1 0` | pan the map by dx columns and dy rows |
| Anim | `Shift+↑` | `pan-map 0 -1` | pan the map by dx columns and dy rows |
| Anim | `Shift+↓` | `pan-map 0 1` | pan the map by dx columns and dy rows |
| Anim | `+` | `zoom-map in` | zoom the map in/out, reset, or step by signed n |
| Anim | `=` | `zoom-map in` | zoom the map in/out, reset, or step by signed n |
| Anim | `-` | `zoom-map out` | zoom the map in/out, reset, or step by signed n |
| Anim | `←` | `anim-step back` | step the animation one frame |
| Anim | `→` | `anim-step forward` | step the animation one frame |
| Anim | `Space` | `anim-play` | toggle animation play/pause |
| Anim | `Esc` | `anim-exit` | exit the animation view |
| Anim | `Enter` | `anim-exit` | exit the animation view |
| Browser | `↑` | `move-selection 0 -1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `K` | `move-selection 0 -1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `↓` | `move-selection 0 1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `J` | `move-selection 0 1` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `←` | `move-selection -1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `H` | `move-selection -1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `→` | `move-selection 1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `L` | `move-selection 1 0` | move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery) |
| Browser | `PgUp` | `page-selection -1` | move the browser's selection by n pages |
| Browser | `PgDn` | `page-selection 1` | move the browser's selection by n pages |
| Browser | `Ctrl+U` | `half-page-selection -1` | move the browser's selection by half a page (vim Ctrl-U/Ctrl-D) |
| Browser | `Ctrl+D` | `half-page-selection 1` | move the browser's selection by half a page (vim Ctrl-U/Ctrl-D) |
| Browser | `Home` | `select-edge first` | jump the browser's selection to the first or last story |
| Browser | `End` | `select-edge last` | jump the browser's selection to the first or last story |
| Browser | `Enter` | `play-story` | launch the selected story |
| Browser | `O` | `open-launch-options` | open the launch-options dialog for the selected story |
| Browser | `Shift+Enter` | `open-launch-options` | open the launch-options dialog for the selected story |
| Browser | `Space` | `open-story-menu` | open the per-story menu beside the selected story |
| Browser | `?` | `show-browser-keys` | show the story browser's key reference |
| Browser | `Tab` | `toggle-info-panel` | open or close the browser's story info panel |
| Browser | `I` | `toggle-info-panel` | open or close the browser's story info panel |
| Browser | `G` | `toggle-gallery` | switch the browser between the story list and the cover gallery |
| Browser | `F` | `fetch-story` | re-fetch the selected story's IFDB metadata, ignoring the cache |
| Browser | `R` | `refresh-library` | fetch IFDB metadata for every story that is missing or stale |
| Browser | `U` | `set-ifdb-url` | point the selected story at an IFDB page by hand |
| Browser | `/` | `search-ifdb` | search IFDB by title or author and download a story into this directory |
| Browser | `Shift+U` | `open-url` | download a story from a URL into this library and open it |
| Browser | `Shift+H` | `download-hints` | download a matching InvisiClues hint file for the selected story |
| Browser | `S` | `sort-library` | cycle the browser's sort column, keeping the direction |
| Browser | `D` | `reverse-sort` | reverse the browser's sort direction, keeping the column |
| Browser | `Ctrl+F` | `find-story` | type to filter the whole library by title, author, filename or folder |
| Browser | `Backspace` | `parent-folder` | leave the current library folder for the one above it |
| Browser | `Q` | `quit-browser` | leave the story browser |
| Browser | `Ctrl+Q` | `quit-browser` | leave the story browser |
| Browser | `Esc` | `cancel-browser` | cancel a running fetch, or leave the browser when nothing is in flight |

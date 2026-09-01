# Documentation map

Three tiers. Start at the top and go deeper only if you want to.

## Guide

Player-facing, task-oriented, no source-reading required.

| Page | What it covers |
|---|---|
| [`guide/getting-started.md`](guide/getting-started.md) | Getting started — install it, point it at a story, and play |
| [`guide/playing.md`](guide/playing.md) | Playing — the terminal cockpit: panes, mouse, command palette, InvisiClues |
| [`guide/the-map.md`](guide/the-map.md) | The map — reading and steering the automap as it draws itself |
| [`guide/saves-and-rewind.md`](guide/saves-and-rewind.md) | Saves and rewind — Save State vs. the game's own save, and per-turn undo |
| [`guide/looks.md`](guide/looks.md) | Looks — themes, glyphs, and fixing tofu boxes in your font |
| [`guide/graphics-and-terminals.md`](guide/graphics-and-terminals.md) | Graphics and terminals — v6 art, original disk presses, and terminal support |
| [`guide/sound.md`](guide/sound.md) | Sound — in-game audio, and hearing it over a remote session |
| [`guide/play-in-a-browser.md`](guide/play-in-a-browser.md) | Play in a browser — the Docker image and its two run modes |
| [`guide/command-line.md`](guide/command-line.md) | Command line — flags, and the bare-terminal `zvm-cli`/`gvm-cli`/`scott-cli` players |
| [`guide/troubleshooting.md`](guide/troubleshooting.md) | Troubleshooting — when something doesn't look or behave right |

## Reference

Generated straight from the code's own registries — a command, key binding,
config setting or style selector can never be documented differently from what
lanthorn actually does. Regenerate after touching a registry:

```sh
LANTHORN_REGEN_DOCS=1 cargo nextest run -p app docs_reference
```

| Page | Generated from |
|---|---|
| [`reference/commands.md`](reference/commands.md) | `slash::COMMANDS` — every slash command |
| [`reference/keys.md`](reference/keys.md) | `keymap::KeyMap::default()` — the built-in key bindings |
| [`reference/config.md`](reference/config.md) | `config_template::GROUPS` — every `config.toml` setting |
| [`reference/style.md`](reference/style.md) | `theme::registry::REGISTRY` — every `style.toml` selector |
| [`reference/standards.md`](reference/standards.md) | hand-maintained — the specs lanthorn implements against (Z-Machine, Glulx, Glk, Quetzal, Blorb, Treaty of Babel) |

## Internals

How lanthorn is built, for anyone changing the code. Hand-maintained; kept
current alongside the feature it describes (README tracks the released build,
these track the code).

| Page | What it covers |
|---|---|
| [`internals/architecture.md`](internals/architecture.md) | The crate layout, the engine/host seam, and how the render pipeline turns three story formats into one screen model |
| [`internals/interface.md`](internals/interface.md) | The terminal cockpit's own mechanics: panes, mouse, command palette, story picker |
| [`internals/interpreter.md`](internals/interpreter.md) | How the three engines are told apart and driven, and how an original release disk resolves to a machine |
| [`internals/mapping.md`](internals/mapping.md) | How the automapper places, routes and de-overlaps rooms as you explore |
| [`internals/customization.md`](internals/customization.md) | `config.toml`/`style.toml` resolution, precedence, and every override surface |
| [`internals/saves.md`](internals/saves.md) | The save/restore feature surface: Save State, `@save`, Quetzal import/export, rewind |
| [`internals/platforms.md`](internals/platforms.md) | What differs across macOS, Linux and Windows, and why |
| [`internals/docker.md`](internals/docker.md) | The container image, its two run modes, and the volumes it expects |
| [`internals/releasing.md`](internals/releasing.md) | The hand-run release procedure: preconditions, dry runs, the release commit, tagging, and the one-time GHCR visibility step |
| [`internals/v6-graphics.md`](internals/v6-graphics.md) | Graphical Z-machine v6: the hybrid/raster render pipeline, art density vs. text density, per-machine typefaces |
| [`internals/persistence.md`](internals/persistence.md) | The three persistence layers in detail: what each captures, when it triggers, what survives |
| [`internals/remote-sound.md`](internals/remote-sound.md) | Why audio plays on the local device, and how to route it back over SSH |
| [`internals/glyphs.md`](internals/glyphs.md) | Exactly which Unicode ranges the map's line art needs from your font |
| [`internals/zvm-embedding-review.md`](internals/zvm-embedding-review.md) | What `zvm`'s public API would need to be a crate someone outside lanthorn depends on |
| [`internals/ci-fixture-coverage.md`](internals/ci-fixture-coverage.md) | Which integration suites `stories/` being gitignored leaves untested on CI, and which of those a fixture could close |
| [`internals/mapping-rules-idea.md`](internals/mapping-rules-idea.md) | A snapshot of how the automapper places and draws rooms today, as a baseline for redesign |
| [`internals/mapping_rules_concept.md`](internals/mapping_rules_concept.md) | Early concept notes on relative room constraints and grid-based layout |

`design/`, `plans/` and `superpowers/` are working notes — planning documents
and specs from past and in-flight efforts, not living documentation. They are
not held to the same accuracy bar as the three tiers above.

<!-- generated from crates/app/src/theme/registry.rs (theme::registry::REGISTRY) by docs_reference; do not edit by hand -->
# Style reference

Every themeable `style.toml` selector: which role or selector it derives from, and what its built-in default changes on top of that parent. An empty Default cell inherits its parent exactly.

| Selector | Section | Kind | Parent | Default | Description |
|---|---|---|---|---|---|
| `text` | Roles | Style |  |  | |
| `chrome` | Roles | Style |  |  | |
| `line` | Roles | Style |  |  | |
| `accent` | Roles | Style |  |  | |
| `muted` | Roles | Style |  |  | |
| `alert` | Roles | Style |  |  | |
| `heading` | Roles | Style |  |  | |
| `transcript` | Elements | Style | `text` |  | |
| `status_bar` | Elements | Style | `chrome` | `reversed` | |
| `help_bar` | Elements | Style | `chrome` | `reversed` | |
| `upper_window` | Elements | Style | `chrome` |  | |
| `story_info` | Elements | Style | `chrome` |  | |
| `status_header` | Elements | Style | `heading` | `bg=Black` | |
| `story_title` | Elements | Style | `heading` |  | |
| `input_line` | Elements | Style | `line` |  | |
| `suggestion_line` | Elements | Style | `line` |  | |
| `scrollbar` | Elements | Style | `line` |  | |
| `scrollbar_track` | Elements | Style | `muted` |  | |
| `transcript_location` | Elements | Style | `accent` |  | |
| `story_badge` | Elements | Style | `text` |  | |
| `badge_icons` | Elements | Placement |  | `glyph="plain"` | |
| `badge_save` | Elements | Placement |  | `glyph="S"` | |
| `badge_hint` | Elements | Placement |  | `glyph="H"` | |
| `badge_hint_available` | Elements | Placement |  | `glyph="h"` | |
| `hyperlink` | Elements | Style | `accent` | `underline` | |
| `story_info_label` | Elements | Style | `muted` |  | |
| `suggestion` | Elements | Style | `muted` |  | |
| `transcript_meta` | Elements | Style | `muted` | `glyph="▏"` | |
| `transcript_warning` | Elements | Style | `alert` | `glyph="!"` | |
| `transcript_crash` | Elements | Style | `alert` | `bold` | |
| `transcript_assist` | Elements | Style | `alert` | `glyph="●"` | |
| `transcript_assist_caution` | Elements | Style | `alert` | `bold glyph="●"` | |
| `transcript_reveal` | Elements | Style | `accent` | `underline` | |
| `panel.background` | Panel | Style |  |  | |
| `panel.border` | Panel | BorderGlyphs | `line` | `border=Single` | |
| `panel.border:active` | Panel | BorderGlyphs | `line` | `bold border=Single` | |
| `panel.title` | Panel | Style | `heading` |  | |
| `panel.tab` | Panel | Style | `muted` |  | |
| `panel.tab:active` | Panel | Style | `accent` | `bold` | |
| `panel.tab_divider` | Panel | BorderGlyphs | `line` | `glyph="│"` | |
| `panel.terminator_left` | Panel | BorderGlyphs | `line` | `glyph="┤"` | |
| `panel.terminator_right` | Panel | BorderGlyphs | `line` | `glyph="├"` | |
| `panel.control` | Panel | Style | `muted` |  | |
| `panel.control:lit` | Panel | Style | `alert` | `bold` | |
| `panel.control:hover` | Panel | Style | `accent` | `reversed` | |
| `glk.buffer.normal` | GlkBuffer | Style | `text` |  | |
| `glk.buffer.emphasized` | GlkBuffer | Style | `text` | `italic` | |
| `glk.buffer.preformatted` | GlkBuffer | Style | `text` |  | |
| `glk.buffer.header` | GlkBuffer | Style | `heading` | `bold` | |
| `glk.buffer.subheader` | GlkBuffer | Style | `heading` | `bold` | |
| `glk.buffer.alert` | GlkBuffer | Style | `alert` | `bold` | |
| `glk.buffer.note` | GlkBuffer | Style | `muted` | `italic` | |
| `glk.buffer.blockquote` | GlkBuffer | Style | `muted` |  | |
| `glk.buffer.input` | GlkBuffer | Style | `accent` |  | |
| `glk.buffer.user1` | GlkBuffer | Style | `text` |  | |
| `glk.buffer.user2` | GlkBuffer | Style | `text` |  | |
| `glk.grid.normal` | GlkGrid | Style | `chrome` |  | |
| `glk.grid.emphasized` | GlkGrid | Style | `chrome` | `italic` | |
| `glk.grid.preformatted` | GlkGrid | Style | `chrome` |  | |
| `glk.grid.header` | GlkGrid | Style | `heading` | `bold` | |
| `glk.grid.subheader` | GlkGrid | Style | `heading` | `bold` | |
| `glk.grid.alert` | GlkGrid | Style | `alert` | `bold` | |
| `glk.grid.note` | GlkGrid | Style | `muted` | `italic` | |
| `glk.grid.blockquote` | GlkGrid | Style | `muted` |  | |
| `glk.grid.input` | GlkGrid | Style | `accent` |  | |
| `glk.grid.user1` | GlkGrid | Style | `chrome` |  | |
| `glk.grid.user2` | GlkGrid | Style | `chrome` |  | |
| `glk.grid.background` | GlkGrid | Style | `chrome` | `reversed` | |
| `map.background` | Map | Style |  |  | |
| `map.room` | Map | Style | `text` |  | |
| `map.room_current` | Map | Style | `accent` |  | |
| `map.room_selected` | Map | Style | `accent` | `reversed` | |
| `map.connector` | Map | Style | `accent` |  | |
| `map.connector_distorted` | Map | Style |  | `fg=Magenta` | |
| `map.connector_portal` | Map | Style | `accent` |  | |
| `map.shared_path` | Map | Style |  | `fg=LightCyan` | |
| `map.loc_indicator` | Map | Style | `muted` |  | |
| `map.matrix.header` | Map | Style | `heading` |  | |
| `map.matrix.row:here` | Map | Style | `accent` | `reversed` | |
| `map.matrix.row:selected` | Map | Style | `accent` |  | |
| `map.matrix.cell:entrance` | Map | Style | `text` | `bold` | |
| `map.matrix.cell:path` | Map | Style | `accent` | `bold underline` | |
| `map.matrix.cell:frontier` | Map | Style | `muted` |  | |
| `map.matrix.footnote` | Map | Style | `muted` |  | |
| `map.edge:oneway` | Map | Style | `map.connector` |  | |
| `map.edge:asym` | Map | Style | `map.connector` |  | |
| `map.trail` | Map | Style | `muted` |  | |
| `map.box_style` | Map | Placement |  | `glyph="rounded"` | |
| `map.arrow_set` | Map | Placement |  | `glyph="filled"` | |
| `map.portal_icons` | Map | Placement |  | `glyph="ascii"` | |
| `map.path_style` | Map | Placement |  | `glyph="light"` | |
| `map.portal_path_style` | Map | Placement |  | `glyph="dotted"` | |
| `map.control_icons` | Map | Placement |  | `glyph="plain"` | |
| `map.diagonal_corners` | Map | Placement |  |  | |
| `debug.pc` | Debug | Style | `accent` | `reversed` | |
| `debug.disasm_executed` | Debug | Style | `accent` | `fg=Blue glyph="|"` | |
| `debug.disasm_rd` | Debug | Style | `text` | `fg=Yellow glyph=" "` | |
| `debug.disasm_soft` | Debug | Style | `muted` | `fg=Red glyph=" "` | |
| `debug.disasm_data` | Debug | Style | `muted` | `italic glyph=" "` | |
| `debug.zstring` | Debug | Style | `accent` | `italic` | |
| `dialog.background` | Dialog | Style | `chrome` |  | |
| `dialog.border` | Dialog | BorderGlyphs | `line` | `bold border=Single` | |
| `dialog.title` | Dialog | Style | `accent` |  | |
| `dialog.button` | Dialog | Style | `chrome` | `reversed` | |
| `dialog.button:active` | Dialog | Style | `accent` | `reversed` | |
| `dialog.shadow` | Dialog | Style | `muted` | `bg=DarkGray` | |
| `tooltip.background` | Tooltip | Style | `dialog.list_selected` | `!bold` | |
| `tooltip.border` | Tooltip | BorderGlyphs | `line` | `border=None` | |
| `more_prompt` | Elements | Style | `chrome` | `reversed` | |
| `tidy_progress` | Elements | Style | `accent` |  | |
| `meta_marker` | Elements | Style | `muted` |  | |
| `inventory_dock` | Elements | Style | `accent` |  | |
| `room_dock` | Elements | Style | `text` |  | |
| `room_dock.header` | Elements | Style | `heading` |  | |
| `room_dock.header:pinned` | Elements | Style | `accent` | `reversed` | |
| `story_info_title` | Elements | Style | `heading` |  | |
| `terminal_dump_heading` | Elements | Style | `heading` | `bold` | |
| `terminal_dump_assumed` | Elements | Style | `alert` |  | |
| `story_info_value` | Elements | Style | `text` |  | |
| `story_info_blurb` | Elements | Style | `muted` | `italic` | |
| `story_info_link` | Elements | Style | `accent` | `underline` | |
| `story_info_cover` | Elements | Style | `chrome` |  | |
| `story_info_continuation` | Elements | Style | `muted` |  | |
| `story_info_artwork` | Elements | Style | `story_info_value` |  | |
| `story_info_artwork:active` | Elements | Style | `accent` | `bold` | |
| `graphics` | Elements | Style | `chrome` |  | |
| `inline_image` | Elements | Style | `chrome` |  | |
| `story_header` | Elements | Style | `muted` |  | |
| `story_header_active` | Elements | Style | `accent` | `bold` | |
| `story_author` | Elements | Style | `text` |  | |
| `story_year` | Elements | Style | `text` |  | |
| `story_rating` | Elements | Style | `text` |  | |
| `story_no_metadata` | Elements | Style | `muted` |  | |
| `story_tile` | Elements | Style | `text` |  | |
| `story_tile_selected` | Elements | Style | `accent` | `bold reversed` | |
| `story_folder` | Elements | Style | `accent` |  | |
| `notification` | Elements | Style | `accent` | `reversed` | |
| `hotkey_key` | Elements | Style | `accent` |  | |
| `sound_beep_high` | Elements | Style |  | `fg=Rgb(255, 180, 40)` | |
| `sound_beep_low` | Elements | Style |  | `fg=Rgb(60, 140, 220)` | |
| `transcript_input` | Elements | Style | `accent` |  | |
| `transcript_system` | Elements | Style | `muted` |  | |
| `warning_marker` | Elements | Style | `alert` |  | |
| `input_text` | Elements | Style | `text` |  | |
| `input_prompt` | Elements | Style | `text` |  | |
| `upper_window_border` | Elements | Style | `line` |  | |
| `room_panel` | Elements | Style | `accent` | `reversed` | |
| `palette_query` | Elements | Style | `text` |  | |
| `palette_name` | Elements | Style | `text` |  | |
| `palette_match` | Elements | Style | `accent` | `bold` | |
| `palette_desc` | Elements | Style | `muted` |  | |
| `palette_selected` | Elements | Style | `accent` | `reversed` | |
| `ifdb_result` | Elements | Style | `text` |  | |
| `ifdb_result_selected` | Elements | Style | `accent` | `bold reversed` | |
| `ifdb_result_meta` | Elements | Style | `muted` |  | |
| `ifdb_download_marker` | Elements | Style | `accent` | `glyph="⭳"` | |
| `ifdb_download_present` | Elements | Style | `muted` | `glyph="✓"` | |
| `ifdb_attribution` | Elements | Style | `muted` | `italic` | |
| `saves_portable` | Elements | Style | `accent` | `glyph="↗"` | |
| `saves_host_only` | Elements | Style | `muted` |  | |
| `dialog.list_selected` | Dialog | Style |  | `fg=Black bg=Cyan bold` | |
| `dialog.list_footer` | Dialog | Style | `muted` |  | |
| `dialog.list_header` | Dialog | Style | `text` | `underline` | |
| `dialog.hint_suggestion` | Dialog | Style | `alert` | `dim` | |
| `dialog.launch_caveat` | Dialog | Style | `alert` |  | |
| `dialog.region_prompt.body` | Dialog | Style | `dialog.background` |  | |
| `dialog.region_prompt.rooms` | Dialog | Style | `dialog.list_footer` |  | |
| `dialog.region_prompt.room` | Dialog | Style | `dialog.region_prompt.rooms` |  | |
| `dialog.region_prompt.option` | Dialog | Style | `dialog.background` |  | |
| `dialog.region_prompt.option:chosen` | Dialog | Style | `dialog.list_selected` |  | |
| `dialog.font_check.sample` | Dialog | Style | `dialog.background` |  | |
| `band.column_header` | Elements | Style | `muted` |  | |
| `band.column_header:active` | Elements | Style | `accent` | `bold` | |
| `band.quick` | Elements | Style | `text` |  | |
| `band.quick:hover` | Elements | Style | `band.quick` | `reversed` | |
| `band.group_label` | Elements | Style | `heading` |  | |
| `band.item:seen` | Elements | Style | `muted` |  | |
| `file_browser_cwd` | Elements | Style | `alert` |  | |
| `file_browser_dir` | Elements | Style | `accent` |  | |
| `inspector_edge_ok` | Elements | Style |  | `fg=Green` | |
| `inspector_edge_distorted` | Elements | Style |  | `fg=Red` | |
| `transcript_search_highlight` | Elements | Style |  | `fg=Black bg=Yellow` | |

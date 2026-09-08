//! Host plumbing shared by `zvm-cli`, `gvm-cli`, and `scott-cli`.
//!
//! The three CLIs drive three unrelated VMs and each owns its own renderer —
//! Z-machine windows, Glk windows, and Scott's prose are genuinely different
//! display models, and nothing useful is shared between them. But *around* the
//! renderer every one of them does the same four things: decide whether the
//! terminal can take escapes, read input (and stop cleanly at EOF), put the
//! terminal back on the way out, and answer `--help` / `--version`. That is what
//! lives here.
//!
//! The motivating evidence is drift, not tidiness. Five escape helpers were
//! byte-identical in `zvm-cli` and `gvm-cli`; more tellingly, the stdin-EOF bug —
//! treating a 0-byte read as a blank line and looping forever on it — was fixed
//! in `zvm-cli` long ago and still shipped in `gvm-cli` months later (SQ-0604),
//! and `zvm-cli`'s *char* path still exited without restoring the terminal. One
//! defect, three chances to get it wrong. Now there is one implementation.
//!
//! Deliberately NOT here: anything that draws. See the module docs for the
//! boundary each piece sits on.

pub mod args;
pub mod disk_set;
pub mod flags;
pub mod input;
pub mod line;
pub mod menu;
pub mod mode;
pub mod pager;
pub mod pin;
pub mod score;
pub mod storage;
pub mod story_pick;
pub mod term;
pub mod titles;

pub use args::{Matches, Opt, scan};
pub use flags::{
    EXIT_USAGE, HELP_WIDTH, handled_common_flags, looks_like_flag, on_off, overlong_help_lines,
    usage_error, wrap_tokens,
};
pub use line::LineHold;
pub use input::{
    key_press, read_byte_or_eof, read_byte_stdin, read_char_from_line, read_line_or_eof,
    read_line_stdin,
};
pub use menu::{Emission, MENU_COMMAND, MenuBlock, MenuTracker, NavKey, Typed, is_menu_request};
pub use mode::{HostMode, PLAIN_FLAGS, no_color, plain_requested};
pub use pager::{Pager, wait_for_keypress};
pub use pin::{
    PIN_COMMAND, Pin, enter_region, leave_and_park, leave_and_park_now, leave_region, pin_request,
    pinned_origin,
};
pub use score::{ScoreWatch, score_in_status};
pub use storage::{
    DiskBuild, QUETZAL_EXT, SCOTT_EXT, disk_story_key, existing_saves, game_dir,
    game_dir_with_key, is_yes, overwrite_warning, pick_save, resolve_save_input,
    save_list_line, story_key, story_key_at, story_key_for, StoryOrigin,
};
pub use titles::known_title;
pub use term::{
    TerminalGuard, cursor_reset, cursor_steady_bar, cursor_steady_block, cursor_steady_underline,
    end_raw_mode, osc_reset_bg, osc_reset_fg, osc_set_bg, osc_set_fg, page_bg_escape,
    restore_and_exit, restore_terminal, rgb24,
};

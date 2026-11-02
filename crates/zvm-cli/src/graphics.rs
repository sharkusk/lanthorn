//! Version 6 picture drawing for a text terminal.
//!
//! Conservative: a pipe, `--plain`, `TERM=dumb`, an empty `TERM`, or `NO_COLOR`
//! never emits graphics. Otherwise kitty first, then sixel if TERM names it or
//! is `foot`, else Unicode half-blocks.

use std::collections::HashMap;
use std::env;

use zvm::cpu::exec::{PaintEvent, PictureEvent};

use crate::pictures::{Kind, Pict};

/// How pictures are drawn when graphics are allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Kitty,
    Sixel,
    Halfblocks,
}

/// What `--image-protocol` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Off,
    Force(Protocol),
}

/// Default Version 6 text cell, ZMSD §8.2.4 / header `$26`/`$27` seed.
pub const CELL: (u16, u16) = (8, 16);

/// Kitty row/column diacritics: index is the value. First 80 cover an 80-column
/// placement. Same sequence `ratatui-image` and `qwertty-term-vt` agree on.
const DIA: [char; 80] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
    '\u{5A8}', '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}', '\u{658}',
    '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}', '\u{6D7}', '\u{6D8}',
    '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}', '\u{6E0}', '\u{6E1}', '\u{6E2}',
];

/// Live picture host for one run.
pub struct Host {
    protocol: Protocol,
    picts: HashMap<u16, Pict>,
    cell: (u16, u16),
    ids: HashMap<(u8, u16), u32>,
    next_id: u32,
}

impl Host {
    /// Open a host when this run may draw pictures.
    pub fn open(
        choice: Choice,
        stdout_tty: bool,
        plain: bool,
        picts: Vec<Pict>,
    ) -> Option<Host> {
        let protocol = resolve(choice, stdout_tty, plain, env_term())?;
        let mut map = HashMap::new();
        for p in picts {
            map.insert(p.number, p);
        }
        Some(Host {
            protocol,
            picts: map,
            cell: CELL,
            ids: HashMap::new(),
            next_id: 1,
        })
    }

    #[allow(dead_code)]
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    #[cfg(test)]
    pub fn for_test(protocol: Protocol, picts: Vec<Pict>) -> Host {
        let mut map = HashMap::new();
        for p in picts {
            map.insert(p.number, p);
        }
        Host {
            protocol,
            picts: map,
            cell: CELL,
            ids: HashMap::new(),
            next_id: 1,
        }
    }

    /// Bytes to write for this turn's paint events. Empty when nothing draws.
    pub fn render(&mut self, events: &[PaintEvent]) -> String {
        let mut out = String::new();
        for ev in events {
            match ev {
                PaintEvent::Picture(p) if p.erase => out.push_str(&self.erase_picture(p)),
                PaintEvent::Picture(p) => out.push_str(&self.draw_picture(p)),
                PaintEvent::Erase(f) => out.push_str(&self.erase_fill(f)),
                _ => {}
            }
        }
        out
    }

    fn draw_picture(&mut self, ev: &PictureEvent) -> String {
        let Some(pict) = self.picts.get(&ev.number) else {
            return String::new();
        };
        if matches!(pict.kind, Kind::Rect) {
            return String::new();
        }
        let Some((pw, ph, rgba)) = decode(pict) else {
            return String::new();
        };
        let (cw, ch) = self.cell;
        let cols = pw.div_ceil(cw).max(1);
        let rows = ph.div_ceil(ch).max(1);
        let (cell_col, cell_row) = pixel_cell(ev, cw, ch);
        let id = self.id_for(ev.window, ev.number);
        let mut s = String::new();
        // Paint events are drained after the turn's text, so even an at-cursor
        // draw is placed by CUP rather than at the live cursor.
        s.push_str(&cup(cell_row, cell_col));
        match self.protocol {
            Protocol::Kitty => {
                s.push_str(&kitty_transmit(id, &rgba, pw, ph, cols, rows));
                s.push_str(&kitty_placeholders(id, cell_row, cell_col, cols, rows, false));
            }
            Protocol::Sixel => s.push_str(&sixel(&rgba, pw, ph)),
            Protocol::Halfblocks => {
                s.push_str(&halfblocks(&rgba, pw, ph, cw, ch, Some((cell_row, cell_col))))
            }
        }
        if ev.margin_after.is_some() && ev.at_cursor {
            s.push('\n');
        }
        s
    }

    fn erase_picture(&mut self, ev: &PictureEvent) -> String {
        match self.protocol {
            Protocol::Kitty => {
                if let Some(&id) = self.ids.get(&(ev.window, ev.number)) {
                    return format!("\x1b_Ga=d,d=i,i={id}\x1b\\");
                }
                String::new()
            }
            Protocol::Sixel | Protocol::Halfblocks => {
                let (cw, ch) = self.cell;
                let (col, row) = pixel_cell(ev, cw, ch);
                let Some(pict) = self.picts.get(&ev.number) else {
                    return String::new();
                };
                let cols = pict.width.div_ceil(cw).max(1);
                let rows = pict.height.div_ceil(ch).max(1);
                blank_rect(row, col, cols, rows)
            }
        }
    }

    fn erase_fill(&self, _f: &zvm::cpu::exec::EraseFill) -> String {
        // Kitty and sixel skip fills: the page colour is OSC 11, and pictures
        // are the only pixels those backends own. Half-blocks used to paint
        // every `erase_window` as a cell rectangle. Scopa's HardPic path
        // (1-pixel borders, 80×20 button boxes, card-shaped white fills)
        // then `div_ceil`s to whole cells, which is the stray white slabs
        // and the extra "card" left of the Aces. Skip fills here too.
        String::new()
    }

    fn id_for(&mut self, window: u8, number: u16) -> u32 {
        *self.ids.entry((window, number)).or_insert_with(|| {
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            id
        })
    }
}

pub fn parse_choice(value: Option<&str>) -> Result<Choice, String> {
    match value {
        None | Some("auto") => Ok(Choice::Auto),
        Some("off") => Ok(Choice::Off),
        Some("kitty") => Ok(Choice::Force(Protocol::Kitty)),
        Some("sixel") => Ok(Choice::Force(Protocol::Sixel)),
        Some("halfblocks") => Ok(Choice::Force(Protocol::Halfblocks)),
        Some(v) => Err(format!(
            "--image-protocol takes auto, kitty, sixel, halfblocks or off, got '{v}'"
        )),
    }
}

/// Resolve the protocol for this run. Piped / plain / NO_COLOR / dumb stay text.
pub fn resolve(choice: Choice, stdout_tty: bool, plain: bool, term: TermEnv) -> Option<Protocol> {
    if !stdout_tty || plain || term.no_color || term.dumb {
        return None;
    }
    match choice {
        Choice::Off => None,
        Choice::Force(p) => Some(p),
        Choice::Auto => auto_detect(&term),
    }
}

/// Environment facts that decide auto protocol. Tests pass this in.
#[derive(Clone, Debug, Default)]
pub struct TermEnv {
    pub term: String,
    pub term_program: String,
    pub kitty_window: bool,
    pub ghostty: bool,
    pub no_color: bool,
    pub dumb: bool,
    pub colorterm: String,
}

pub fn env_term() -> TermEnv {
    let term = env::var("TERM").unwrap_or_default();
    let dumb = term.is_empty() || term == "dumb";
    TermEnv {
        term_program: env::var("TERM_PROGRAM").unwrap_or_default(),
        kitty_window: env::var("KITTY_WINDOW_ID").is_ok(),
        ghostty: env::var("GHOSTTY_RESOURCES_DIR").is_ok()
            || env::var("TERM_PROGRAM").as_deref() == Ok("ghostty"),
        no_color: cli_host::no_color(),
        dumb,
        colorterm: env::var("COLORTERM").unwrap_or_default(),
        term,
    }
}

fn auto_detect(term: &TermEnv) -> Option<Protocol> {
    let t = term.term.to_ascii_lowercase();
    let prog = term.term_program.to_ascii_lowercase();
    if term.kitty_window || t.contains("kitty") || term.ghostty || prog == "ghostty" {
        return Some(Protocol::Kitty);
    }
    if t.contains("sixel") || t.contains("foot") || t.contains("mlterm") || prog == "wezterm" {
        return Some(Protocol::Sixel);
    }
    if !term.colorterm.is_empty() || t.contains("256color") || t.contains("truecolor") {
        return Some(Protocol::Halfblocks);
    }
    None
}

fn pixel_cell(ev: &PictureEvent, cw: u16, ch: u16) -> (u16, u16) {
    let (wx, wy, _, _) = ev.win_box;
    let px = wx.saturating_add(ev.x.saturating_sub(1)).saturating_sub(1);
    let py = wy.saturating_add(ev.y.saturating_sub(1)).saturating_sub(1);
    ((px / cw) + 1, (py / ch) + 1)
}

fn cup(row: u16, col: u16) -> String {
    format!("\x1b[{row};{col}H")
}

fn blank_rect(row: u16, col: u16, cols: u16, rows: u16) -> String {
    let mut s = String::new();
    let spaces = " ".repeat(cols as usize);
    for r in 0..rows {
        s.push_str(&cup(row + r, col));
        s.push_str(&spaces);
    }
    s
}

fn decode(pict: &Pict) -> Option<(u16, u16, Vec<u8>)> {
    let img = image::load_from_memory(&pict.bytes).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    let w = u16::try_from(w).ok()?;
    let h = u16::try_from(h).ok()?;
    Some((w, h, img.into_raw()))
}

/// Standard alphabet, padded. Kitty payload only.
pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Uncompressed virtual kitty transmit, `U=1`, anonymous placement.
pub fn kitty_transmit(id: u32, rgba: &[u8], w: u16, h: u16, cols: u16, rows: u16) -> String {
    let chunks: Vec<&[u8]> = rgba.chunks(3072).collect();
    let n = chunks.len();
    let mut out = String::new();
    for (i, chunk) in chunks.into_iter().enumerate() {
        let more = u8::from(i + 1 < n);
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Gq=2,i={id},a=T,U=1,f=32,t=d,s={w},v={h},r={rows},c={cols},m={more};"
            ));
        } else {
            out.push_str(&format!("\x1b_Gq=2,m={more};"));
        }
        out.push_str(&b64(chunk));
        out.push_str("\x1b\\");
    }
    out
}

/// Placeholder grid. `at_cursor` skips CUP so the image sits in the text flow.
pub fn kitty_placeholders(
    id: u32,
    row0: u16,
    col0: u16,
    cols: u16,
    rows: u16,
    at_cursor: bool,
) -> String {
    let [id_hi, id_r, id_g, id_b] = id.to_be_bytes();
    let extra = *DIA.get(usize::from(id_hi)).unwrap_or(&DIA[0]);
    let mut s = format!("\x1b[38;2;{id_r};{id_g};{id_b}m");
    for r in 0..rows {
        if !at_cursor || r > 0 {
            s.push_str(&cup(row0 + r, col0));
        }
        let row_d = *DIA.get(usize::from(r)).unwrap_or(&DIA[0]);
        for c in 0..cols {
            s.push('\u{10EEEE}');
            if let Some(&col_d) = DIA.get(usize::from(c)) {
                s.push(row_d);
                s.push(col_d);
                s.push(extra);
            }
        }
        if at_cursor && r + 1 < rows {
            s.push('\n');
        }
    }
    s.push_str("\x1b[39m");
    s
}

/// Upper-half block: fg is the top pixel, bg the bottom.
///
/// Sampled to the Version 6 text cell so a 52×84 card occupies the same
/// 7×6 cells kitty would, rather than 52 columns of raw pixels.
pub fn halfblocks(
    rgba: &[u8],
    w: u16,
    h: u16,
    cw: u16,
    ch: u16,
    origin: Option<(u16, u16)>,
) -> String {
    let src_w = w as usize;
    let src_h = h as usize;
    let cw = cw.max(1) as usize;
    let ch = ch.max(1) as usize;
    let dst_w = (src_w.div_ceil(cw)).max(1);
    let dst_h = (src_h.div_ceil(ch)).max(1) * 2;
    let mut s = String::new();
    let mut y = 0;
    while y < dst_h {
        if y > 0 {
            match origin {
                Some((row, col)) => s.push_str(&cup(row + (y as u16 / 2), col)),
                None => s.push('\n'),
            }
        }
        for x in 0..dst_w {
            let top = sample(rgba, src_w, src_h, x, y, dst_w, dst_h);
            let bot = if y + 1 < dst_h {
                sample(rgba, src_w, src_h, x, y + 1, dst_w, dst_h)
            } else {
                [0, 0, 0, 0]
            };
            if top[3] < 16 && bot[3] < 16 {
                s.push(' ');
                continue;
            }
            s.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                top[0], top[1], top[2], bot[0], bot[1], bot[2]
            ));
        }
        s.push_str("\x1b[0m");
        y += 2;
    }
    s
}

fn sample(rgba: &[u8], sw: usize, sh: usize, x: usize, y: usize, dw: usize, dh: usize) -> [u8; 4] {
    let sx = (x * sw / dw).min(sw.saturating_sub(1));
    let sy = (y * sh / dh).min(sh.saturating_sub(1));
    pixel(rgba, sw, sx, sy)
}

fn pixel(rgba: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// Naive sixel: one palette entry per distinct RGB in each 6-pixel band.
pub fn sixel(rgba: &[u8], w: u16, h: u16) -> String {
    let w = w as usize;
    let h = h as usize;
    let mut out = String::from("\x1bPq");
    let mut palette: HashMap<(u8, u8, u8), u16> = HashMap::new();
    let mut next_col = 0u16;
    let mut y = 0;
    while y < h {
        let mut by_colour: HashMap<u16, Vec<u8>> = HashMap::new();
        for x in 0..w {
            for dy in 0..6 {
                if y + dy >= h {
                    break;
                }
                let p = pixel(rgba, w, x, y + dy);
                if p[3] < 16 {
                    continue;
                }
                let key = (p[0], p[1], p[2]);
                let id = *palette.entry(key).or_insert_with(|| {
                    let id = next_col;
                    next_col = next_col.saturating_add(1);
                    out.push_str(&format!(
                        "#{id};2;{};{};{}",
                        scale_sixel(key.0),
                        scale_sixel(key.1),
                        scale_sixel(key.2)
                    ));
                    id
                });
                by_colour.entry(id).or_insert_with(|| vec![0u8; w])[x] |= 1 << dy;
            }
        }
        let mut first = true;
        for (id, cols) in by_colour {
            if !first {
                out.push('$');
            }
            first = false;
            out.push_str(&format!("#{id}"));
            for b in cols {
                out.push(char::from(63 + b));
            }
        }
        out.push('-');
        y += 6;
    }
    out.push_str("\x1b\\");
    out
}

fn scale_sixel(c: u8) -> u8 {
    ((u16::from(c) * 100) / 255) as u8
}


#[cfg(test)]
mod tests {
    use super::*;

    fn env(term: &str) -> TermEnv {
        TermEnv {
            term: term.into(),
            ..TermEnv::default()
        }
    }

    #[test]
    fn piped_plain_dumb_and_nocolor_stay_text() {
        let color = env("xterm-256color");
        assert_eq!(resolve(Choice::Auto, false, false, color.clone()), None);
        assert_eq!(resolve(Choice::Auto, true, true, color.clone()), None);
        let mut dumb = color.clone();
        dumb.dumb = true;
        dumb.term = "dumb".into();
        assert_eq!(resolve(Choice::Auto, true, false, dumb), None);
        let mut nc = color.clone();
        nc.no_color = true;
        assert_eq!(resolve(Choice::Auto, true, false, nc), None);
        let mut empty = color;
        empty.dumb = true;
        empty.term.clear();
        assert_eq!(resolve(Choice::Auto, true, false, empty), None);
    }

    #[test]
    fn forced_protocol_still_respects_the_text_gates() {
        assert_eq!(
            resolve(Choice::Force(Protocol::Kitty), false, false, env("xterm-kitty")),
            None
        );
        assert_eq!(
            resolve(Choice::Force(Protocol::Kitty), true, false, env("xterm-kitty")),
            Some(Protocol::Kitty)
        );
    }

    #[test]
    fn auto_picks_kitty_then_sixel_then_halfblocks() {
        let mut kitty = env("xterm-256color");
        kitty.kitty_window = true;
        assert_eq!(resolve(Choice::Auto, true, false, kitty), Some(Protocol::Kitty));
        assert_eq!(
            resolve(Choice::Auto, true, false, env("xterm-kitty")),
            Some(Protocol::Kitty)
        );
        assert_eq!(
            resolve(Choice::Auto, true, false, env("foot")),
            Some(Protocol::Sixel)
        );
        let mut wez = env("xterm-256color");
        wez.term_program = "WezTerm".into();
        assert_eq!(resolve(Choice::Auto, true, false, wez), Some(Protocol::Sixel));
        let mut hb = env("xterm-256color");
        hb.colorterm = "truecolor".into();
        assert_eq!(resolve(Choice::Auto, true, false, hb), Some(Protocol::Halfblocks));
    }

    #[test]
    fn kitty_transmit_is_virtual_rgba() {
        let rgba = [10u8, 20, 30, 255, 11, 21, 31, 255];
        let s = kitty_transmit(7, &rgba, 2, 1, 2, 1);
        assert!(s.contains("a=T,U=1,f=32,t=d"), "{s}");
        assert!(s.contains("i=7"), "{s}");
        assert!(s.contains("s=2,v=1,r=1,c=2"), "{s}");
        assert!(s.starts_with("\x1b_G"), "{s}");
        assert!(s.contains('\u{001b}'), "APC");
    }

    #[test]
    fn kitty_placeholders_use_the_unicode_grid() {
        let s = kitty_placeholders(1, 2, 3, 2, 1, false);
        assert!(s.contains('\u{10EEEE}'), "{s:?}");
        assert!(s.contains("\x1b[2;3H"), "{s:?}");
        assert_eq!(s.chars().filter(|&c| c == '\u{10EEEE}').count(), 2);
    }

    #[test]
    fn halfblocks_emit_upper_half_and_sgr() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let s = halfblocks(&rgba, 2, 2, 8, 16, None);
        assert!(s.contains('▀'), "{s}");
        assert!(s.contains("38;2;255;0;0"), "{s}");
        assert!(s.contains("48;2;0;0;255"), "{s}");
    }

    #[test]
    fn halfblocks_origin_cups_each_row() {
        let rgba = [9u8, 8, 7, 255].repeat(8 * 32);
        let s = halfblocks(&rgba, 8, 32, 8, 16, Some((5, 3)));
        assert!(s.contains("\x1b[6;3H"), "{s:?}");
        assert!(!s.contains('\n'), "{s:?}");
    }

    #[test]
    fn halfblocks_scale_to_the_text_cell() {
        let rgba = [200u8, 10, 20, 255].repeat(16 * 32);
        let s = halfblocks(&rgba, 16, 32, 8, 16, None);
        assert_eq!(s.chars().filter(|&c| c == '▀').count(), 4, "{s}");
        assert!(s.contains("38;2;200;10;20"), "{s}");
    }

    #[test]
    fn sixel_is_a_dcs_payload() {
        let rgba = [255u8, 0, 0, 255].repeat(6);
        let s = sixel(&rgba, 1, 6);
        assert!(s.starts_with("\x1bPq"), "{s}");
        assert!(s.ends_with("\x1b\\"), "{s}");
        assert!(s.contains('#'), "{s}");
    }

    #[test]
    fn parse_choice_names() {
        assert_eq!(parse_choice(None).unwrap(), Choice::Auto);
        assert_eq!(parse_choice(Some("auto")).unwrap(), Choice::Auto);
        assert_eq!(parse_choice(Some("off")).unwrap(), Choice::Off);
        assert_eq!(
            parse_choice(Some("kitty")).unwrap(),
            Choice::Force(Protocol::Kitty)
        );
        assert!(parse_choice(Some("raster")).is_err());
    }

    fn tiny_png(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(px));
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("png encode");
        buf
    }

    #[test]
    fn host_stays_closed_when_piped() {
        assert!(Host::open(Choice::Force(Protocol::Kitty), false, false, vec![]).is_none());
    }

    #[test]
    fn host_draws_a_png_as_kitty() {
        let pict = Pict {
            number: 3,
            width: 8,
            height: 16,
            kind: Kind::Png,
            bytes: tiny_png(8, 16, [200, 10, 30, 255]),
        };
        let mut host = Host::for_test(Protocol::Kitty, vec![pict]);
        let ev = PaintEvent::Picture(PictureEvent::new(
            3,
            0,
            1,
            1,
            false,
            0,
            None,
            false,
            (1, 1, 640, 400),
        ));
        let s = host.render(&[ev]);
        assert_eq!(host.protocol(), Protocol::Kitty);
        assert!(s.contains("a=T,U=1,f=32,t=d"), "{s}");
        assert!(s.contains('\u{10EEEE}'), "{s:?}");
        assert!(s.contains("\x1b[1;1H"), "{s:?}");
    }

    #[test]
    fn halfblocks_skip_erase_fills_like_kitty_and_sixel() {
        // Scopa HardPic: a 90×20 white box and a 1-pixel strip. Quantizing
        // those to cells painted the stray white rectangles in the capture.
        let fill = PaintEvent::Erase(zvm::cpu::exec::EraseFill::new(
            3,
            276,
            191,
            90,
            20,
            zvm::screen::ZColour::True(32767),
            0,
        ));
        let hairline = PaintEvent::Erase(zvm::cpu::exec::EraseFill::new(
            3,
            200,
            180,
            80,
            1,
            zvm::screen::ZColour::True(32767),
            0,
        ));
        for protocol in [Protocol::Halfblocks, Protocol::Kitty, Protocol::Sixel] {
            let s = Host::for_test(protocol, vec![]).render(&[fill, hairline]);
            assert!(
                s.is_empty(),
                "{protocol:?} must not paint erase fills: {s:?}"
            );
        }
    }

    #[test]
    fn host_draws_halfblocks_and_sixel() {
        let pict = Pict {
            number: 1,
            width: 2,
            height: 2,
            kind: Kind::Png,
            bytes: tiny_png(2, 2, [255, 0, 0, 255]),
        };
        let ev = PaintEvent::Picture(PictureEvent::new(
            1,
            0,
            1,
            1,
            false,
            0,
            None,
            false,
            (1, 1, 640, 400),
        ));
        let hb = Host::for_test(Protocol::Halfblocks, vec![pict.clone()]).render(&[ev]);
        assert!(hb.contains('▀'), "{hb}");
        assert!(hb.contains("38;2;255;0;0"), "{hb}");
        let sx = Host::for_test(Protocol::Sixel, vec![pict]).render(&[ev]);
        assert!(sx.contains("\x1bPq"), "{sx}");
        assert!(sx.ends_with("\x1b\\"), "{sx}");
    }

    #[test]
    fn kitty_stream_resolves_a_placement_in_qwertty() {
        use qwertty_term_vt::kitty::{resolve_window, TerminalGeometry};
        use qwertty_term_vt::stream::{Stream, TerminalHandler};
        use qwertty_term_vt::terminal::{Options, Terminal};

        let rgba = [255u8, 0, 0, 255].repeat(8 * 16);
        let stream = format!(
            "{}{}",
            kitty_transmit(1, &rgba, 8, 16, 1, 1),
            kitty_placeholders(1, 1, 1, 1, 1, false)
        );
        let mut t = Terminal::new(Options {
            cols: 20,
            rows: 6,
            max_scrollback: 100,
            ..Default::default()
        });
        t.width_px = 20 * 8;
        t.height_px = 6 * 16;
        let mut s = Stream::new(TerminalHandler::new(t));
        s.feed(stream.as_bytes());
        let t = s.handler.terminal;
        let geo = TerminalGeometry::new(t.cols, t.rows, t.width_px, t.height_px);
        let screen = t.screen();
        let (placements, images, _) =
            resolve_window(&screen.kitty_images, &screen.pages, &geo, 0);
        assert!(
            images.iter().any(|i| i.id == 1),
            "image id 1 stored; ids={:?}",
            images.iter().map(|i| i.id).collect::<Vec<_>>()
        );
        assert!(
            !placements.is_empty(),
            "virtual placement should resolve"
        );
    }
}

//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the command band and the two v6 render switches had no presence on
//! screen at all: a player who turned guidance on saw nothing and could only
//! conclude it was broken. These assert the four things that close the gap —
//! the controls are THERE, they SHOW their state, the v6-only pair appears only
//! on a v6 story, and each one sits where the thing it governs is.
//!
//! **The placement rule, because it is the part most likely to be undone by a
//! later edit that means well.** A control rides the border nearest what it
//! switches: the command band opens BELOW the story pane and the map lives to
//! the RIGHT, so those toggles take the bottom border and its right-hand end;
//! guidance and the word reveal have no direction of their own and join the
//! band; and the two v6 switches govern how the story pane ITSELF is drawn, so
//! they keep that pane's own top border. Off v6 there is no top cluster at all.
//!
//! The return probe joins the map toggle in the anchored group (SQ-1107) — see
//! `return_probe.rs` for why the map pane could not keep it, and for the order
//! inside that pair.
//!
//! Everything here renders into a buffer and reads cells back, because that is
//! the only evidence about a screen that is worth anything.

use app::render::controls::{
    control_at, controls_for, draw_control_hint, draw_pane_with_controls, map_controls_for,
    BorderControl, ControlPane,
};
use app::render::panel::{draw_panel_with_controls, PanelSpec, PanelStrip};
use app::render::paneframe::{header_controls_width, InsetSegment, PaneGlyphs};
use app::state::{AppState, CommandBandState, Layout};

use mapper::layer::{MapView, MAIN_LAYER};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// Draw the story panel the way `main::draw_story_panel` does, into a fresh
/// buffer, and hand back the buffer plus the control hit-rects.
fn draw(state: &AppState, w: u16, h: u16) -> (Buffer, Vec<(BorderControl, Rect)>) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let views = controls_for(state);
    let title_style = state.colors.theme.get("story_title").style;
    let segs = [InsetSegment { text: &state.pane_title, active: false }];
    // The same seam `main::draw_story_panel` goes through, zero-area filter and
    // all — not a copy of it (SQ-1148).
    let (_, hits) = draw_pane_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip { segments: &segs, base: title_style, active: title_style }),
            body_fill: None,
        },
        &state.colors.theme,
        &views,
    );
    (buf, hits)
}

/// Draw the MAP panel the way `main`'s split layout does — the layer-tab strip on
/// its top border and its own five controls on its bottom one (SQ-1148).
fn draw_map(state: &AppState, view: MapView, w: u16, h: u16) -> (Buffer, Vec<(BorderControl, Rect)>) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let views = map_controls_for(state, view);
    let segs = [InsetSegment { text: "Main", active: true }];
    let (_, hits) = draw_pane_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip {
                segments: &segs,
                base: state.colors.theme.get("panel.tab").style,
                active: state.colors.theme.get("panel.tab:active").style,
            }),
            body_fill: None,
        },
        &state.colors.theme,
        &views,
    );
    (buf, hits)
}

/// Every control there is. A list rather than a match on the enum because the
/// enum has no iterator; a control added without a line here escapes the two
/// registry guards below, which is the only way those guards can go stale.
const EVERY_CONTROL: [BorderControl; 7] = [
    BorderControl::Map,
    BorderControl::Guidance,
    BorderControl::VerbPanel,
    BorderControl::V6Render,
    BorderControl::V6PixelLock,
    BorderControl::ReturnProbe,
    BorderControl::Reveal,
];

/// The MAP pane's cluster, in the order it is drawn (SQ-1148).
const EVERY_MAP_CONTROL: [BorderControl; 5] = [
    BorderControl::RoomNumbers,
    BorderControl::Centre,
    BorderControl::ZoomOut,
    BorderControl::ZoomIn,
    BorderControl::ViewMode,
];

/// Both clusters, for the guards that are about the MECHANISM rather than about
/// either pane — the registry lookups, the hints, the persistence declaration.
/// One mechanism means one list; a control that reaches only one of these two
/// arrays escapes half of them.
fn every_control_anywhere() -> Vec<BorderControl> {
    EVERY_CONTROL.iter().chain(EVERY_MAP_CONTROL.iter()).copied().collect()
}

/// One buffer row as a string.
fn row(buf: &Buffer, y: u16) -> String {
    (buf.area.x..buf.area.right())
        .map(|x| buf.cell((x, y)).unwrap().symbol().to_owned())
        .collect()
}

/// A state with a title, a known z-version and every toggle explicitly OFF, so
/// the border has something to centre, the v6 gate has something to read, and no
/// case here silently depends on what `AppState::default()` happens to switch on
/// (it opens with the map shown and the Guiding Light lit).
fn story(zversion: Option<u8>) -> AppState {
    let mut st = AppState::default();
    st.pane_title = "ZORK I".into();
    st.story_zversion = zversion;
    st.layout = Layout::TranscriptFull;
    st.config.guidance = false;
    st.config.return_probe = false; // on by default since SQ-1215; this fixture is the all-off row
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.v6_pixel_lock = false;
    st
}

/// Light a word reveal, the way a press of the reveal control does — without an
/// engine, since nothing here is running a story. The trigger's control lights
/// for exactly as long as this is up (SQ-1107).
fn light_reveal(state: &mut AppState) {
    state.reveal = Some(app::reveal::Reveal {
        words: ["lantern".to_string()].into_iter().collect(),
        until: std::time::Instant::now() + app::reveal::REVEAL_HOLD,
    });
}

fn open_band(state: &mut AppState) {
    state.overlays.command_band = Some(CommandBandState::new(
        app::render::command_band::default_verbs(),
        app::render::command_band::default_quick(),
    ));
    state.band_dock.toggle_to(true, true);
}

// ── The border, drawn ────────────────────────────────────────────────────────

/// Where each control sits, printed. The bottom border carries the two switches
/// centred and the map toggle at its right-hand end; the top border carries the
/// v6 pair and, off v6, nothing at all.
#[test]
fn the_controls_ride_the_border_nearest_what_they_switch() {
    let plain = story(Some(3));
    let (buf, hits) = draw(&plain, 44, 6);
    let (top, bottom) = (row(&buf, 0), row(&buf, 5));
    println!("z3 top:    {top}");
    println!("z3 bottom: {bottom}");
    assert_eq!(hits.len(), 5, "off v6: map, probe, guidance, command band, reveal");

    // Bottom: `┤○ ▲ ◈├` centred, `┤◌ ◀├` anchored right, one corner clear of each.
    assert!(bottom.contains("┤○ ▲ ◈├"), "the centred group: {bottom:?}");
    assert!(bottom.ends_with("┤◌ ◀├┘"), "the anchored pair takes the right end, map at the corner: {bottom:?}");
    // Off v6 the top border carries NO cluster — the two v6 switches are the only
    // controls that ever live there — so nothing is reserved and the title strip
    // is centred across the WHOLE row. (Which is a behaviour change of its own:
    // the first pass reserved eleven columns on every story, v6 or not, and
    // `render_overflow` clipped long titles against that. SQ-1127.)
    assert!(top.contains("ZORK I"), "the title still fits: {top:?}");
    for g in ['◀', '○', '▲', '◈', '◌', '◧', '□', '▶', '●', '▼', '■', '▣', '▦'] {
        assert!(!top.contains(g), "z3 top border must carry no control, found {g:?}: {top:?}");
    }
    let dashes = |t: &str, part: &str| t.split(part).map(|p| p.matches('─').count()).collect::<Vec<_>>();
    let d = dashes(&top, "┤ ZORK I ├");
    assert_eq!(d[0], d[1], "off v6 the title is centred across the whole row: {top:?}");

    let v6 = story(Some(6));
    let (buf, hits) = draw(&v6, 44, 6);
    let (top, bottom) = (row(&buf, 0), row(&buf, 5));
    println!("z6 top:    {top}");
    println!("z6 bottom: {bottom}");
    assert_eq!(hits.len(), 7, "on v6: the render mode and the pixel lock join");
    assert!(top.contains("┤◧ □├"), "the v6 pair keeps the top border: {top:?}");
    // …and now that the cluster IS reserved, the title is centred in what is left
    // of the row rather than in the row: fewer dashes on its left than its right.
    let d = dashes(&top, "┤ ZORK I ├");
    assert!(d[0] < d[1], "the v6 cluster's columns come out of the title's: {top:?}");
    assert!(bottom.contains("┤○ ▲ ◈├"), "…and the bottom row is unchanged by it: {bottom:?}");
    assert!(bottom.ends_with("┤◌ ◀├┘"), "{bottom:?}");
}

/// Nothing is ever drawn on the story pane's RIGHT border column, which is where
/// the vertical splitter is dragged (`story.right() - 1`, two columns wide with
/// the map pane's own left border). The map toggle is anchored one column inside
/// it, against the corner.
#[test]
fn no_control_lands_on_the_splitters_column() {
    let st = story(Some(6));
    let (buf, hits) = draw(&st, 44, 20);
    let right = buf.area.right() - 1;
    for (id, r) in &hits {
        assert!(r.right() <= right, "{id:?} at {r:?} reaches the splitter column {right}");
    }
    // …and the border column itself is still an unbroken run of frame.
    for y in 1..19u16 {
        assert_eq!(buf.cell((right, y)).unwrap().symbol(), "│", "row {y} of the right border");
    }
}

/// Every control draws a DIFFERENT glyph in its other state — a control that
/// looks the same on and off is half a control. Both borders are printed.
#[test]
fn every_control_changes_glyph_with_its_state() {
    let mut on = story(Some(6));
    on.layout = Layout::Split;
    on.config.guidance = true;
    open_band(&mut on);
    on.config.v6_render = app::config::V6RenderMode::Raster;
    on.config.v6_pixel_lock = true;
    // The reveal has no second GLYPH — it is a trigger, so its state is carried
    // by colour alone (see the case below). Lit here so the row is the every-on
    // row it claims to be.
    light_reveal(&mut on);
    on.config.return_probe = true;

    let off = story(Some(6));

    let (on_buf, _) = draw(&on, 44, 6);
    let (off_buf, _) = draw(&off, 44, 6);
    for (tag, b) in [("on", &on_buf), ("off", &off_buf)] {
        println!("z6 {tag:>3} top:    {}", row(b, 0));
        println!("z6 {tag:>3} bottom: {}", row(b, 5));
    }

    // Map shown → ▶ (click and it leaves to the right); hidden → ◀.
    // Guidance lit → ●, out → ○. Band open → ▼ (click and it drops), closed → ▲.
    // Raster → ■ / hybrid → ◧. Lock on → ▣ / off → □.
    // The reveal is ◈ and the return probe ◌ in both rows: a trigger has no other
    // mode to draw, and the probe has no other mode at all.
    assert!(row(&on_buf, 5).contains("┤● ▼ ◈├"), "every-on bottom: {:?}", row(&on_buf, 5));
    assert!(row(&on_buf, 5).ends_with("┤◌ ▶├┘"), "every-on bottom: {:?}", row(&on_buf, 5));
    assert!(row(&on_buf, 0).contains("┤■ ▣├"), "every-on top: {:?}", row(&on_buf, 0));
    assert!(row(&off_buf, 5).contains("┤○ ▲ ◈├"), "every-off bottom: {:?}", row(&off_buf, 5));
    assert!(row(&off_buf, 5).ends_with("┤◌ ◀├┘"), "every-off bottom: {:?}", row(&off_buf, 5));
    assert!(row(&off_buf, 0).contains("┤◧ □├"), "every-off top: {:?}", row(&off_buf, 0));

    // …and the third render mode is a third glyph, not a repeat of either.
    let mut ext = story(Some(6));
    ext.config.v6_render = app::config::V6RenderMode::Extended;
    let (ext_buf, _) = draw(&ext, 44, 6);
    println!("z6 ext top:    {}", row(&ext_buf, 0));
    assert!(row(&ext_buf, 0).contains("┤▦ □├"), "extended top: {:?}", row(&ext_buf, 0));
}

/// **Every control that is ON is lit yellow**, and it gets that yellow from the
/// theme's `alert` role — the same slot `transcript_assist` uses — not from a
/// hard-coded colour. Restyling the role must move all of them with it.
///
/// So the state is carried TWICE: by the glyph and by the colour. That is
/// deliberate. A player who cannot tell the two colours apart still has the
/// shape, and the shape change is legible at a glance without reading colour.
///
/// **One control coming will break the first half of that, and it was decided
/// with the trade in view** (SQ-1148). The map pane's room-numbers toggle draws
/// `#` in BOTH states and lets colour alone say which one is in force — the
/// first TWO-MODE control here that does not change shape. The two single-glyph
/// controls beside it, `return_probe` and `reveal`, are exempt because they have
/// no opposite mode to draw; room numbers do have one, so this is a real
/// departure and not their rationale inherited. It was taken for coverage and
/// legibility: `#` is ASCII and so cannot tofu in any face, where every plain
/// mark that says "number" by shape is carried by at most fourteen of the
/// sixteen terminal faces surveyed, and the ones inside Geometric Shapes by as
/// few as five.
///
/// **What makes colour-only survivable here is that it is not colour-only**, and
/// this case is what holds that: `panel.control:lit` adds BOLD on top of the
/// `alert` hue, asserted below, so a colour-blind player or a low-contrast theme
/// still gets a WEIGHT change rather than nothing. The default pair is a
/// brightness step too — `muted` is DarkGray and `alert` is Yellow, which
/// separate by luminance in every standard ANSI palette, not merely by hue. If a
/// later edit ever drops that BOLD, room numbers become genuinely unreadable to
/// those players, which is why the assertion is worth keeping even though it
/// looks redundant beside the colour check.
#[test]
fn every_on_state_is_lit_from_the_alert_role_and_every_off_state_is_muted() {
    let alert = AppState::default().colors.theme.get("alert").style.fg.unwrap();
    let muted = AppState::default().colors.theme.get("muted").style.fg.unwrap();

    let mut on = story(Some(6));
    on.layout = Layout::Split;
    on.config.guidance = true;
    open_band(&mut on);
    on.config.v6_render = app::config::V6RenderMode::Raster;
    on.config.v6_pixel_lock = true;
    // The trigger has no on STATE; it lights while its reveal is up, which is the
    // click's own acknowledgement rather than a state report (SQ-1107).
    light_reveal(&mut on);
    on.config.return_probe = true;
    let (buf, hits) = draw(&on, 44, 6);
    println!("all on  top: {} / bottom: {}", row(&buf, 0), row(&buf, 5));
    for (id, r) in &hits {
        let cell = buf.cell((r.x, r.y)).unwrap();
        assert_eq!(cell.fg, alert, "{id:?} is on and must be lit");
        assert!(cell.modifier.contains(Modifier::BOLD), "{id:?}: panel.control:lit is bold");
    }

    // …and off, every one of them is the quiet `panel.control`.
    let off = story(Some(6));
    let (buf, hits) = draw(&off, 44, 6);
    println!("all off top: {} / bottom: {}", row(&buf, 0), row(&buf, 5));
    for (id, r) in &hits {
        assert_eq!(buf.cell((r.x, r.y)).unwrap().fg, muted, "{id:?} is off and must be muted");
    }

    // The render mode is a CYCLE, not a switch, so "on" needs a reading: hybrid
    // is how the game arrives and is not lit; the other two are choices the
    // player made, and both are.
    for (mode, want_lit) in [
        (app::config::V6RenderMode::Hybrid, false),
        (app::config::V6RenderMode::Raster, true),
        (app::config::V6RenderMode::Extended, true),
    ] {
        let mut st = story(Some(6));
        st.config.v6_render = mode;
        let (buf, hits) = draw(&st, 44, 6);
        let (_, r) = hits.iter().find(|(id, _)| *id == BorderControl::V6Render).unwrap();
        let fg = buf.cell((r.x, r.y)).unwrap().fg;
        assert_eq!(fg == alert, want_lit, "{mode:?} lit? expected {want_lit}");
    }
}

/// A hovered control takes `panel.control:hover`, so whatever the pointer is on
/// always reads as reachable — even the ones that are otherwise idle.
#[test]
fn the_hovered_control_is_highlighted() {
    let mut st = story(Some(3));
    let (_, hits) = draw(&st, 44, 6);
    let (_, r) = hits.iter().find(|(id, _)| *id == BorderControl::VerbPanel).unwrap();
    let (r_x, r_y) = (r.x, r.y);

    st.control_hover = Some(BorderControl::VerbPanel);
    let (buf, _) = draw(&st, 44, 6);
    let cell = buf.cell((r_x, r_y)).unwrap();
    assert!(
        cell.modifier.contains(Modifier::REVERSED),
        "panel.control:hover is reversed by default",
    );
}

// ── The hint ─────────────────────────────────────────────────────────────────

/// The hover hint says what the control is, what a click does, and how to do the
/// same from the keyboard — and it goes INTO the pane, away from the icon being
/// pointed at. Guidance rides the BOTTOM border now, so "into the pane" is
/// upwards: a hint that still dropped one row would land in the command band.
#[test]
fn the_hint_names_the_control_and_sits_inside_the_pane() {
    let mut st = story(Some(3));
    st.config.guidance = false;
    st.control_hover = Some(BorderControl::Guidance);

    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    let views = controls_for(&st);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let segs = [InsetSegment { text: "ZORK I", active: false }];
    let title = st.colors.theme.get("story_title").style;
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip { segments: &segs, base: title, active: title }),
            body_fill: None,
        },
        &ctls,
        &st.colors.theme,
    );
    let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
    let anchor = hits.iter().find(|(id, _)| *id == BorderControl::Guidance).unwrap().1;

    let tip = draw_control_hint(&mut buf, area, &st, &views, &hits).expect("the hint is drawn");
    assert!(tip.bottom() <= anchor.y, "a bottom-border hint rises into the pane, never over it");
    assert_eq!(row(&buf, anchor.y).chars().nth(anchor.x as usize), Some('○'),
               "…and the control itself is still visible");

    let text: String = (tip.y..tip.bottom()).map(|y| row(&buf, y)).collect();
    println!("hint: {}", (tip.y..tip.bottom()).map(|y| row(&buf, y)).collect::<Vec<_>>().join(" / "));
    assert!(text.contains("Guiding Light: off"), "the hint states the state: {text:?}");
    assert!(text.contains("click to light it"), "…and what a click does: {text:?}");
    assert!(text.contains("/set-guidance"), "…and the command that does the same: {text:?}");
}

/// Near the right edge the hint slides LEFT rather than off the screen, and near
/// the bottom it flips ABOVE the control. Neither may panic, and neither may
/// draw outside the pane.
#[test]
fn the_hint_stays_inside_the_pane_at_both_edges() {
    let mut st = story(Some(6));
    // The two hardest anchors: the top border's right-most control (slides left,
    // drops down) and the map toggle in the BOTTOM-RIGHT corner, which has to
    // slide left AND rise, with nothing below it to fall into.
    for anchor_id in [BorderControl::V6PixelLock, BorderControl::Map] {
    st.control_hover = Some(anchor_id);

    for (w, h) in [(44u16, 8u16), (30, 4), (26, 3)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let views = controls_for(&st);
        let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
        let (_, rects) = draw_panel_with_controls(
            &mut buf,
            &PanelSpec {
                area,
                border_selector: "panel.border",
                border_color: None,
                border_style: None,
                glyphs: &PaneGlyphs::default(),
                header_on: true,
                strip: None,
                body_fill: None,
            },
            &ctls,
            &st.colors.theme,
        );
        let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
        if let Some(tip) = draw_control_hint(&mut buf, area, &st, &views, &hits) {
            assert!(tip.right() <= area.right(), "{anchor_id:?} {w}x{h}: ran off the right edge");
            assert!(tip.bottom() <= area.bottom(), "{anchor_id:?} {w}x{h}: ran off the bottom");
            assert!(tip.x >= area.x && tip.y >= area.y, "{anchor_id:?} {w}x{h}: ran off the top-left");
        }
    }
    }
}

// ── Hit-testing and dispatch ─────────────────────────────────────────────────

/// The click path and the hover path resolve through ONE function against ONE
/// list of rects, so they can never disagree about what is under the pointer.
#[test]
fn a_click_and_a_hover_resolve_to_the_same_control() {
    let st = story(Some(6));
    let (_, hits) = draw(&st, 44, 6);
    for (id, r) in &hits {
        assert_eq!(control_at(&st, &hits, r.x, r.y), Some(*id));
    }
    // A cell one row up from a bottom control (inside the pane) is not a control.
    let guide = hits.iter().find(|(id, _)| *id == BorderControl::Guidance).unwrap().1;
    assert_eq!(control_at(&st, &hits, guide.x, guide.y - 1), None);
    // …nor is the separator column between two controls.
    assert_eq!(control_at(&st, &hits, guide.x + 1, guide.y), None);
}

/// Every control drives an existing `slash::COMMANDS` entry, and the LINE a
/// click puts through the pipeline parses. A control that named a command the
/// registry does not have would silently do nothing; this is the guard, because
/// nothing structural stops the string drifting.
///
/// **Both halves of [`app::render::controls::ControlCommand`] are checked, and
/// they check different things** (SQ-1148). The registry is keyed by `name`
/// alone, so that half must be a real entry; and what a click actually runs is
/// the whole line, so THAT is what must parse. `zoom-map` is the case that
/// separates them: it is a real entry whose bare form is an error, and the two
/// zoom controls reach it with an argument each. Checking only the name would
/// pass a control that does nothing when clicked; checking only the line would
/// pass one whose `Context` lookup silently fell back to `Global`.
#[test]
fn every_control_names_a_real_slash_command_and_its_click_line_parses() {
    for id in every_control_anywhere() {
        let cmd = id.command();
        let spec = app::slash::COMMANDS.iter().find(|c| c.name == cmd.name).unwrap_or_else(|| {
            panic!("{id:?} names {:?}, which is not in slash::COMMANDS", cmd.name)
        });
        let line = cmd.to_string();
        let outcome = app::slash::parse_in_context(&line, '/', spec.context);
        assert!(
            !matches!(outcome, app::slash::SlashOutcome::Error(_)),
            "`{line}` is an error, so a click on {id:?} would do nothing: {outcome:?}",
        );
    }
    // …and the widening is load-bearing rather than decorative: exactly the two
    // zoom controls carry an argument, and the entry they share REFUSES the bare
    // form, which is why `command()` could not stay a plain string.
    let with_args: Vec<_> =
        every_control_anywhere().into_iter().filter(|c| c.command().arg.is_some()).collect();
    assert_eq!(with_args, vec![BorderControl::ZoomOut, BorderControl::ZoomIn], "{with_args:?}");
    assert!(
        matches!(
            app::slash::parse("zoom-map", '/'),
            app::slash::SlashOutcome::Error(_)
        ),
        "if bare `zoom-map` ever became valid, this test stops proving anything",
    );
}

/// Every control says which PANE it rides, and the two lists agree with it
/// (SQ-1148). `ControlPlacement` is an anchor on a frame and cannot say which
/// frame; without this, a map control listed by `controls_for` would be drawn on
/// the story pane's border and the failure would be a layout oddity rather than
/// an error.
#[test]
fn every_control_is_drawn_on_the_pane_it_says_it_rides() {
    let st = story(Some(6));
    for v in controls_for(&st) {
        assert_eq!(v.id.pane(), ControlPane::Story, "{:?} is in the story pane's list", v.id);
    }
    for v in map_controls_for(&st, MapView::Drawn) {
        assert_eq!(v.id.pane(), ControlPane::Map, "{:?} is in the map pane's list", v.id);
    }
    for id in EVERY_CONTROL {
        assert_eq!(id.pane(), ControlPane::Story, "{id:?}");
    }
    for id in EVERY_MAP_CONTROL {
        assert_eq!(id.pane(), ControlPane::Map, "{id:?}");
    }
}

/// **What a click switches, it also remembers.** Every control's command
/// persists its result in the per-game `config.toml` sidecar, so a preference
/// chosen for one story stays with that story and no other.
///
/// This reads the registry's own description rather than the behaviour, because
/// behaviour is what each command's own dispatch case already pins — what this
/// catches is a control being added later whose command does not persist, which
/// would look identical on screen and quietly forget itself. Two commands changed
/// semantics to make this true: `set-v6-render` and `set-guidance` were both
/// session-only, and are now per-game like the pixel lock beside them.
///
/// **The reveal is the one exception, and it is stated rather than skipped**
/// (SQ-1107). It is a TRIGGER: there is nothing to remember about a light that
/// was on for four seconds, and `BorderControl::persists` is where that is
/// declared — so a future control added without a thought about persistence
/// still fails here, and only a control whose author wrote `persists() == false`
/// is exempt.
#[test]
fn every_control_switches_something_that_is_remembered_per_game() {
    for id in every_control_anywhere() {
        let name = id.command().name;
        let spec = app::slash::COMMANDS.iter().find(|c| c.name == name).unwrap();
        if !id.persists() {
            assert!(
                !spec.description.contains("persisted per-game"),
                "{id:?} says it persists nothing, but `{name}` promises to remember: {:?}",
                spec.description,
            );
            continue;
        }
        assert!(
            spec.description.contains("persisted per-game"),
            "{id:?} runs `{name}`, whose description does not promise to remember it: {:?}",
            spec.description,
        );
    }
}

/// The trigger is not a switch, and the difference is worth pinning: it names a
/// command that takes no argument and stores nothing, it has ONE glyph in every
/// state, and its hint has to say what a press DOES because the glyph cannot.
#[test]
fn the_reveal_is_a_trigger_and_says_so() {
    assert!(!BorderControl::Reveal.persists(), "a trigger has nothing to remember");
    assert!(
        EVERY_CONTROL.iter().filter(|c| !c.persists()).count() == 1,
        "the reveal is still the only trigger on the STORY pane; a second one \
         wants its own thinking",
    );
    // The map pane's five persist nothing either, and each for its own reason
    // rather than as a group (SQ-1148) — see `BorderControl::persists`. The one
    // worth restating is the view switch: it DOES remember, per layer, in the map
    // archive, which is a different store from the per-game sidecar this method
    // is about, so `false` here is a statement about the sidecar and not a claim
    // that a click is forgotten.
    assert!(
        EVERY_MAP_CONTROL.iter().all(|c| !c.persists()),
        "no map control writes the per-game sidecar",
    );

    // Every other control's hint is two lines — a state and its opposite, then
    // the command. This one needs three: the glyph says nothing about WHAT it
    // lights, so the hint has to.
    let st = story(Some(3));
    let views = controls_for(&st);
    let reveal = views.iter().find(|v| v.id == BorderControl::Reveal).expect("drawn");
    let text = reveal.hint.join(" / ");
    println!("reveal hint: {text}");
    assert!(text.contains("light the nouns and named things on screen"), "it says what it does: {text:?}");
    assert!(text.contains("/reveal-words"), "…and how to do it from the keyboard: {text:?}");
    // Guidance is out in `story()`, and a press would then do nothing at all —
    // which the hint has to say, or the player concludes the button is broken.
    assert!(text.contains("Guiding Light"), "…and why a press will do nothing: {text:?}");

    let mut lit = story(Some(3));
    lit.config.guidance = true;
    let on = controls_for(&lit);
    let reveal = on.iter().find(|v| v.id == BorderControl::Reveal).unwrap();
    assert!(
        !reveal.hint.join(" / ").contains("Guiding Light"),
        "with the light on there is nothing to warn about: {:?}",
        reveal.hint,
    );
}

/// **Every key and every command a hint names must actually reach that control.**
///
/// This case exists because the one beside it could not see the defect that
/// produced it (SQ-1142). `the_reveal_is_a_trigger_and_says_so` asserts
/// `text.contains("/reveal-words")`, and the hint `"F4 · /reveal-words"`
/// satisfies that perfectly — so when SQ-1142 unbound F2, F3 and F4, the reveal
/// went on advertising a dead key and the suite stayed green; the command band's
/// hint was the bare string `"F2"` and was not checked for a command at all. A
/// substring check on the half that is right cannot fail on the half that is
/// wrong.
///
/// So the hints are checked against the KEYMAP and the REGISTRY rather than
/// against literals: a token that looks like a key must be a route the keymap or
/// the leader panel really binds to this control's own command, and a token that
/// looks like a slash command must be in `slash::COMMANDS` and be this control's
/// command. Unbinding a key now fails the hint that advertises it, which is the
/// whole point — the next person to remove a binding has no reason to know which
/// tooltips recite it.
#[test]
fn no_hint_advertises_a_key_that_is_not_bound() {
    // A token is a key label if it carries a modifier or is Fn — the two shapes
    // a hint has ever used. Anything else is prose and is not checked.
    fn looks_like_a_key(tok: &str) -> bool {
        tok.starts_with("Ctrl+")
            || tok.starts_with("Alt+")
            || tok.starts_with("Shift+")
            || (tok.len() >= 2
                && tok.starts_with('F')
                && tok[1..].chars().all(|c| c.is_ascii_digit()))
    }

    let st = story(Some(6));
    let mut views = controls_for(&st);
    assert_eq!(views.len(), EVERY_CONTROL.len(), "a v6 story draws every control");
    // Both clusters: one mechanism means one hint guard, and a map control that
    // reached only the map's own cases would escape this one entirely (SQ-1148).
    views.extend(map_controls_for(&st, MapView::Drawn));
    assert_eq!(views.len(), every_control_anywhere().len());

    for view in &views {
        let cmd = view.id.command();
        let line = cmd.to_string();

        // The routes that genuinely reach this control from the keyboard: its
        // direct binding, and its leader-panel letter behind the prefix. An
        // argument-bearing control is matched on the WHOLE line, because `+` is
        // bound to `zoom-map in` and would otherwise read as a route to
        // `zoom-map out` as well.
        let mut routes: Vec<String> = Vec::new();
        let direct = match cmd.arg {
            Some(_) => st.keymap.primary_key_exact(&line),
            None => st.keymap.primary_key(cmd.name),
        };
        if let Some(k) = direct {
            routes.push(k.label());
        }
        if let Some((letter, _, _)) =
            st.hotkeys.groups.iter().flat_map(|(_, entries)| entries.iter()).find(|(_, c, _)| {
                match cmd.arg {
                    Some(_) => *c == line,
                    None => c.split_whitespace().next() == Some(cmd.name),
                }
            })
        {
            routes.push(format!("{} {letter}", st.hotkeys.prefix.label()));
        }

        for line in &view.hint {
            for tok in line.split_whitespace() {
                if looks_like_a_key(tok) {
                    assert!(
                        routes.iter().any(|r| r == tok || r.starts_with(&format!("{tok} "))),
                        "{:?}'s hint advertises the key {tok:?}, which does not reach \
                         `{line}`. Routes that do: {routes:?}. Hint: {:?}",
                        view.id,
                        view.hint,
                    );
                }
                if let Some(name) = tok.strip_prefix('/') {
                    assert!(
                        app::slash::find_command(name).is_some(),
                        "{:?}'s hint names /{name}, which is not in slash::COMMANDS: {:?}",
                        view.id,
                        view.hint,
                    );
                    assert_eq!(
                        name, cmd.name,
                        "{:?}'s hint names /{name} but a click runs `{line}`: {:?}",
                        view.id, view.hint,
                    );
                }
            }
        }
    }
}

/// A modal overlay owns the screen: while one is open the border controls are
/// unreachable by both click and hover, so a stray pointer cannot toggle
/// anything behind a dialog.
#[test]
fn a_modal_overlay_takes_the_controls_out_of_reach() {
    let mut st = story(Some(3));
    let (_, hits) = draw(&st, 44, 6);
    let (_, r) = hits[0];
    assert!(control_at(&st, &hits, r.x, r.y).is_some());
    st.overlays.quit_dialog = true;
    assert!(st.any_modal_overlay_open(), "the quit dialog is modal");
    assert_eq!(control_at(&st, &hits, r.x, r.y), None);
}

// ── Geometry ─────────────────────────────────────────────────────────────────

/// The cluster's columns come OUT of the title's before the title is centred, so
/// a title long enough to reach the controls is trimmed by the strip's own
/// overflow rules instead of being painted over them.
#[test]
fn a_long_title_never_overwrites_a_control() {
    let mut st = story(Some(6));
    st.pane_title = "A VERY LONG ADVENTURE TITLE INDEED".into();
    for w in 30..=60u16 {
        let (buf, hits) = draw(&st, w, 5);
        let r = row(&buf, 0);
        for (id, rect) in &hits {
            let sym = buf.cell((rect.x, rect.y)).unwrap().symbol().to_owned();
            let view = controls_for(&st).into_iter().find(|v| v.id == *id).unwrap();
            assert_eq!(sym, view.glyph.to_string(), "w={w}: title overwrote {id:?} — {r:?}");
        }
    }
}

/// Each group is drawn WHOLE or not at all, and the groups give way in a fixed
/// order as the pane narrows. A half cluster is unclickable chrome.
///
/// The map toggle is anchored and the pair is centred in what the anchor leaves,
/// so **the centred pair is what gives way first** — and the map toggle, the one
/// control that moves a whole pane, survives longest. The printed rows are the
/// record of where each threshold actually falls.
#[test]
fn the_groups_drop_whole_and_the_centred_pair_gives_way_first() {
    let st = story(Some(6));
    let has = |hits: &[(BorderControl, Rect)], id: BorderControl| {
        hits.iter().any(|(i, _)| *i == id)
    };
    let mut seen: Vec<(u16, bool, bool, bool)> = Vec::new();
    for w in 4..=24u16 {
        let (buf, hits) = draw(&st, w, 5);
        let map = has(&hits, BorderControl::Map);
        let pair = has(&hits, BorderControl::Guidance);
        let v6 = has(&hits, BorderControl::V6Render);
        // Guidance, the command band and the reveal are one group: never one of
        // them without the others.
        assert_eq!(pair, has(&hits, BorderControl::VerbPanel), "w={w}: half the centred group");
        assert_eq!(pair, has(&hits, BorderControl::Reveal), "w={w}: half the centred group");
        assert_eq!(v6, has(&hits, BorderControl::V6PixelLock), "w={w}: half the v6 pair");
        // The pair can never outlive the map toggle it has to make room for.
        assert!(!(pair && !map), "w={w}: the centred pair survived the anchored one");
        println!("w={w:>2} map={map:<5} pair={pair:<5} v6={v6:<5}  {} | {}", row(&buf, 0), row(&buf, 4));
        seen.push((w, map, pair, v6));
    }
    // The thresholds, pinned: 3 columns for the map toggle alone plus a spare, 7
    // for `┤○ ▲ ◈├` plus a clear column past the anchored pair, and 5 for the v6
    // pair plus a spare. The centred group cost two more columns when the reveal
    // joined it and two more again when the probe joined the anchored pair it has
    // to clear (SQ-1107) — the price of a group being drawn whole or not at all.
    // Where the ANCHORED pair sheds its own inboard member is
    // `return_probe.rs::the_map_toggle_outlives_the_probe_as_the_pane_narrows`.
    let first = |f: fn(&(u16, bool, bool, bool)) -> bool| seen.iter().find(|r| f(r)).unwrap().0;
    assert_eq!(first(|r| r.1), 7, "the map toggle alone needs a 7-column pane");
    assert_eq!(first(|r| r.2), 20, "the centred group needs 20");
    assert_eq!(first(|r| r.3), 9, "the top border's v6 pair needs 9");
}

/// A pane with no bottom border row to put them on draws no bottom controls —
/// and does not panic reaching for one.
#[test]
fn a_pane_with_no_room_for_a_bottom_border_draws_no_bottom_controls() {
    let st = story(Some(6));
    for h in 1..=3u16 {
        let (_, hits) = draw(&st, 44, h);
        for (id, r) in &hits {
            assert!(r.y < h, "h={h}: {id:?} at {r:?} is off the buffer");
        }
    }
}

/// …and the hint follows the same rule as the click: a modal that opens while
/// the pointer is resting on a control must not leave a hint floating over the
/// dialog. The hint is drawn after the overlay ladder, so this is its own guard
/// rather than a consequence of the hit test.
#[test]
fn a_modal_overlay_also_suppresses_the_hint() {
    let mut st = story(Some(3));
    st.control_hover = Some(BorderControl::Map);
    let area = Rect::new(0, 0, 50, 8);
    let mut buf = Buffer::empty(area);
    let views = controls_for(&st);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: None,
            body_fill: None,
        },
        &ctls,
        &st.colors.theme,
    );
    let hits: Vec<_> = views.iter().map(|v| v.id).zip(rects).collect();
    assert!(draw_control_hint(&mut buf, area, &st, &views, &hits).is_some(), "…normally it draws");
    st.overlays.quit_dialog = true;
    assert!(draw_control_hint(&mut buf, area, &st, &views, &hits).is_none());
}

// ── The MAP pane's cluster (SQ-1148) ─────────────────────────────────────────
//
// **Why these cases exist at all, stated where the next author will read it.**
// Three gated, reviewed rounds shipped this feature's whole vocabulary — the
// glyphs, both presets, the seven override keys, the font-check row, the docs —
// and not one control, and every round closed reporting "border_controls 18/18".
// That is the story pane's suite. It passes identically whether or not a map
// cluster exists, so it could not have caught it and did not. A feature whose
// only guard is a suite named after a NEIGHBOURING feature has no guard.
//
// So the first case below is a NON-VACUITY case: it fails when the map pane
// draws no cluster, which is the exact defect that shipped.

/// The Adventure fixture, which carries a second layer to rule on.
fn advent() -> mapper::mapper::Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    mapper::persist::from_json(&std::fs::read_to_string(&path).expect("fixture")).expect("map")
}

/// The map pane draws its five controls on its own bottom border, and every one
/// of them is clickable.
///
/// The list is spelled out as glyphs rather than counted, because a count passes
/// on five of anything: `┤# ¤ − + M├` in the plain preset, in the order the
/// cluster was designed in — numbers · centre · out · in · view.
#[test]
fn the_map_pane_draws_its_own_control_cluster() {
    let st = story(Some(3));
    let (buf, hits) = draw_map(&st, MapView::Drawn, 44, 8);
    let (top, bottom) = (row(&buf, 0), row(&buf, 7));
    println!("map top:    {top}");
    println!("map bottom: {bottom}");

    assert_eq!(hits.len(), 5, "five controls, all on screen: {hits:?}");
    assert_eq!(
        hits.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        EVERY_MAP_CONTROL.to_vec(),
        "…in the order the cluster is designed in",
    );
    assert!(bottom.contains("┤# ¤ − + M├"), "the map cluster: {bottom:?}");

    // It is on the BOTTOM border and nowhere else: the top one carries the layer
    // tabs, and a control that drifted up there would sit on them.
    assert!(top.contains("Main"), "the layer strip still has its row: {top:?}");
    // The cluster as a whole, not glyph by glyph: `M` is also the first letter
    // of the layer tab this pane's top border carries, so a per-mark scan reads
    // the tab strip as a stray control.
    assert!(!top.contains("¤"), "the map cluster must not reach the top border: {top:?}");
    assert!(!top.contains("# ¤ − + M"), "{top:?}");
    for (id, r) in &hits {
        assert_eq!(r.y, 7, "{id:?} at {r:?} is not on the bottom border");
        assert!(r.x > 0 && r.right() < 44, "{id:?} at {r:?} lands on a corner");
    }

    // …and the pointer can reach every one of them, which is the difference
    // between a cluster and a decoration.
    for (id, r) in &hits {
        assert_eq!(control_at(&st, &hits, r.x, r.y), Some(*id), "{id:?} at {r:?} is unclickable");
    }
}

/// **The production map pane goes through the same seam this suite draws
/// through.** The case above renders `map_controls_for` into a buffer, which
/// proves the cluster CAN be drawn; it cannot prove `main` draws it, and "the
/// vocabulary exists and nothing renders it" is exactly the failure this quest
/// was reopened for. So this reads the source: a guard beats a convention, and
/// the reversion it guards against — swapping the map pane back to a plain
/// `draw_panel` — is a one-line edit no behavioural test in this crate can see,
/// because `main` is a binary.
#[test]
fn main_draws_the_map_pane_through_the_control_seam() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
    )
    .expect("main.rs");
    // The map pane's own PanelSpec identifies the site (`area: pane_layout.map`),
    // and BOTH halves have to be in the lines that lead to it: the cluster is
    // resolved, and the call that carries the spec is the seam rather than the
    // plain `draw_panel` it used to be. Checking the whole file for
    // `map_controls_for` is not enough — the hover-hint path names it too, so a
    // map pane reverted to a bare border would still satisfy it, which is the
    // same "a neighbour's guard is not a guard" mistake in miniature.
    let at = src.find("area: pane_layout.map").expect("the map pane's PanelSpec");
    let before = &src[at.saturating_sub(500)..at];
    assert!(
        before.contains("map_controls_for("),
        "main.rs no longer resolves the map pane's cluster, so its border is bare: {before:?}",
    );
    assert!(
        before.contains("draw_pane_with_controls"),
        "the map pane's PanelSpec is not drawn through the control seam: {before:?}",
    );
}

/// Each control runs the command that does its job — asserted by putting the
/// click's own line through the slash pipeline and applying what comes back, so
/// the whole route is exercised and not just the string.
///
/// A click IS the command: there is no second implementation of any of these
/// five, which is what keeps a border control and a typed `/zoom-map in`
/// identical, including whatever either persists.
#[test]
fn every_map_control_runs_the_command_that_does_its_job() {
    let mut m = advent();
    // No pane size is set, so `apply_recenter` falls back to its own 80x24 —
    // which is all this case needs, since it asserts that the view MOVED.
    let mut st = AppState::default();
    // Name the layer being looked at, rather than inheriting whichever one the
    // fixture's current room happens to sit on: `/view-map` rules the ACTIVE
    // layer, and a case that does not say which is asserting about a layer it
    // did not choose.
    st.set_viewed_layer(Some(MAIN_LAYER));

    fn run(id: BorderControl, st: &mut AppState, m: &mut mapper::mapper::Mapper) {
        let cmd = id.command();
        let spec = app::slash::COMMANDS.iter().find(|c| c.name == cmd.name).unwrap();
        match app::slash::parse_in_context(&cmd.to_string(), '/', spec.context) {
            app::slash::SlashOutcome::Action(a) => app::input::apply_action(a, st, m),
            other => panic!("{id:?} → {other:?}"),
        }
    }

    // Room numbers: a boolean, and back again.
    assert!(!st.show_room_numbers, "the fixture starts with them hidden");
    run(BorderControl::RoomNumbers, &mut st, &mut m);
    assert!(st.show_room_numbers, "a click shows room numbers");
    run(BorderControl::RoomNumbers, &mut st, &mut m);
    assert!(!st.show_room_numbers, "…and the next click hides them again");

    // Zoom: two one-shots that move in opposite directions.
    st.zoom_reset();
    let start = st.zoom;
    run(BorderControl::ZoomOut, &mut st, &mut m);
    let zoomed_out = st.zoom;
    assert_ne!(zoomed_out, start, "zoom out moved the map");
    run(BorderControl::ZoomIn, &mut st, &mut m);
    assert_eq!(st.zoom, start, "…and zoom in brought it back, which one cycling slot could not");

    // Centre: a one-shot that puts the view back on the room in play.
    st.scroll = (999, 999);
    st.char_pan = (3, 4);
    run(BorderControl::Centre, &mut st, &mut m);
    assert_ne!(st.scroll, (999, 999), "a click re-centres the view");
    assert_eq!(st.char_pan, (0, 0), "…and drops the char-granular pan with it");

    // View: the mode, which writes the PER-LAYER override.
    assert_eq!(m.graph.layer_view_choice(MAIN_LAYER), None, "nobody has ruled on this layer yet");
    run(BorderControl::ViewMode, &mut st, &mut m);
    assert_eq!(m.graph.layer_view(MAIN_LAYER), MapView::Matrix, "a click switches the view");
}

/// **The view switch writes the per-layer override and never flattens the
/// unruled state into a value it merely inherited** (SQ-0666's model, which this
/// quest decided to use rather than replace).
///
/// `None` (nobody has ruled) and `Some(Drawn)` (someone ruled, and chose the
/// value the derivation would have given anyway) are different states: if the
/// layer later stops deriving `Drawn`, the ruled one must still be drawn. Drawing
/// the cluster reads `effective_view` and must not write anything at all — a
/// control that resolved the override in order to paint itself would silently
/// rule on every layer the player ever looked at.
#[test]
fn the_view_control_rules_one_layer_and_only_by_being_clicked() {
    let mut m = advent();
    let mut st = AppState::default();
    let maze: mapper::layer::LayerId = 1;
    st.set_viewed_layer(Some(maze));
    m.graph.set_layer_maze(maze, true);
    assert_eq!(m.graph.layer_view(maze), MapView::Matrix, "derived, not ruled");
    assert_eq!(m.graph.layer_view_choice(maze), None);

    // Drawing the cluster is a READ. Nothing about it may make a ruling.
    let _ = draw_map(&st, m.graph.layer_view(maze), 44, 8);
    assert_eq!(m.graph.layer_view_choice(maze), None, "rendering must not rule on a layer");

    // A click does rule, on THIS layer, and cycles from what is on screen.
    match app::slash::parse("view-map", '/') {
        app::slash::SlashOutcome::Action(a) => app::input::apply_action(a, &mut st, &mut m),
        other => panic!("{other:?}"),
    }
    assert_eq!(m.graph.layer_view_choice(maze), Some(MapView::Drawn), "the ruling is stored");
    assert_eq!(
        m.graph.layer_view_choice(MAIN_LAYER),
        None,
        "…and no other layer was ruled on with it",
    );
}

/// The two two-mode map controls carry their state, and the plain preset's `#`
/// and `M` carry it in COLOUR because ASCII has no off-shape for either.
///
/// **This is the one place in either cluster where the shape rule bends**, so it
/// is asserted rather than left to be noticed: the same glyph both ways, lit from
/// `alert` and BOLD when on, `muted` when off. The BOLD is what keeps it readable
/// without colour — see
/// `every_on_state_is_lit_from_the_alert_role_and_every_off_state_is_muted`,
/// which states the whole trade.
#[test]
fn room_numbers_and_the_view_switch_carry_their_state_in_colour() {
    let alert = AppState::default().colors.theme.get("alert").style.fg.unwrap();
    let muted = AppState::default().colors.theme.get("muted").style.fg.unwrap();

    let mut on = story(Some(3));
    on.show_room_numbers = true;
    let (buf, hits) = draw_map(&on, MapView::Matrix, 44, 8);
    println!("map on : {}", row(&buf, 7));
    assert!(row(&buf, 7).contains("┤# ¤ − + M├"), "the same marks in both states");
    for id in [BorderControl::RoomNumbers, BorderControl::ViewMode] {
        let (_, r) = hits.iter().find(|(i, _)| *i == id).unwrap();
        let cell = buf.cell((r.x, r.y)).unwrap();
        assert_eq!(cell.fg, alert, "{id:?} is on and must be lit");
        assert!(cell.modifier.contains(Modifier::BOLD), "{id:?}: :lit is bold, not merely yellow");
    }

    let mut off = story(Some(3));
    off.show_room_numbers = false;
    let (buf, hits) = draw_map(&off, MapView::Drawn, 44, 8);
    println!("map off: {}", row(&buf, 7));
    assert!(row(&buf, 7).contains("┤# ¤ − + M├"), "…which is why colour has to say which");
    for (id, r) in &hits {
        assert_eq!(buf.cell((r.x, r.y)).unwrap().fg, muted, "{id:?} is off and must be muted");
    }

    // The one-shots are never lit: they report no state, so a yellow one would be
    // claiming one.
    let mut mixed = story(Some(3));
    mixed.show_room_numbers = true;
    let (buf, hits) = draw_map(&mixed, MapView::Matrix, 44, 8);
    for id in [BorderControl::Centre, BorderControl::ZoomOut, BorderControl::ZoomIn] {
        let (_, r) = hits.iter().find(|(i, _)| *i == id).unwrap();
        assert_eq!(buf.cell((r.x, r.y)).unwrap().fg, muted, "{id:?} is a one-shot, not a switch");
    }
}

/// The patched preset keeps the shape rule outright: a distinct off-glyph for
/// both two-mode controls, because a nerd font has one and ASCII does not.
#[test]
fn the_patched_preset_changes_shape_where_the_plain_one_changes_colour() {
    let mut st = story(Some(3));
    st.symbols.map_controls =
        app::symbols::MapControlGlyphs::preset("nerdfont").expect("the patched preset");
    st.show_room_numbers = true;
    let (on, _) = draw_map(&st, MapView::Matrix, 44, 8);
    st.show_room_numbers = false;
    let (off, _) = draw_map(&st, MapView::Drawn, 44, 8);
    println!("patched on : {}", row(&on, 7));
    println!("patched off: {}", row(&off, 7));
    assert_ne!(row(&on, 7), row(&off, 7), "the patched cluster changes SHAPE with its state");
}

/// The map cluster is one group and stands or falls whole: a pane too narrow for
/// five controls draws none of them rather than a clickable half-cluster.
#[test]
fn the_map_cluster_is_drawn_whole_or_not_at_all() {
    let st = story(Some(3));
    let want = header_controls_width(EVERY_MAP_CONTROL.len());
    let mut ever_drawn = false;
    for w in 6..=30u16 {
        let (buf, hits) = draw_map(&st, MapView::Drawn, w, 8);
        assert!(hits.len() == 5 || hits.is_empty(), "w={w}: half a cluster: {hits:?}");
        if !hits.is_empty() {
            ever_drawn = true;
            assert!(w > want + 2, "w={w}: {want} columns of cluster do not fit");
            assert!(row(&buf, 7).contains("┤# ¤ − + M├"), "w={w}: {:?}", row(&buf, 7));
        }
    }
    assert!(ever_drawn, "a 30-column map pane is wide enough for five controls");
}

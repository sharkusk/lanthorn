//! Glulx debug-inspector adapter (SQ-0465): implements the engine-neutral
//! [`Debugger`] trait for [`GlulxSession`] on top of the `gvm` disassembler +
//! discovery cache ([`gvm::disasm`]).
//!
//! **Addressing.** Glulx is a real PC-based VM, so this mirrors the Z-machine
//! adapter far more closely than the Scott one: `pc()` is the machine's parked
//! instruction PC, the Disassembly section is a real windowed disassembly over
//! ROM (and executed RAM code), and `next_instr`/`prev_instr` step by whole
//! instructions.
//!
//! **Sections.** Glulx keeps every panel section but repurposes three of the
//! plain-list slots for Glulx-specific content, relabelled via
//! [`section_label`](Debugger::section_label):
//!
//! * Objects → **Functions** — every discovered function (entry, C0/C1, locals
//!   count, confidence tier, and an `[accel: …]` badge for accelerated ones).
//! * Dictionary → **Strings** — discovered string objects with a short preview.
//! * Globals → **Glk** — the live Glk window tree (the `/dump-windows` snapshot).
//!
//! Call Stack / Eval Stack / Locals / Memory keep their normal meaning (register-
//! machine state), rendered from the live call frames + value stack + RAM.
//!
//! **Cost.** The discovery cache is built lazily on the FIRST inspector read (see
//! [`GlulxSession::with_disasm_cache`]) and never otherwise — a closed inspector
//! on a normal launch touches none of this, and the VM's trace flag stays off, so
//! the interpreter hot loop is untouched.

use std::collections::HashSet;

use gvm::disasm::Tier;

use crate::debug_panel::Section;
use crate::engine::{Debugger, DisasmProvenance};
use crate::glulx_session::GlulxSession;

/// The Glulx inspector's tab layout: the full Z-machine set of windows, with
/// Objects/Dictionary/Globals repurposed for Functions/Strings/Glk (relabelled
/// in [`GlulxSession::section_label`]). Returning an explicit layout (rather than
/// `None`) is what lets the relabelling take effect — see `apply_engine_layout`.
pub const GLULX_TABS: [&[Section]; 3] = [
    &[Section::Disasm],
    &[Section::Globals, Section::Locals, Section::Objects, Section::Dict],
    &[Section::CallStack, Section::EvalStack, Section::Memory],
];

/// A hard cap on the number of Strings rows the list builds per refresh. Preview
/// decoding (E1 Huffman walks) is the cost, and a giant I7 image can hold tens of
/// thousands of string objects (cragne.gblorb: ~55k, ~7µs each) — decoding them
/// all every refresh would drag each turn. 4k rows is plenty to browse and keeps
/// a refresh well under a frame; the list appends a truncation note when it bites.
const MAX_STRING_ROWS: usize = 4_000;

/// Map the disassembler's static confidence [`Tier`] to the panel's neutral
/// [`DisasmProvenance`].
fn tier_prov(t: Tier) -> DisasmProvenance {
    match t {
        Tier::Rd => DisasmProvenance::Rd,
        Tier::Soft => DisasmProvenance::Soft,
        Tier::Data => DisasmProvenance::Data,
    }
}

/// Escape control characters in a string preview so it renders on one line.
fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

/// Render a widened (i64) register value as `0xXXXXXXXX  (signed)`.
fn word(v: i64) -> String {
    format!("0x{:08x}  ({})", v as u32, v as i32)
}

impl Debugger for GlulxSession {
    fn pc(&self) -> u32 {
        self.machine.instr_start_pc()
    }

    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String> {
        self.with_disasm_cache(|c| c.disassemble(self.machine.mem(), self.machine.accel_funcs(), addr, lines))
    }

    fn disassemble_tiered(&self, addr: u32, lines: usize) -> Vec<(String, DisasmProvenance)> {
        self.with_disasm_cache(|c| {
            c.disassemble_tiered(self.machine.mem(), self.machine.accel_funcs(), addr, lines)
                .into_iter()
                .map(|(s, t)| (s, tier_prov(t)))
                .collect()
        })
    }

    // gvm renders a single disassembly format (already a plain mnemonic form with
    // resolved call/branch/glk/string annotations), so Basic and Raw show the same
    // text as Full — the panel's `r` toggle is a no-op for Glulx. The provenance is
    // paired from `disassemble_tiered`, matching these lines one-for-one.
    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String> {
        self.disassemble(addr, lines)
    }

    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String> {
        self.disassemble(addr, lines)
    }

    fn next_instr(&self, addr: u32) -> u32 {
        self.with_disasm_cache(|c| c.next_instr(self.machine.mem(), addr))
    }

    fn prev_instr(&self, addr: u32) -> u32 {
        self.with_disasm_cache(|c| c.prev_instr(self.machine.mem(), addr))
    }

    fn describe_line(&self, addr: u32) -> Option<Vec<String>> {
        Some(self.with_disasm_cache(|c| c.describe(self.machine.mem(), self.machine.accel_funcs(), addr)))
    }

    fn executed_pcs(&self) -> HashSet<u32> {
        self.machine.executed_pcs.clone()
    }

    fn ever_executed_pcs(&self) -> HashSet<u32> {
        self.machine.ever_executed.clone()
    }

    fn stack_lines(&self) -> Vec<String> {
        let frames = self.machine.call_frames(); // innermost first
        if frames.is_empty() {
            return vec!["(no frames)".to_string()];
        }
        // Render outermost first (#0) … innermost last, like the Z-machine panel.
        frames
            .iter()
            .rev()
            .enumerate()
            .map(|(i, f)| {
                format!(
                    "#{i}  ret={:06x}  {} local{}  {} val{}",
                    f.return_pc,
                    f.locals.len(),
                    if f.locals.len() == 1 { "" } else { "s" },
                    f.operands.len(),
                    if f.operands.len() == 1 { "" } else { "s" },
                )
            })
            .collect()
    }

    fn eval_stack_lines(&self) -> Vec<String> {
        // The innermost frame's working value stack, top of stack first.
        match self.machine.call_frames().first() {
            Some(f) if !f.operands.is_empty() => f
                .operands
                .iter()
                .enumerate()
                .rev()
                .map(|(i, &v)| format!("[{i:>3}] {}", word(v)))
                .collect(),
            _ => vec!["(empty)".to_string()],
        }
    }

    fn locals_lines(&self) -> Vec<String> {
        match self.machine.call_frames().first() {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(none)".to_string()],
            Some(f) => f.locals.iter().enumerate().map(|(i, &v)| format!("local{i} = {}", word(v))).collect(),
        }
    }

    fn globals_lines(&self) -> Vec<String> {
        // The "Glk" section: the live Glk window-tree snapshot (read-only), reused
        // from the same cache `/dump-windows` prints.
        if self.window_dump_cache.is_empty() {
            vec!["(no Glk windows)".to_string()]
        } else {
            self.window_dump_cache.clone()
        }
    }

    fn object_tree_lines(&self) -> Vec<String> {
        // The "Functions" section: every discovered function. Each row leads with
        // its entry address as a clickable `@0x……` Memory-jump token (the panel's
        // Objects-slot click convention), then C0/C1, locals count, confidence
        // tier, and an accel badge when the VM has accelerated it.
        self.with_disasm_cache(|c| {
            c.functions(self.machine.accel_funcs())
                .iter()
                .map(|f| {
                    let kind = if f.type_byte == 0xC0 { "C0" } else { "C1" };
                    let tier = match f.tier {
                        Tier::Rd => "Rd",
                        Tier::Soft => "Soft",
                        Tier::Data => "Data",
                    };
                    let mut s = format!(
                        "@0x{:06x}  {kind}  {} local{}  [{tier}]",
                        f.addr,
                        f.locals_count,
                        if f.locals_count == 1 { "" } else { "s" },
                    );
                    if let Some(num) = f.accel {
                        s.push_str(&format!("  [accel: {}]", gvm::accel::accel_name(num)));
                    }
                    s
                })
                .collect()
        })
    }

    fn dictionary_lines(&self) -> Vec<String> {
        // The "Strings" section: discovered string objects with a short preview.
        // Each row leads with its address as a clickable `@0x……` Memory-jump token.
        self.with_disasm_cache(|c| {
            let total = c.string_count();
            let mut out: Vec<String> = c
                .strings(self.machine.mem(), MAX_STRING_ROWS)
                .iter()
                .map(|s| {
                    let kind = match s.type_byte {
                        0xE0 => "str",
                        0xE1 => "str(E1)",
                        _ => "str(uni)",
                    };
                    format!("@0x{:06x}  {kind}  \"{}\"", s.addr, escape(&s.preview))
                })
                .collect();
            if total > MAX_STRING_ROWS {
                out.push(format!("… {} more strings (list truncated)", total - MAX_STRING_ROWS));
            }
            out
        })
    }

    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String> {
        let mem = self.machine.mem();
        let len = mem.mem_size();
        let ramstart = mem.ramstart();
        let mut out = Vec::with_capacity(rows);
        let mut a = addr.min(len);
        for _ in 0..rows {
            if a >= len {
                break;
            }
            let end = (a + 16).min(len);
            let mut hex = String::new();
            let mut ascii = String::new();
            for i in a..end {
                let b = mem.read8(i).unwrap_or(0) as u8;
                hex.push_str(&format!("{b:02x} "));
                ascii.push(if (0x20..=0x7e).contains(&b) { b as char } else { '.' });
            }
            // RAMSTART-relative annotation: mark rows in writable RAM (the ROM/RAM
            // boundary is the single most useful landmark in a Glulx memory map).
            let tag = if a >= ramstart { "  <RAM>" } else { "" };
            out.push(format!("{a:06x}  {hex:<48}{ascii}{tag}"));
            a = end;
        }
        out
    }

    fn memory_len(&self) -> u32 {
        self.machine.mem().mem_size()
    }

    fn object_detail(&self, _obj: u16) -> Vec<String> {
        // Functions aren't expandable (they carry no `[N]` id the tree parser keys
        // on), so this is never invoked for the Glulx Functions list.
        Vec::new()
    }

    fn frame_locals(&self, idx: usize) -> Vec<String> {
        // `idx` is the display index (outermost = #0); map back to the innermost-
        // first `call_frames` order.
        let frames = self.machine.call_frames();
        let n = frames.len();
        match (idx < n).then(|| &frames[n - 1 - idx]) {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(no locals)".to_string()],
            Some(f) => f.locals.iter().enumerate().map(|(i, &v)| format!("local{i} = {}", word(v))).collect(),
        }
    }

    fn var_value(&self, _var: u8) -> Option<u16> {
        // Glulx has no compact Z-style variable-number space; the Memory jump box
        // only dereferences literal hex addresses here.
        None
    }

    fn sections(&self) -> Option<[&'static [Section]; 3]> {
        Some(GLULX_TABS)
    }

    fn section_label(&self, s: Section) -> &'static str {
        match s {
            Section::Globals => "Glk",
            Section::Objects => "Functions",
            Section::Dict => "Strings",
            other => other.label(),
        }
    }
}

impl GlulxSession {
    /// Lazily build (on first use) and borrow the debug disassembly cache, then
    /// run `f` over it. Discovery is not cheap on a multi-MB I7 image, so this is
    /// the ONLY place the cache is built — and only ever from an inspector read.
    ///
    /// Coverage is kept in step with execution: the full cumulative `ever_executed`
    /// set is folded in at build time (so boot + prior-run PCs decode as real
    /// code), and each subsequent read folds the small per-turn `executed_pcs` set
    /// so code first reached after the inspector opened still renders.
    pub(crate) fn with_disasm_cache<R>(&self, f: impl FnOnce(&gvm::disasm::DisasmCache) -> R) -> R {
        {
            let mut slot = self.disasm_cache.borrow_mut();
            if slot.is_none() {
                let mut cache = gvm::disasm::DisasmCache::build(self.machine.mem());
                cache.seed_executed(self.machine.ever_executed.iter().copied());
                *slot = Some(cache);
            } else {
                let pcs: Vec<u32> = self.machine.executed_pcs.iter().copied().collect();
                if !pcs.is_empty() {
                    slot.as_mut().expect("checked Some above").seed_executed(pcs);
                }
            }
        }
        let slot = self.disasm_cache.borrow();
        f(slot.as_ref().expect("built above"))
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use crate::debug_panel::{DebugPanelState, Section};
    use crate::engine::Engine;
    use crate::glulx_session::GlulxSession;

    /// The Glulxercise test image, or `None` (test skips cleanly) when absent.
    fn fixture_session() -> Option<GlulxSession> {
        let path = "../gvm-cli/tests/fixtures/glulxercise.ulx";
        let bytes = std::fs::read(path).ok()?;
        GlulxSession::new(bytes, 80, 24, true, false, false, (1, 1), None, &[]).ok()
    }

    #[test]
    fn sections_relabel_functions_strings_and_glk() {
        let Some(sess) = fixture_session() else {
            eprintln!("skip: glulxercise.ulx not found");
            return;
        };
        let dbg = sess.debugger().expect("Glulx exposes a debugger");
        // Every register-machine section survives (unlike Scott); three slots are
        // repurposed and relabelled.
        let layout = dbg.sections().expect("Glulx provides a layout");
        let all: Vec<Section> = layout.iter().flat_map(|w| w.iter().copied()).collect();
        for s in [Section::Disasm, Section::CallStack, Section::EvalStack, Section::Locals, Section::Memory] {
            assert!(all.contains(&s), "{s:?} kept");
        }
        assert_eq!(dbg.section_label(Section::Objects), "Functions");
        assert_eq!(dbg.section_label(Section::Dict), "Strings");
        assert_eq!(dbg.section_label(Section::Globals), "Glk");
        assert_eq!(dbg.section_label(Section::Memory), "Memory");
    }

    #[test]
    fn discovery_populates_functions_strings_and_disasm_at_pc() {
        let Some(sess) = fixture_session() else {
            eprintln!("skip: glulxercise.ulx not found");
            return;
        };
        let dbg = sess.debugger().expect("debugger");
        assert!(!dbg.object_tree_lines().is_empty(), "Functions list non-empty");
        assert!(
            dbg.object_tree_lines().iter().all(|l| l.starts_with("@0x")),
            "each Functions row leads with a clickable @0x token"
        );
        assert!(!dbg.dictionary_lines().is_empty(), "Strings list non-empty");
        // Disassembly at the parked PC renders whole instruction rows (6-hex addr).
        let disasm = dbg.disassemble(dbg.pc(), 16);
        assert!(!disasm.is_empty(), "disasm at pc renders");
        assert!(
            disasm[0].len() >= 6 && u32::from_str_radix(&disasm[0][..6], 16).is_ok(),
            "row leads with a 6-hex address: {:?}",
            disasm[0]
        );
        // Memory view + length are sane.
        assert!(dbg.memory_len() > 0);
        assert!(!dbg.memory_hex(0, 4).is_empty(), "memory hex renders");
    }

    #[test]
    fn tracing_grows_executed_sets_over_a_turn() {
        let Some(mut sess) = fixture_session() else {
            eprintln!("skip: glulxercise.ulx not found");
            return;
        };
        sess.set_debug_trace(true);
        let _ = sess.submit("1"); // any input drives real code
        let dbg = sess.debugger().expect("debugger");
        assert!(!dbg.executed_pcs().is_empty(), "code executed this turn");
        assert!(!dbg.ever_executed_pcs().is_empty(), "cumulative coverage recorded");
    }

    #[test]
    fn panel_refresh_end_to_end_adopts_glulx_layout() {
        let Some(mut sess) = fixture_session() else {
            eprintln!("skip: glulxercise.ulx not found");
            return;
        };
        sess.set_debug_trace(true);
        let mut panel = {
            let dbg = sess.debugger().expect("debugger");
            let mut p = DebugPanelState::new(dbg.pc());
            p.apply_engine_layout(dbg);
            p.refresh(dbg);
            p
        };
        assert_eq!(panel.tab_label(Section::Objects), "Functions");
        assert_eq!(panel.tab_label(Section::Dict), "Strings");
        assert_eq!(panel.tab_label(Section::Globals), "Glk");
        assert!(!panel.snapshot.disasm.is_empty(), "disasm populated");
        assert!(!panel.snapshot.objects.is_empty(), "Functions populated");
        assert!(!panel.snapshot.dict.is_empty(), "Strings populated");
        // Play a turn and re-refresh; coverage accrues.
        sess.submit("2");
        {
            let dbg = sess.debugger().expect("debugger");
            panel.refresh(dbg);
        }
        assert!(!panel.snapshot.executed_ever.is_empty(), "cumulative coverage after a turn");
    }
}

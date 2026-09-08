//! Structured crash stack trace (zero-dep value type + text formatter).

/// A single call frame captured at a fault. Innermost (faulting) frame first
/// in `StackTrace::frames`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TraceFrame {
    /// Routine entry address (0 = unknown; gvm always reports 0).
    pub func_addr: u32,
    /// PC to resume in the caller.
    pub return_pc: u32,
    /// Local variables, widened to i64.
    pub locals: Vec<i64>,
    /// This frame's working eval-stack values, widened to i64.
    pub operands: Vec<i64>,
}

/// A crash stack trace: the fault site plus the live call stack.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StackTrace {
    /// Human-readable fault, e.g. "memory fault: read16 @0x004a1c".
    pub fault: String,
    /// Start-PC of the faulting instruction.
    pub fault_pc: u32,
    /// Decoded mnemonic of the faulting instruction, e.g. "loadw".
    pub fault_op: String,
    /// Hex render width in bytes: 2 = u16 (zvm), 4 = u32 (gvm).
    pub width: u8,
    /// Call frames, innermost (faulting) first.
    pub frames: Vec<TraceFrame>,
}

impl StackTrace {
    /// Canonical multi-line text form, shared by every host surface. One string
    /// per line, no trailing newlines.
    pub fn to_lines(&self) -> Vec<String> {
        let digits = (self.width as usize) * 2;
        let hexw = |v: i64| format!("0x{:0width$x}", v as u64 & mask(self.width), width = digits);
        let list = |xs: &[i64]| {
            xs.iter().map(|&v| hexw(v)).collect::<Vec<_>>().join(",")
        };
        let mut out = vec![
            "*** VM FAULT ***".to_string(),
            self.fault.clone(),
            format!("PC=0x{:06x}  op={}", self.fault_pc, self.fault_op),
        ];
        for (i, f) in self.frames.iter().enumerate() {
            let mut line = format!(
                "  #{i}  fn@0x{:06x}  ret=0x{:06x}  locals=[{}]",
                f.func_addr,
                f.return_pc,
                list(&f.locals),
            );
            if !f.operands.is_empty() {
                line.push_str(&format!("  stack=[{}]", list(&f.operands)));
            }
            out.push(line);
        }
        out
    }
}

fn mask(width: u8) -> u64 {
    match width {
        2 => 0xFFFF,
        4 => 0xFFFF_FFFF,
        _ => u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_lines_renders_u32_width() {
        let t = StackTrace {
            fault: "memory fault: read32 @0x00040000".to_string(),
            fault_pc: 0x1abc,
            fault_op: "aload".to_string(),
            width: 4,
            frames: vec![TraceFrame { func_addr: 0, return_pc: 0x00001234, locals: vec![0xdead_beef], operands: vec![] }],
        };
        let lines = t.to_lines();
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert_eq!(lines[2], "PC=0x001abc  op=aload");
        // width=4 → 8 hex digits; func_addr 0 renders as fn@0x000000
        assert_eq!(lines[3], "  #0  fn@0x000000  ret=0x001234  locals=[0xdeadbeef]");
    }
}

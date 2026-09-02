//! One rule, checked mechanically, about the one thing a process id is not: a
//! discriminator between two tests.
//!
//! `std::process::id()` is unique per PROCESS, and a scratch path built from it alone
//! is therefore unique per process too. Under `cargo nextest run` that is the same
//! thing as unique per test, because nextest gives every test its own process. Under
//! `cargo test` — which is what GitHub Actions runs — one binary's tests share a
//! process and run on parallel threads, so every caller of a pid-keyed helper gets the
//! SAME directory. `fs::write` truncates and a case's closing `remove_dir_all` deletes
//! whatever a neighbour is halfway through reading, so the failure is not a crash: a
//! correct fixture fails its own assertion, somewhere else, intermittently.
//!
//! **This class of race is structurally invisible to the local gate.** Not merely
//! unlikely there — impossible, because nextest's process-per-test model means no test
//! can observe another's scratch space. It is the palette race of SQ-0904 in a new
//! costume, and it cost the same: CI was red for eight consecutive commits while
//! `cargo nextest run --workspace` printed 0 every single time. The cause was
//! `verb-synonyms-gen`'s `scratch()`, which keyed one directory on the pid and handed
//! it to every caller of `wordnet_fixture()`; one thread read `index.verb` while
//! another rewrote it, `WordNet::load` came back empty, `build` returned no groups.
//!
//! # The three cases
//!
//! | case | shape it fails | why that shape is wrong |
//! |---|---|---|
//! | [`a_shared_helper_counts_its_calls`] | a scratch path in a fn that is not a test, whose name carries no per-call counter | a helper is called once per CALLER, and neither the pid nor a `tag` moves between calls |
//! | [`no_two_tests_name_the_same_scratch_path`] | one bare-pid name built at two sites | two tests in one binary, one directory, and each deletes it when it ends |
//! | [`the_scan_reaches_unit_tests_in_src`] | this file quietly scanning only `tests/` | the `src/` `#[cfg(test)]` modules are where most scratch directories live |
//!
//! The remedy for the first two, inside `app`, is [`app::scratch_dir`], which is unique
//! per call by construction. Crates that cannot depend on `app` — `zvm`, `gvm` and
//! `scott` take zero external dependencies, and the CLI crates sit below it — spell an
//! `AtomicUsize` beside the pid locally. **The rule is the counter, not the function.**
//!
//! # A `tag` is not a counter (SQ-1163)
//!
//! The first case used to fail only the *bare* pid, and said so: a helper interpolating
//! a `tag` its callers pass satisfied it, and a tag two callers spell the same way
//! collides exactly as a bare pid does. That is the exemplar's own shape, left standing
//! as a hand-maintained invariant — and a measurement found fifty-one helpers relying on
//! it, two of them (`hints::tests::scratch` and `hints::tests::scratch_dir`) literally
//! `bm-{tag}-{pid}`. So the case now asks for the counter itself: a scratch path built
//! outside a `#[test]` must have an `AtomicUsize` in the same function, which is what
//! makes the directory unique per CALL rather than per spelling.
//!
//! A site INSIDE a `#[test]` needs no counter — a case body runs once per process — and
//! is left to the second case, which is what stops two of them naming one directory.
//!
//! Production code is out of scope on purpose. `fetch_worker::maybe_fetch_cover` and
//! `hint_download::finalize_download` build `.part-<pid>` siblings for atomic writes,
//! and `crates/app/examples/` runs one boot per process by hand; none of them has a
//! second thread in the same process racing for the same name.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `.rs` file under `crates/`, as (path relative to the workspace, source).
///
/// `target/` is skipped: it holds generated code that is nobody's to fix.
fn rust_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !name.starts_with("target") {
                    stack.push(path);
                }
            } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    out.push((relative(&root, &path), src));
                }
            }
        }
    }
    out.sort();
    out
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// The source with every comment blanked to spaces, newlines kept.
///
/// Two reasons, and the second is this file. Comments hold `fn` and `#[test]` and
/// braces that would derail the scans below — and this file's own prose has to be able
/// to describe the forbidden spelling without becoming an instance of it, the way
/// `palette_lock_discipline` cannot and warns about instead.
///
/// String literals are tracked on the way through, raw strings included, so a `//`
/// inside one is left alone.
fn without_comments(src: &str) -> Vec<char> {
    let c: Vec<char> = src.chars().collect();
    let mut out = c.clone();
    let mut i = 0usize;
    while i < c.len() {
        // A raw string: r, then any number of #, then the quote.
        if c[i] == 'r' && !prev_is_ident(&c, i) {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < c.len() && c[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < c.len() && c[j] == '"' {
                i = skip_raw_string(&c, j + 1, hashes);
                continue;
            }
        }
        match c[i] {
            '"' => i = skip_string(&c, i + 1),
            '\'' => {
                // A char literal, or a lifetime — only the literal has a closing quote
                // within three characters, and either way stepping one is safe.
                if i + 2 < c.len() && (c[i + 1] == '\\' || c[i + 2] == '\'') {
                    i = skip_char_literal(&c, i + 1);
                } else {
                    i += 1;
                }
            }
            '/' if i + 1 < c.len() && c[i + 1] == '/' => {
                while i < c.len() && c[i] != '\n' {
                    out[i] = ' ';
                    i += 1;
                }
            }
            '/' if i + 1 < c.len() && c[i + 1] == '*' => {
                let mut depth = 1usize;
                out[i] = ' ';
                out[i + 1] = ' ';
                let mut j = i + 2;
                while j < c.len() && depth > 0 {
                    if c[j] == '/' && j + 1 < c.len() && c[j + 1] == '*' {
                        depth += 1;
                    } else if c[j] == '*' && j + 1 < c.len() && c[j + 1] == '/' {
                        depth -= 1;
                    }
                    if c[j] != '\n' {
                        out[j] = ' ';
                    }
                    j += 1;
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

fn prev_is_ident(c: &[char], i: usize) -> bool {
    i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_')
}

/// Index just past the closing quote of a normal string starting at `from`.
fn skip_string(c: &[char], from: usize) -> usize {
    let mut i = from;
    while i < c.len() {
        match c[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    c.len()
}

fn skip_char_literal(c: &[char], from: usize) -> usize {
    let mut i = from;
    while i < c.len() {
        match c[i] {
            '\\' => i += 2,
            '\'' => return i + 1,
            _ => i += 1,
        }
    }
    c.len()
}

/// Index just past the `"` + `hashes` that closes a raw string body starting at `from`.
fn skip_raw_string(c: &[char], from: usize, hashes: usize) -> usize {
    let mut i = from;
    while i < c.len() {
        if c[i] == '"' && c[i + 1..].iter().take(hashes).filter(|h| **h == '#').count() == hashes {
            return i + 1 + hashes;
        }
        i += 1;
    }
    c.len()
}

fn at(c: &[char], i: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(k, ch)| c.get(i + k) == Some(&ch))
}

fn text(c: &[char], a: usize, b: usize) -> String {
    c[a.min(c.len())..b.min(c.len())].iter().collect()
}

/// One `format!` that builds a filesystem path out of the process id.
struct Site {
    file: String,
    line: usize,
    /// The format string's literal text, exactly as written.
    literal: String,
    /// Holes in the literal other than the one the pid fills.
    other_holes: usize,
    /// True when the innermost enclosing chain of `fn`s reaches a `#[test]`, i.e. this
    /// runs at most once per process.
    inside_a_test: bool,
    /// True when the innermost enclosing `fn` also holds an `AtomicUsize` bump, i.e.
    /// the name it builds is unique per CALL and not merely per spelling.
    counted: bool,
}

/// The literal with every `{…}` normalised to `{}`, which is what two sites have to
/// share for their paths to be the same string at runtime.
fn skeleton(literal: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in literal.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth == 1 {
                    out.push_str("{}");
                }
            }
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn holes(literal: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for ch in literal.chars() {
        match ch {
            '{' => cur = Some(String::new()),
            '}' => {
                if let Some(h) = cur.take() {
                    out.push(h);
                }
            }
            _ => {
                if let Some(h) = cur.as_mut() {
                    h.push(ch);
                }
            }
        }
    }
    out
}

/// Is the site in code that only ever runs under a test runner?
///
/// Everything under a `tests/` directory is, and so is anything inside a
/// `#[cfg(test)]` module — which is where the bulk of `app`'s cases live, all of them
/// in the one library test binary and therefore all in one process under `cargo test`.
fn in_test_code(file: &str, line: usize, cfg_test_spans: &[(usize, usize)]) -> bool {
    file.contains("/tests/") || cfg_test_spans.iter().any(|(a, b)| *a <= line && line <= *b)
}

/// Line spans of every `#[cfg(test)]` module, by brace depth from its opening brace.
fn cfg_test_spans(c: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < c.len() {
        if c[i] == '\n' {
            line += 1;
        }
        if at(c, i, "#[cfg(test)]") {
            let start_line = line;
            // Walk to the module's opening brace, then to its match.
            let mut j = i;
            let mut l = line;
            while j < c.len() && c[j] != '{' {
                if c[j] == '\n' {
                    l += 1;
                }
                j += 1;
            }
            let mut depth = 0usize;
            while j < c.len() {
                match c[j] {
                    '\n' => l += 1,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    '"' => {
                        j = skip_string(c, j + 1);
                        continue;
                    }
                    _ => {}
                }
                j += 1;
            }
            spans.push((start_line, l));
            i = j;
            line = l;
        }
        i += 1;
    }
    spans
}

/// Does the chain of `fn`s enclosing `line` reach one marked `#[test]`?
///
/// By indentation, which is exact on `cargo fmt`-formatted source and is what tells a
/// helper nested INSIDE a case (`fn chunk(..)` in `main.rs`'s blorb tests — called four
/// times, but only ever from the one case that defines it) apart from a free-standing
/// helper every case in the file calls.
fn inside_a_test(lines: &[&str], line: usize) -> bool {
    let mut idx = line.saturating_sub(1);
    let mut indent = usize::MAX;
    loop {
        let l = lines[idx];
        if let Some(col) = fn_indent(l) {
            if col < indent {
                indent = col;
                let marked = (idx.saturating_sub(8)..idx)
                    .any(|j| lines[j].trim_start().starts_with("#[test]") || lines[j].trim_start().starts_with("#[tokio::test]"));
                if marked {
                    return true;
                }
                if col == 0 {
                    return false;
                }
            }
        }
        if idx == 0 {
            return false;
        }
        idx -= 1;
    }
}

/// Does the innermost `fn` enclosing `line` bump an `AtomicUsize`?
///
/// That bump is the whole remedy — it is what `app::scratch_dir` does internally, and
/// what a crate that cannot depend on `app` spells for itself. Asking for it in the
/// SAME function as the `format!` is deliberate: a counter three frames up is a fact
/// the reader of this helper cannot see, which is the arrangement being replaced.
///
/// The body runs from the `fn` line to the first line that is a lone `}` at the `fn`'s
/// own indent, which is exact on `cargo fmt`-formatted source.
fn counted(lines: &[&str], line: usize) -> bool {
    let site = lines[line - 1];
    let site_indent = site.len() - site.trim_start().len();
    let mut idx = line.saturating_sub(1);
    let start = loop {
        match fn_indent(lines[idx]) {
            Some(col) if col < site_indent => break idx,
            _ if idx == 0 => return false,
            _ => idx -= 1,
        }
    };
    let col = fn_indent(lines[start]).unwrap_or(0);
    let close = format!("{}}}", " ".repeat(col));
    let end = (start + 1..lines.len()).find(|i| lines[*i] == close).unwrap_or(lines.len() - 1);
    lines[start..=end].iter().any(|l| l.contains("fetch_add("))
}

/// The column of a `fn` item declared on this line, if it declares one.
fn fn_indent(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let mut rest = line.trim_start();
    for prefix in ["pub(crate) ", "pub(super) ", "pub ", "const ", "async ", "unsafe ", "extern "] {
        while let Some(r) = rest.strip_prefix(prefix) {
            rest = r;
        }
    }
    rest.starts_with("fn ").then_some(indent)
}

/// Every `format!` in the workspace's test code that interpolates the process id into a
/// filesystem path.
///
/// "Into a filesystem path" is the load-bearing half: a pid in a log line, a trace tag
/// or a lock name is not this bug. The test is the call the `format!` is an argument
/// to — `.join(`, `.push(`, `.with_file_name(` — or a `temp_dir()` a couple of lines
/// above it, which is how every one of these is spelled here.
fn scratch_sites() -> Vec<Site> {
    const PID: &str = "process::id()";
    let mut out = Vec::new();
    for (file, src) in rust_sources() {
        if !src.contains(PID) {
            continue;
        }
        let c = without_comments(&src);
        let lines: Vec<&str> = src.lines().collect();
        let spans = cfg_test_spans(&c);
        let mut i = 0usize;
        let mut line = 1usize;
        while i < c.len() {
            if c[i] == '\n' {
                line += 1;
            }
            if !at(&c, i, "format!(") {
                i += 1;
                continue;
            }
            // The literal, then the rest of the call's arguments.
            let mut j = i + "format!(".len();
            while j < c.len() && c[j].is_whitespace() {
                j += 1;
            }
            if c.get(j) != Some(&'"') {
                i += 1;
                continue;
            }
            let lit_end = skip_string(&c, j + 1);
            let literal = text(&c, j + 1, lit_end - 1);
            let mut k = lit_end;
            let mut depth = 1usize;
            while k < c.len() && depth > 0 {
                match c[k] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    '"' => {
                        k = skip_string(&c, k + 1);
                        continue;
                    }
                    _ => {}
                }
                k += 1;
            }
            let args = text(&c, lit_end, k);
            if !args.contains(PID) && !literal.contains(PID) {
                i += 1;
                continue;
            }
            let before = text(&c, i.saturating_sub(200), i);
            let tail = before.trim_end();
            let path_forming = tail.ends_with(".join(")
                || tail.ends_with(".push(")
                || tail.ends_with(".with_file_name(")
                || before.contains("temp_dir()");
            if path_forming && in_test_code(&file, line, &spans) {
                let hs = holes(&literal);
                let pid_holes = hs.iter().filter(|h| h.contains("process::id")).count().max(1);
                out.push(Site {
                    file: file.clone(),
                    line,
                    literal: literal.clone(),
                    other_holes: hs.len().saturating_sub(pid_holes),
                    inside_a_test: inside_a_test(&lines, line),
                    counted: counted(&lines, line),
                });
            }
            i += 1;
        }
    }
    out
}

/// A helper is called once per CALLER, and neither the pid nor a `tag` moves between
/// calls.
///
/// This is the exemplar's shape, and the one a new harness reaches for: a free-standing
/// `fn boot()` or `fn scratch(tag)` that every case in the suite calls, building one
/// directory from `temp_dir()` and the pid. Under nextest each case is its own process
/// and owns it; under `cargo test` all of them share it, and each `remove_dir_all`s it
/// on the way out.
///
/// **A `tag` parameter is not a fix** (SQ-1163). It looks like one — every caller passes
/// a different string, so every caller gets a different directory — but that is an
/// invariant maintained by hand across call sites in different files, and the moment two
/// spell one the same way it is the bare form again with extra steps. What the case asks
/// for is the counter: an `AtomicUsize` in the same function, which is what
/// [`app::scratch_dir`] appends and what makes the name unique per CALL.
///
/// Falsified by dropping the counter out of any helper it names — done for SQ-1131 with
/// `v6_macintosh_profile::launch`, whose eighteen call sites all resolved to
/// `lanthorn-mac-profile-<pid>`, and for SQ-1163 with `hints::tests::scratch`, whose
/// `bm-{tag}-<pid>` this case could not see at all until it stopped asking about holes
/// and started asking about counters.
#[test]
fn a_shared_helper_counts_its_calls() {
    let sites = scratch_sites();
    let bad: Vec<String> = sites
        .iter()
        .filter(|s| !s.inside_a_test && !s.counted)
        .map(|s| format!("{}:{} — \"{}\"", s.file, s.line, s.literal))
        .collect();
    assert!(
        bad.is_empty(),
        "these scratch paths are built inside a helper with no per-call counter:\n\
         \x20   {}\n\
         A helper runs once per CALLER, and neither the pid nor a tag changes between \
         calls: under cargo test — which is what CI runs — one binary's tests share a \
         process, so two callers reaching the same name get the same directory, truncate \
         each other's fixtures, and delete them mid-read (SQ-1131, SQ-1163). cargo nextest \
         run cannot see this, because it gives every test its own process. Inside app, take\n\
         \x20   let dir = app::scratch_dir(\"a-tag\");   // unique per call, created empty\n\
         and elsewhere put an AtomicUsize beside the pid, in the same function.",
        bad.join("\n\x20   ")
    );
    assert!(
        sites.len() >= 100,
        "only {} scratch sites found — this case is looking in the wrong place and would pass \
         vacuously",
        sites.len()
    );
}

/// The scan reads `src/` too, and most of what it finds is there.
///
/// SQ-1163 was filed on the belief that this file only read
/// `crates/app/tests/suites/` — it never did, [`rust_sources`] has always walked the
/// whole of `crates/` and [`in_test_code`] has always counted a `#[cfg(test)]` module
/// in a source file. But the belief was reasonable, because nothing here SAID so and
/// nothing would have failed if a later edit narrowed the walk: the vacuity floor above
/// is met by `tests/` alone.
///
/// So the claim is now a case. `app`'s unit tests outnumber its integration suites'
/// scratch paths several times over, and `crates/app/src/config.rs` alone holds more
/// than every file under `tests/suites/` put together.
#[test]
fn the_scan_reaches_unit_tests_in_src() {
    let sites = scratch_sites();
    let in_src = sites.iter().filter(|s| s.file.contains("/src/")).count();
    assert!(
        in_src >= 50,
        "only {in_src} of {} scratch sites are under a src/ directory — the walk has been \
         narrowed, and every #[cfg(test)] module in the workspace is now unguarded",
        sites.len()
    );
    assert!(
        sites.iter().any(|s| s.file == "crates/app/src/config.rs"),
        "config.rs's #[cfg(test)] module is the densest scratch-path site in the workspace \
         and must be among those scanned"
    );
}

/// Two tests must not name the same directory.
///
/// Where the first case is about one site reached many times, this is about many sites
/// that resolve to one string. MEASURED for SQ-1131: `persist_files::tests::
/// save_then_load_round_trips` and `turn::tests::apply_turn_events_halts_and_logs_on_fault`
/// both built `lanthorn-test-<pid>`, both wrote into it, and both ended with
/// `remove_dir_all` — in the same library test binary, which under `cargo test` is one
/// process running them on parallel threads.
///
/// Only paths whose sole hole is the pid are compared, because those are the only ones
/// whose runtime string is known here; two sites sharing the skeleton
/// `bm-{}-<pid>` are told apart by the tag their callers pass.
#[test]
fn no_two_tests_name_the_same_scratch_path() {
    let sites = scratch_sites();
    let mut bare: Vec<(String, &Site)> = sites
        .iter()
        .filter(|s| s.other_holes == 0)
        .map(|s| (skeleton(&s.literal), s))
        .collect();
    bare.sort_by(|a, b| a.0.cmp(&b.0));
    let mut clashes = Vec::new();
    for pair in bare.windows(2) {
        if pair[0].0 == pair[1].0 {
            clashes.push(format!(
                "\"{}\" at {}:{} and {}:{}",
                pair[0].0, pair[0].1.file, pair[0].1.line, pair[1].1.file, pair[1].1.line
            ));
        }
    }
    assert!(
        clashes.is_empty(),
        "these scratch paths are the same string in two places:\n\
         \x20   {}\n\
         The process id is the only thing varying in each, and under cargo test two tests in \
         one binary share it — so one deletes the other's directory while it is still being \
         read (SQ-1131). Give each its own name, or take app::scratch_dir(\"a-tag\"), which is \
         unique per call by construction.",
        clashes.join("\n\x20   ")
    );
}

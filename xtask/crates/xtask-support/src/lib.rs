//! `xtask-support` — shared foundation for the decomposed xtask commands (v-xtask-decompose). First
//! slice: the content-addressing / build-cache-fingerprint helpers, carved out of the xtask monolith so
//! per-command crates reuse them without duplication. (The corpus/Tools/convert machinery follows here.)

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The platform CONTENT ADDRESS of `bytes`: `Hash::of(Blob, bytes)` rendered as the canonical string —
/// byte-identical to `cdz-run`'s `content_address` and the store's `put()` key, so the store address ==
/// blob key == compose-dep `+hash` == `REQUIRED_RUNTIME_HASH` are one string across the fleet (design §8).
pub fn content_address(bytes: &[u8]) -> String {
    cdz_contract::Hash::of(cdz_contract::HashTag::Blob, bytes).to_string()
}

/// A deterministic SHA-256 fingerprint of a whole directory TREE (sorted path + content), used as an
/// internal build-cache key (NOT the content-address digest above — this is a private cache fingerprint,
/// no cross-boundary contract). `None` if the tree can't be enumerated.
pub fn hash_tree(root: &Path) -> Option<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(root, &mut files).ok()?;
    files.sort();
    let mut h = Sha256::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0u8]); // path/content separator
        if let Ok(bytes) = std::fs::read(f) {
            h.update(&bytes);
        }
        h.update([0u8]); // file separator
    }
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    Some(s)
}

/// Recursively collect every regular file under `dir` into `out` (used by `hash_tree`).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_files(&path, out)?;
        } else if ty.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

// ── Corpus record model (v-xtask-decompose slice 2a) — the parsed shape of the `cdz-syntax corpus`
// stream, SHARED by the gate/roundtrip/emit commands. Moved here so the per-command crates (xtask-gate,
// xtask-roundtrip, …) reuse the ONE parser + model instead of duplicating it (drift-sensitive). All
// std-only; fields are `pub` so a consumer crate can construct/read them.

/// A parsed corpus record (the flat stream `cdz-syntax corpus` emits).
pub struct CorpusRecord {
    pub description: String,
    pub program: String,
    /// Sibling LIBRARY modules of a multi-file PACKAGE case, each a `(name, program)` from a `module`
    /// record line. Empty for a single-file case.
    pub modules: Vec<(String, String)>,
    /// PEER components of a CROSS-COMPONENT case — each an `(interface, provider-program)` from a `peer`
    /// record line. Empty for a single-component case.
    pub peers: Vec<(String, String)>,
    /// One or more TRIALS — each an optional `(call …)` paired with the `expect` payload it must produce.
    pub trials: Vec<Trial>,
    /// The `(needs …)` capabilities a case documents (documentation only now — grading is by what the
    /// compiler actually does).
    #[allow(dead_code)]
    pub needs: Vec<String>,
    /// The HOST-CALL RESPONSES (E2h) — `(op, value)` pairs from the stream's `host-response` lines.
    pub host_responses: Vec<(String, String)>,
    /// The recorded HOST-CALL sequence (E2h) — the dotted `E.op` names from the stream's `host-call` lines.
    pub host_calls: Vec<String>,
    /// The WARNING pins — `(code, optional message-substring)` from the case's `(warns …)` clauses.
    pub warns: Vec<(String, Option<String>)>,
    /// An explicit WIT WORLD the case imposes (from the stream's `wit-world` line). `None` for synthesized.
    pub wit_world: Option<String>,
    /// The interface a `(wit-world …)` case's guest exports under (stream `component-name` line).
    pub component_name: Option<String>,
    /// The live-heap-cell count a `(live-objects N)` clause asserts after the run. `None` if absent.
    pub live_objects: Option<u32>,
}

/// One (call, expected-payload) trial of a case — a single run of the compiled program.
pub struct Trial {
    /// The `(call …)` for this trial, or `None` to invoke the sole export with no arguments.
    pub call: Option<Call>,
    /// The `expect` payload, e.g. `output (: 42 Int64)`, `error CDZ0201`, `trap "…"`.
    pub expect: String,
}

/// A corpus case's `(call <export> <arg>…)` clause, parsed from the record stream.
pub struct Call {
    pub export: String,
    pub args: Vec<String>,
    /// A `(then <arg>…)` continuation (two-call-on-one-handle): the SECOND call's arguments, or `None`.
    pub second_call: Option<Vec<String>>,
    /// A `(drop)` clause: resource-drop the minted closure handle after the call(s) before reading.
    pub drop_handle: bool,
    /// A `(call-method <member> …)` clause: the NAMED value-resource member to invoke. `None` otherwise.
    pub method: Option<String>,
}

// moved from xtask/src/main.rs (v-xtask-decompose slice 2b) — &Tools/&Paths → &Path.
pub fn default_corpus_files(repo: &Path) -> Vec<PathBuf> {
    let dir = repo.join("spec/semantics");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("xtask gate: reading {}: {e}", dir.display());
            std::process::exit(1);
        })
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sexp"))
        // Only the `NN-feature` corpus files (a numeric prefix).
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()))
        })
        .collect();
    files.sort();
    files
}
// moved from xtask/src/main.rs (v-xtask-decompose slice 2b) — &Tools/&Paths → &Path.
/// Run `cdz-corpus records <file>` and parse its record stream.
pub fn read_corpus(corpus_bin: &Path, file: &Path) -> Vec<CorpusRecord> {
    use std::process::Command;
    let out = Command::new(corpus_bin)
        .arg("records")
        .arg(file)
        .output()
        .unwrap_or_else(|e| launch_fail("cdz-corpus records", e));
    if !out.status.success() {
        eprintln!(
            "xtask gate: reading {}: {}",
            file.display(),
            first_line(&out.stderr)
        );
        std::process::exit(1);
    }
    parse_records(&String::from_utf8_lossy(&out.stdout))
}
// moved from xtask/src/main.rs (v-xtask-decompose slice 2b) — &Tools/&Paths → &Path.
/// Parse the flat record stream: `key\tvalue` lines, records separated by a `---` line. Each TRIAL is a
/// `call` line (the export) + its following `arg` lines + the `expect` line that CLOSES it — so an
/// `expect` flushes the pending call/args into a trial. A single-trial case is the historical shape.
pub fn parse_records(text: &str) -> Vec<CorpusRecord> {
    let mut records = Vec::new();
    let (mut desc, mut prog, mut needs) = (String::new(), String::new(), Vec::new());
    let mut modules: Vec<(String, String)> = Vec::new();
    let mut peers: Vec<(String, String)> = Vec::new();
    let mut trials: Vec<Trial> = Vec::new();
    let mut host_responses: Vec<(String, String)> = Vec::new();
    let mut host_calls: Vec<String> = Vec::new();
    let mut warns: Vec<(String, Option<String>)> = Vec::new();
    let (mut wit_world, mut component_name): (Option<String>, Option<String>) = (None, None);
    let mut live_objects: Option<u32> = None;
    let (mut call_export, mut call_args): (Option<String>, Vec<String>) = (None, Vec::new());
    // The pending `(then …)` continuation's args (two-call-on-one-handle), or `None` until a `then-call`
    // marker line opens it. Flushed into the trial's `Call` alongside `call_args` on the `expect` line.
    let mut second_call: Option<Vec<String>> = None;
    // The pending `(drop)` flag (resource-drop the closure handle after the call), set by a `drop-handle`
    // marker line, flushed into the trial's `Call` on the `expect` line.
    let mut drop_handle = false;
    // The pending `(call-method <member>)` value-resource member, set by a `call-method` line. A method
    // case has NO `call` line (no export), so the trial's `Call` is produced from `method` alone.
    let mut method: Option<String> = None;
    for line in text.lines() {
        if line == "---" {
            records.push(CorpusRecord {
                description: std::mem::take(&mut desc),
                program: std::mem::take(&mut prog),
                modules: std::mem::take(&mut modules),
                peers: std::mem::take(&mut peers),
                trials: std::mem::take(&mut trials),
                needs: std::mem::take(&mut needs),
                host_responses: std::mem::take(&mut host_responses),
                host_calls: std::mem::take(&mut host_calls),
                warns: std::mem::take(&mut warns),
                wit_world: std::mem::take(&mut wit_world),
                component_name: std::mem::take(&mut component_name),
                live_objects: live_objects.take(),
            });
            // Defensive: a well-formed record ends every trial with an `expect`, so nothing is pending.
            call_export = None;
            call_args.clear();
            second_call = None;
            drop_handle = false;
            method = None;
            continue;
        }
        if let Some((key, val)) = line.split_once('\t') {
            match key {
                "case" => desc = val.to_string(),
                "program" => prog = val.to_string(),
                // `module\t<name>\t<program>` — a library file (two tab-separated values). Split the
                // name off the program.
                "module" => {
                    if let Some((name, mprog)) = val.split_once('\t') {
                        modules.push((name.to_string(), mprog.to_string()));
                    }
                }
                // `peer\t<iface>\t<program>` — a cross-component provider (interface + its program). Split
                // the interface off the program; the wasm gate compiles each peer to its own component and
                // composes via `--peer <iface>=<path>`.
                "peer" => {
                    if let Some((iface, pprog)) = val.split_once('\t') {
                        peers.push((iface.to_string(), pprog.to_string()));
                    }
                }
                "call" => call_export = Some(val.to_string()),
                // `call-method\t<member>` — a value-resource member drive (no export; the member is reached
                // on the resource the program's producer makes).
                "call-method" => method = Some(val.to_string()),
                "arg" => call_args.push(val.to_string()),
                // `then-call\t<n>` opens a two-call continuation (n = its arg count, unused — the args
                // arrive as `then-arg` lines); a bare `(then)` emits `then-call\t0` and no `then-arg`, so
                // `Some(vec![])` (a nullary second call) is distinct from `None` (no second call).
                "then-call" => second_call = Some(Vec::new()),
                "then-arg" => {
                    if let Some(sc) = second_call.as_mut() {
                        sc.push(val.to_string());
                    }
                }
                // `drop-handle\t1` — the `(drop)` clause: resource-drop the minted handle after the call.
                "drop-handle" => drop_handle = true,
                "expect" => {
                    // The `expect` closes a trial: pair the pending call (if any) with this payload,
                    // carrying any `(then …)` second-call args and the `(drop)` flag.
                    let sc = second_call.take();
                    let dh = std::mem::take(&mut drop_handle);
                    let m = method.take();
                    // A trial has a call if it named an export OR a `(call-method)` member (the latter has
                    // no export — the program's producer makes the value-resource).
                    let call = if call_export.is_some() || m.is_some() {
                        Some(Call {
                            export: call_export.take().unwrap_or_default(),
                            args: std::mem::take(&mut call_args),
                            second_call: sc,
                            drop_handle: dh,
                            method: m,
                        })
                    } else {
                        None
                    };
                    call_args.clear();
                    trials.push(Trial {
                        call,
                        expect: val.to_string(),
                    });
                }
                "needs" => needs.push(val.to_string()),
                // `host-response\t<op>\t<value>` — a recorded host-call response (two tab-separated
                // values). Split the op off the value.
                "host-response" => {
                    if let Some((op, value)) = val.split_once('\t') {
                        host_responses.push((op.to_string(), value.to_string()));
                    }
                }
                // `host-call\t<op>` — one recorded host operation, in call order.
                "host-call" => host_calls.push(val.to_string()),
                // `warns\t<CODE>` or `warns\t<CODE> (message "phrase")` — a compile-warning pin. Reuse
                // split_message_clause (the `(message …)` parser shared with error/declines) to split the
                // CODE from the optional phrase.
                "warns" => {
                    let (code, message) = split_message_clause(val);
                    warns.push((code.to_string(), message.map(str::to_string)));
                }
                // `wit-world\t<world-sexpr>` / `component-name\t<iface>` — an explicit WIT world the case
                // imposes on the guest (general WIT-ABI shape). Threaded into the wasm emit (world artifact +
                // `--component-name`) and the run (`--call <iface>#<export>`).
                "wit-world" => wit_world = Some(val.to_string()),
                "component-name" => component_name = Some(val.to_string()),
                // `live-objects\t<N>` (or `live-objects\tknown-leak\t<N>` for the opt-out marker) — the
                // post-run heap-balance the case asserts on the debug-counters runtime. Both forms assert
                // == N (the known-leak intent is source-only; the gate needs just the count), so strip an
                // optional `known-leak\t` prefix and parse N.
                "live-objects" => {
                    let count = val.strip_prefix("known-leak\t").unwrap_or(val);
                    // ONE count = uniform; 2+ TAB-separated counts = PER-CALL positional (`live-objects
                    // 3 13 0`, from #5008's every-call surfacing). This DIRECT gate checks the FIRST call's
                    // balance, so it uses the FIRST count; without splitting, `"3\t13\t0".parse` fails →
                    // None → the case falls to the Default(0) check → a spurious pass→fail regression. (The
                    // nix `cdz-run --grade` path reads the full per-call list; this direct path is call[0].)
                    live_objects = count
                        .split('\t')
                        .next()
                        .and_then(|s| s.trim().parse::<u32>().ok());
                }
                _ => {}
            }
        }
    }
    records
}
// moved from xtask/src/main.rs (v-xtask-decompose slice 2b) — &Tools/&Paths → &Path.
/// Run `cdz-syntax --from <from> --to <to>` over `input` bytes (stdin) and return its stdout.
pub fn convert_bytes(cdz_bin: &Path, input: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(cdz_bin)
        .args(["convert", "--from", from, "--to", to, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    child.stdin.take().unwrap().write_all(input).ok();
    let out = child.wait_with_output().expect("wait cdz-syntax");
    out.status.success().then_some(out.stdout)
}
// moved from xtask/src/main.rs (v-xtask-decompose slice 2b) — &Tools/&Paths → &Path.
/// A program's sexpr text → its canonical binary AST bytes (via `cdz-syntax`).
pub fn to_binary(cdz_bin: &Path, program: &str) -> Option<Vec<u8>> {
    convert_bytes(cdz_bin, program.as_bytes(), "sexpr", "binary")
}

// shared util moved from main.rs (v-xtask-decompose slice 2b).
/// A stage's binary could not be spawned at all (missing/not-executable) — distinct from it running
/// and exiting non-zero, which is surfaced by its wait status.
pub fn launch_fail(stage: &str, e: std::io::Error) -> ! {
    eprintln!("xtask run: could not launch {stage}: {e}");
    std::process::exit(1);
}
// shared util moved from main.rs (v-xtask-decompose slice 2b).
pub fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}
// shared util moved from main.rs (v-xtask-decompose slice 2b).
/// Split an `error`/`declines` payload into its leading token (the CODE, or empty for `declines`) and
/// an optional `(message "PHRASE")` clause — the diagnostic-text half of the portable-diagnostic-test
/// capability (operator seq353). E.g. `CDZ0201 (message "malformed record")` → `("CDZ0201", Some("malformed
/// record"))`; `CDZ0201` → `("CDZ0201", None)`; `(message "IEEE partial order")` → `("", Some("IEEE partial
/// order"))`. The PHRASE is graded as a case-sensitive SUBSTRING of the emitted diagnostic message, with
/// NO normalization (v-diagnostics ruling: messages are single-source/single-line, case is load-bearing).
/// A malformed/unterminated `(message …)` yields `None` (the clause is simply not asserted — never panics).
pub fn split_message_clause(payload: &str) -> (&str, Option<&str>) {
    match payload.find("(message ") {
        None => (payload.trim(), None),
        Some(at) => {
            let head = payload[..at].trim();
            let rest = &payload[at + "(message ".len()..];
            // The phrase is a "double-quoted" span: take from the first `"` to the next `"`.
            let phrase = rest
                .strip_prefix('"')
                .and_then(|r| r.split('"').next())
                .filter(|p| !p.is_empty());
            (head, phrase)
        }
    }
}

// ── EMOJI-BAN lint cluster (v-xtask-decompose) — moved from xtask/src/main.rs so the `xtask-lint-emoji`
// command crate AND the xtask `check`/dev-gate steps share ONE detector (no drift). `emoji_free_lint`
// takes the repo ROOT (`&Path`, from CDZ_REPO_ROOT) instead of `&Paths`; the char/line classifiers stay
// private to the module (unit-tested here).

/// True if `c` is an emoji / pictographic / dingbat char the ban rejects (as opposed to legitimate
/// technical typography — em-dash/arrows/box-drawing/math/section/Greek/accented-Latin — which is fine):
/// Miscellaneous Symbols + Dingbats (U+2600–27BF), Misc Symbols & Arrows decorative (U+2B00–2BFF), the
/// `⚑` flag marker (U+2691), the Variation Selectors that render text-glyphs as emoji (U+FE00–FE0F), and
/// the Supplemental/Emoticons/Pictographs planes (U+1F000–1FAFF). Pure so the ranges are unit-tested.
fn is_emoji_char(c: char) -> bool {
    let o = c as u32;
    (0x2600..=0x27BF).contains(&o)      // Misc Symbols + Dingbats (✓ ✗ ✔ ⚠ ☃ ✉ ❯ …)
        || (0x2B00..=0x2BFF).contains(&o)   // Misc Symbols & Arrows decorative (⬆ ⬇ ⭐ …)
        || (0x2691..=0x2691).contains(&o)   // ⚑ flag (used as a status marker)
        || (0xFE00..=0xFE0F).contains(&o)   // Variation Selectors (emoji-presentation ️)
        || (0x1F000..=0x1FAFF).contains(&o) // Emoticons / Pictographs / Supplemental (😀 🔑 🎉 🪤 🔴 🩸 …)
}

/// True if a line is a Rust COMMENT (`///` doc, `//!` inner-doc, `//` line, or a `*` doc-block
/// continuation) — the emoji-ban targets COMMENTS/DOCS (the operator's stated pain), not code. A
/// functional emoji in a string/char literal (an output marker, a Unicode test string) is out of scope;
/// scanning only comment lines skips those structurally instead of by a fragile per-file exception.
fn line_is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("///") || t.starts_with("//!") || t.starts_with("//") || t.starts_with('*')
}

/// True if a comment line legitimately DOCUMENTS a character as the subject of Unicode-handling test
/// data (e.g. "surrogate PAIR U+1F600 😀", `"😀" = 4 bytes`) — stripping the emoji there would break the
/// comment's meaning, and the operator excludes deliberate-Unicode test data. The distinction that
/// matters is whether the emoji is the SUBJECT (documented test data) vs a decorative MARKER: a marker
/// (⚠/⚡ opening a warning) is banned even on a comment that happens to say "byte"/"scalar" in prose (a
/// compiler discusses byte-lengths + scalars constantly). So exclude ONLY on a STRONG codepoint signal
/// (`U+…`, surrogate/astral/codepoint) OR when an emoji appears INSIDE a quoted `"…"` string in the
/// comment (the emoji quoted AS the datum under test). Bare "byte"/"scalar" no longer excludes — that
/// over-match was letting decorative ⚠ markers through (v-agent-harness caught component_store.rs:80).
fn is_unicode_test_doc(line: &str) -> bool {
    let strong = line.contains("U+")
        || line.contains("surrogate")
        || line.contains("astral")
        || line.contains("codepoint");
    strong || emoji_inside_a_quote(line)
}

/// True if any emoji char appears inside a double-quoted `"…"` substring of `line` — i.e. the emoji is
/// quoted as a literal datum (a Unicode test string the comment documents, like `"😀" = 4 bytes`), not a
/// bare decorative marker. A simple quote-state scan; pure + unit-tested.
fn emoji_inside_a_quote(line: &str) -> bool {
    let mut in_quote = false;
    for c in line.chars() {
        if c == '"' {
            in_quote = !in_quote;
        } else if in_quote && is_emoji_char(c) {
            return true;
        }
    }
    false
}

/// Every (1-based line, emoji char) an emoji-ban lint would FLAG in `text`: emoji chars that appear in a
/// COMMENT line that is NOT Unicode-test documentation. This is the exact predicate the cleanup applied,
/// factored out pure so both the lint and its scope are unit-tested off the filesystem. Shared with the
/// xtask dev-gate `dev_gate_emoji_warn` step (WARN-only) and the `check` `emoji-free` step.
pub fn banned_emoji_hits(text: &str) -> Vec<(usize, char)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line_is_comment(line) && !is_unicode_test_doc(line))
        .flat_map(|(i, line)| {
            line.chars()
                .filter(|c| is_emoji_char(*c))
                .map(move |c| (i + 1, c))
        })
        .collect()
}

/// Recursively collect every `*.rs` file under `dir` into `out`. Skips `target/` build dirs. A plain
/// helper for the emoji lint's file walk (the corpus lints read a flat dir; source is a tree).
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// EMOJI-BAN lint (operator directive 2026-08-07: "ban emojis in the codebase ... tired of seeing
/// emojis"). Walks every `implementation/**/*.rs` source file under `repo` and returns `Err` if any
/// emoji / pictographic / dingbat char appears in a NON-Unicode-test-doc COMMENT. SCOPE (concierge-
/// confirmed emoji-only, NOT all-non-ASCII — the compiler's em-dash/arrow/box/math typography is
/// legitimate and stays): only `implementation/` source (the compiler + libraries), NOT `xtask/` (fleet-
/// tooling output markers like the inbox glyphs are functional) and NOT `fleet/`. Comment-scoped, so a
/// functional emoji in a string/char literal (a Unicode test string, an output marker) is structurally
/// out of scope. Fails loudly if it cannot enumerate its inputs (a silent 0-file pass would let an emoji
/// slip in), mirroring `needs_free_lint`. `repo` is the repo root (CDZ_REPO_ROOT for the nix app).
pub fn emoji_free_lint(repo: &Path) -> Result<(), String> {
    let root = repo.join("implementation");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs_files(&root, &mut files).map_err(|e| {
        format!(
            "cannot enumerate {} for the emoji lint: {e}",
            root.display()
        )
    })?;
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "no *.rs source files found under {} — the emoji lint would pass vacuously",
            root.display()
        ));
    }
    let mut hits: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read {} for the emoji lint: {e}", file.display()))?;
        let rel = file.strip_prefix(repo).unwrap_or(file).display();
        for (line_no, ch) in banned_emoji_hits(&text) {
            hits.push(format!("{rel}:{line_no}: {ch:?} (U+{:04X})", ch as u32));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    Err(format!(
        "found {} emoji character(s) in source COMMENTS — the codebase bans emojis (operator directive; \
         legitimate technical typography — em-dash, arrows, box-drawing, math — is fine, only \
         emoji/pictographic/dingbat chars are rejected). Replace with an ASCII label (e.g. WARNING:, \
         CRITICAL:, KEYSTONE:) or drop it. At:\n  {}",
        hits.len(),
        hits.join("\n  ")
    ))
}

// ── GATE-BASELINE text algebra (v-xtask-decompose) — the pure, std-only baseline-text primitives shared
// by the gate (verdict grading + save), the `merge-baseline` git driver, the `check` baseline-no-dup lint,
// and the standalone `xtask-canonicalize-baselines` command. Moved here so the canonicalizer command crate
// reuses the ONE canonicalize/merge core (drift-sensitive) instead of duplicating it, and so the eventual
// gate extraction pulls `Verdict` from the foundation. No new deps (std-only).

/// A single case's baseline verdict. `pass`/`todo`/`fail` — the three states a `.gate-baseline*` line can
/// carry (todo = a known decline; fail = a hard failure pinned against regression).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Todo,
    Fail,
}

impl Verdict {
    pub fn tag(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Todo => "todo",
            Verdict::Fail => "fail",
        }
    }
    pub fn parse(s: &str) -> Option<Verdict> {
        match s {
            "pass" => Some(Verdict::Pass),
            "todo" => Some(Verdict::Todo),
            "fail" => Some(Verdict::Fail),
            _ => None,
        }
    }
}

/// The exact serialized form `save_baseline` writes: a `#` header then one `verdict\tdescription` line
/// per case, sorted. Factored out so the file-level canonicalizer ([`canonicalize_baseline_text`])
/// produces a byte-identical file WITHOUT needing a gate run to rebuild verdicts. Pure.
pub fn serialize_baseline(by_desc: &std::collections::BTreeMap<String, Verdict>) -> String {
    let mut lines: Vec<String> = by_desc
        .iter()
        .map(|(d, v)| format!("{}\t{d}", v.tag()))
        .collect();
    lines.sort();
    format!(
        "# gate baseline — per-case verdicts (verdict\\tdescription). Regenerate with `cargo xtask gate --save`.\n{}\n",
        lines.join("\n")
    )
}

/// Why a baseline union can't be auto-resolved — distinguishes the two `canonicalize_baseline_text`
/// `Ok(None)`/`Err` cases the merge driver must treat DIFFERENTLY (a conflict → leave for a human; an
/// unparseable line → also leave for a human, but a distinct reason). Kept separate so the driver never
/// writes data it can't model. Pure + unit-testable.
#[derive(Debug, PartialEq, Eq)]
pub enum BaselineMergeErr {
    /// Same description, different verdicts on the two sides — a real integrity conflict.
    Conflict(Vec<String>),
    /// A non-comment, non-blank line that isn't `verdict\tdescription` — we don't understand it, so we
    /// won't rewrite (a rewrite would silently drop it).
    Unparseable,
}

/// Union two gate-baseline texts into the canonical sorted + verdict-aware-deduped form, or say why not.
/// The pure core of the `merge-baseline` git driver: unlike [`canonicalize_baseline_text`] (whose
/// `Ok(None)` conflates "already canonical" with "leave alone"), this ALWAYS returns the canonical union
/// string on success, so the driver can write it unconditionally. Errors distinguish a real
/// different-verdict [`BaselineMergeErr::Conflict`] from an [`BaselineMergeErr::Unparseable`] line.
pub fn merge_baseline_union(ours: &str, theirs: &str) -> Result<String, BaselineMergeErr> {
    let mut by_desc: std::collections::BTreeMap<String, Verdict> =
        std::collections::BTreeMap::new();
    let mut conflicting: Vec<String> = Vec::new();
    for line in ours.lines().chain(theirs.lines()) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((v, d)) = line.split_once('\t') else {
            return Err(BaselineMergeErr::Unparseable);
        };
        let Some(verdict) = Verdict::parse(v) else {
            return Err(BaselineMergeErr::Unparseable);
        };
        match by_desc.insert(d.to_string(), verdict) {
            None => {}
            Some(prev) if prev == verdict => {} // benign same-verdict dup — collapsed
            Some(_) => conflicting.push(d.to_string()),
        }
    }
    if !conflicting.is_empty() {
        conflicting.sort();
        conflicting.dedup();
        return Err(BaselineMergeErr::Conflict(conflicting));
    }
    Ok(serialize_baseline(&by_desc))
}

/// Canonicalize a gate-baseline FILE from its text alone — sort + de-dup verdict-aware — WITHOUT a gate
/// run. This is the root-fix for the `merge=union` re-accumulation (concierge assign 2026-08-10, option
/// (a)): the `.gate-baseline*` files carry `merge=union` so every concurrent baseline append merges BOTH
/// sides' rows, re-injecting benign same-verdict duplicate lines; the within-file `baseline_no_dup_titles`
/// lint then reds `cargo xtask check` FLEET-WIDE, and a manual dedup MR can never win the race against the
/// steady append stream (corpus-bugfix's heal was rebuilt 3+ times). Running this as a pr-sync POST-LAND
/// step lands every baseline already-canonical, killing the accumulation at the source while KEEPING
/// `merge=union`'s conflict-free appends (best of both).
///
/// Verdict-aware, per the assign's must-not-silently-dedup requirement:
/// - same title + SAME verdict  → a benign `merge=union` duplicate → collapse to one line.
/// - same title + DIFFERENT verdict → a REAL integrity conflict (the map-keyed baseline would mask one
///   via last-wins) → return `Err(conflicting titles)`. The caller MUST surface this, never silently pick
///   a side. (This mirrors `check_baseline`'s benign-vs-conflicting split, at the file layer.)
///
/// Returns `Ok(Some(canonical))` when the input was NON-canonical (caller rewrites the file),
/// `Ok(None)` when it was ALREADY canonical (caller writes nothing — no dirty worktree, no churn), and
/// `Err(titles)` on a conflicting dup. Malformed lines (no tab / unknown verdict tag) are preserved by
/// being ignored here only for the dedup decision — but since a rewrite would DROP them, we treat any
/// unparseable non-comment/non-blank line as "leave the file alone" (never eat data we don't understand).
pub fn canonicalize_baseline_text(text: &str) -> Result<Option<String>, Vec<String>> {
    let mut by_desc: std::collections::BTreeMap<String, Verdict> =
        std::collections::BTreeMap::new();
    let mut conflicting: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // A non-comment, non-blank line that doesn't parse as `verdict\tdescription` is unexpected. Refuse
        // to rewrite (a rewrite would silently DROP it) — treat the file as "already canonical / hands
        // off" so we never eat data we don't understand. Zero-risk: the no-dup lint still guards dups.
        let Some((v, d)) = line.split_once('\t') else {
            return Ok(None);
        };
        let Some(verdict) = Verdict::parse(v) else {
            return Ok(None);
        };
        match by_desc.insert(d.to_string(), verdict) {
            None => {}
            Some(prev) if prev == verdict => {} // benign same-verdict dup — collapsed by the map insert
            Some(_) => conflicting.push(d.to_string()),
        }
    }
    if !conflicting.is_empty() {
        conflicting.sort();
        conflicting.dedup();
        return Err(conflicting);
    }
    let canonical = serialize_baseline(&by_desc);
    if canonical == text {
        Ok(None) // already canonical — caller writes nothing (keeps the worktree clean)
    } else {
        Ok(Some(canonical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_baseline_text_collapses_benign_dups_and_sorts() {
        // The core of the merge=union root-fix: a file with same-verdict duplicate lines (the merge
        // artifact) canonicalizes to one sorted line per case. Unsorted + dup'd input → canonical output.
        let input = "# gate baseline — per-case verdicts (verdict\\tdescription). Regenerate with `cargo xtask gate --save`.\n\
                     pass\tzeta case\n\
                     pass\talpha case\n\
                     pass\tzeta case\n\
                     todo\tbeta case\n";
        let out = canonicalize_baseline_text(input)
            .expect("no conflict")
            .expect("input was non-canonical → rewritten");
        // Sorted by the full `verdict\tdesc` line, one per case, dup collapsed, canonical header + trailing \n.
        assert_eq!(
            out,
            "# gate baseline — per-case verdicts (verdict\\tdescription). Regenerate with `cargo xtask gate --save`.\n\
             pass\talpha case\n\
             pass\tzeta case\n\
             todo\tbeta case\n"
        );
        // Idempotent: canonicalizing the OUTPUT is a no-op (already canonical → Ok(None), no rewrite).
        assert_eq!(
            canonicalize_baseline_text(&out).expect("no conflict"),
            None,
            "an already-canonical file must not be rewritten (keeps the worktree clean)"
        );
    }

    #[test]
    fn canonicalize_baseline_text_surfaces_a_conflicting_dup_never_silently_picks() {
        // The assign's hard requirement: same title + DIFFERENT verdict is a REAL conflict (the map-keyed
        // baseline would mask one via last-wins) — it must be SURFACED, never silently deduped.
        let input = "pass\tcontested case\n\
                     todo\tcontested case\n\
                     pass\tfine case\n";
        let err = canonicalize_baseline_text(input)
            .expect_err("a pass-vs-todo conflict on the same title must be surfaced");
        assert_eq!(err, vec!["contested case".to_string()]);
    }

    #[test]
    fn canonicalize_baseline_text_leaves_an_unparseable_file_alone() {
        // A non-comment/non-blank line that isn't `verdict\tdescription` (no tab, or unknown verdict tag)
        // is data we don't understand — refuse to rewrite (a rewrite would DROP it), returning Ok(None).
        assert_eq!(
            canonicalize_baseline_text("pass\tok case\nthis line has no tab\n")
                .expect("no conflict"),
            None,
            "a line with no tab → hands off (never eat unrecognized data)"
        );
        assert_eq!(
            canonicalize_baseline_text("mystery\tsome case\n").expect("no conflict"),
            None,
            "an unknown verdict tag → hands off"
        );
    }

    #[test]
    fn merge_baseline_union_dedups_benign_conflicts_on_verdict_and_declines_unparseable() {
        // The recurring toil: two sides each appended the SAME verdict\tdesc line. Union + dedup → ONE
        // line (the merge=union bug was keeping BOTH). Result is canonical (sorted, header, deduped).
        let ours = "# hdr\npass\tcase a\npass\tshared\n";
        let theirs = "# hdr\npass\tshared\ntodo\tcase b\n";
        let merged = merge_baseline_union(ours, theirs).expect("clean union");
        // `shared` appears exactly once; both distinct cases present; sorted.
        assert_eq!(
            merged.matches("\tshared").count(),
            1,
            "benign dup collapsed to one line"
        );
        assert!(merged.contains("pass\tcase a") && merged.contains("todo\tcase b"));
        // Idempotent: unioning the merged result with itself is a fixpoint (no re-accumulation).
        assert_eq!(
            merge_baseline_union(&merged, &merged).expect("idempotent"),
            merged,
            "re-merging the canonical union must be a fixpoint"
        );
        // Same description, DIFFERENT verdict on the two sides → Conflict (never silently pick one).
        match merge_baseline_union("pass\tx\n", "todo\tx\n") {
            Err(BaselineMergeErr::Conflict(t)) => assert_eq!(t, vec!["x".to_string()]),
            other => panic!("expected Conflict, got {other:?}"),
        }
        // An unparseable line (no tab / unknown tag) → Unparseable, never rewritten.
        assert_eq!(
            merge_baseline_union("pass\tok\nno-tab-line\n", ""),
            Err(BaselineMergeErr::Unparseable)
        );
        assert_eq!(
            merge_baseline_union("mystery\tcase\n", ""),
            Err(BaselineMergeErr::Unparseable)
        );
    }

    #[test]
    fn banned_emoji_hits_scopes_to_non_testdoc_comments() {
        // The char classifier: emojis/dingbats flagged; technical typography (em-dash/arrows/box/math/
        // section/Greek/accented-Latin) is NOT — banning THOSE would be scope (A), rejected.
        assert!(
            is_emoji_char('😀') && is_emoji_char('🔑') && is_emoji_char('⚠') && is_emoji_char('⚑')
        );
        assert!(!is_emoji_char('—') && !is_emoji_char('→') && !is_emoji_char('─'));
        assert!(
            !is_emoji_char('∀')
                && !is_emoji_char('≥')
                && !is_emoji_char('§')
                && !is_emoji_char('é')
        );

        // FLAGGED: an emoji in a plain comment (1-based line + the char).
        assert_eq!(
            banned_emoji_hits("// 🪤 a trap\nlet x = 1;\n"),
            vec![(1, '🪤')]
        );
        assert_eq!(
            banned_emoji_hits("// ok\n// status ✓ vs ✗\n"),
            vec![(2, '✓'), (2, '✗')]
        );

        // NOT flagged: technical typography in a comment (the whole point of scope B).
        assert!(banned_emoji_hits("/// A → B ⇒ C — ∀x ∈ S, x ≥ 0 ≠ 1 … ── §4 café β\n").is_empty());

        // NOT flagged: an emoji in CODE (a string/char literal) — comment-scoped, so functional emoji
        // (output markers, Unicode test strings) are structurally out of scope.
        assert!(banned_emoji_hits("let s = \"👍\".repeat(3);\n").is_empty());
        assert!(banned_emoji_hits("let mark = if act { \"⚑\" } else { \".\" };\n").is_empty());

        // NOT flagged: a COMMENT that documents Unicode test data — via a STRONG codepoint signal...
        assert!(banned_emoji_hits("// surrogate PAIR U+1F600 😀 is a 4-byte scalar\n").is_empty());
        assert!(banned_emoji_hits("// a😀b: scalar 1 = U+1F600\n").is_empty());
        // ...OR the emoji QUOTED as the datum under test (no U+ needed, e.g. `"😀" = 4 bytes`).
        assert!(banned_emoji_hits("// `\"😀\"` = 4 (four bytes vs one scalar)\n").is_empty());

        // BUT a DECORATIVE marker is flagged even on a comment that says byte/scalar in prose — the
        // over-broad bare-"byte" exclusion was letting these through (v-agent-harness, component_store).
        assert_eq!(
            banned_emoji_hits("/// ⚠ the 32-byte SHA-256 address — a scalar walk\n"),
            vec![(1, '⚠')]
        );
    }

    #[test]
    fn hash_tree_is_deterministic_and_change_sensitive() {
        let base = std::env::temp_dir().join(format!("cdz-hashtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.cdz"), "alpha").unwrap();
        std::fs::write(base.join("sub/b.cdz"), "beta").unwrap();

        let h1 = hash_tree(&base).expect("hashable");
        let h2 = hash_tree(&base).expect("hashable");
        assert_eq!(
            h1, h2,
            "same tree → same hash (order-independent, deterministic)"
        );

        // A content edit changes the hash.
        std::fs::write(base.join("a.cdz"), "alpha!").unwrap();
        let h3 = hash_tree(&base).expect("hashable");
        assert_ne!(h1, h3, "editing a file's content changes the tree hash");

        // Adding a file changes the hash.
        std::fs::write(base.join("c.cdz"), "gamma").unwrap();
        let h4 = hash_tree(&base).expect("hashable");
        assert_ne!(h3, h4, "adding a file changes the tree hash");

        // A rename (same bytes, different path) changes the hash — path is folded in.
        std::fs::remove_file(base.join("c.cdz")).unwrap();
        std::fs::write(base.join("d.cdz"), "gamma").unwrap();
        let h5 = hash_tree(&base).expect("hashable");
        assert_ne!(h4, h5, "a rename changes the tree hash (path folded in)");

        let _ = std::fs::remove_dir_all(&base);
    }
}

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
    /// The live-heap-cell residual a CLEAN `(live-objects N)` clause asserts after the run. `None` if absent.
    /// Unused (kept `None`/ignored) for a known-leak case — see `live_objects_known_leak`.
    pub live_objects: Option<u32>,
    /// `true` iff the case carries the seq-15 PURE-BINARY `(live-objects known-leak)` marker — an
    /// accepted-as-leaking case that is NOT count-checked (its leak magnitude does not matter). The gate maps
    /// it to `LiveObjectsCheck::Off`.
    pub live_objects_known_leak: bool,
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

/// The repo ROOT: from `CDZ_REPO_ROOT` (the nix-app wrappers set it to the invoking worktree, since a
/// relocated nix binary can't self-locate), else the current dir (a bare `cargo run`). The one place every
/// decomposed xtask leaf-crate resolves its root, so the env-var contract lives in a single spot.
pub fn repo_root() -> PathBuf {
    std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"))
}

/// The seed-toolchain BINARY dir: from `CDZ_SEED_BIN_DIR` (the nix apps inject the warm nix-built `cdz`/
/// `cdz-corpus`), else `<repo>/target/debug` for a bare `cargo run`. Pairs with [`repo_root`].
pub fn seed_bin_dir(repo: &Path) -> PathBuf {
    std::env::var_os("CDZ_SEED_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/debug"))
}

/// The three committed gate-baseline files (repo-relative), in `[wasm, rust, rust-async]` order — the
/// canonical set the baseline canonicalizer/pruner sweep. One copy so the list can't drift between them.
pub const BASELINE_REL: [&str; 3] = [
    "spec/semantics/.gate-baseline",
    "spec/semantics/.gate-baseline-rust",
    "spec/semantics/.gate-baseline-rust-async",
];

/// A dependency-free, RAII unique temp directory for TESTS (no `tempfile` dep — that would need
/// same-window flake registration). The dir is `<system-temp>/<prefix><pid>-<counter>`, uniqued by
/// pid + a process-wide counter so parallel `cargo test` runs never collide; `Drop` reaps it, so it
/// is cleaned up even if a test panics. Shared here so the decomposed xtask crates don't each re-grow
/// the same struct.
pub struct TmpDir(PathBuf);

impl TmpDir {
    /// Create a fresh unique temp dir named `<prefix><pid>-<counter>`.
    pub fn new(prefix: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "{prefix}{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir)
    }

    /// The temp dir's root path.
    pub fn dir(&self) -> &Path {
        &self.0
    }

    /// `<dir>/<rel>` (parent dirs are NOT created — use [`TmpDir::write`] for that).
    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// Write `contents` to `<dir>/<rel>` (creating any parent dirs) and return the written path.
    pub fn write(&self, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, contents).unwrap();
        p
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    let mut live_objects_known_leak = false;
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
                live_objects_known_leak: std::mem::take(&mut live_objects_known_leak),
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
                // `live-objects\t<N>` — a CLEAN case's post-run residual (the reachable-return count; N=0 =
                // fully reclaimed), asserted on the debug-counters runtime. `live-objects\tknown-leak`
                // (seq-15 PURE-BINARY marker, NO count) = accepted-as-leaking, NOT count-checked (the gate
                // maps it to `LiveObjectsCheck::Off`). A legacy `known-leak\t<N>` still parses (count ignored
                // under binary).
                "live-objects" => {
                    if let Some(rest) = val.strip_prefix("known-leak") {
                        live_objects_known_leak = true;
                        // Bare marker → no count; a legacy `known-leak\t<N>` leaves the (ignored) first count.
                        live_objects = rest
                            .trim_start_matches('\t')
                            .split('\t')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok());
                    } else {
                        // ONE count = uniform; 2+ TAB-separated counts = PER-CALL positional (`live-objects
                        // 0 0 0`). This DIRECT gate checks the FIRST call's balance, so it uses the FIRST
                        // count. (The nix `cdz-run --grade` path reads the full per-call list; this is call[0].)
                        live_objects = val
                            .split('\t')
                            .next()
                            .and_then(|s| s.trim().parse::<u32>().ok());
                    }
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

/// The source-file byte ceiling (operator directive seq-274): 512 KiB. Above ~512 KB GitHub stops
/// syntax-highlighting a file, so an oversized source file becomes un-highlighted and hard to review.
/// Keyed on BYTES (GitHub's actual cutoff), not LOC.
pub const MAX_SOURCE_BYTES: u64 = 512 * 1024;

/// Files already over [`MAX_SOURCE_BYTES`] at seq-274 adoption, GRANDFATHERED pending a split (repo-relative
/// paths). The lint blocks any NEW oversized file without red-flagging the fleet on day one. SELF-EXPIRING:
/// an entry that no longer exists OR has dropped back under the limit is STALE and FAILS the lint, forcing
/// its removal — so this list can only SHRINK as files are split, never rot. REMOVE an entry once its file
/// is split under the limit. (cdz-runtime/lib.rs is being split under seq-273.)
pub const FILE_SIZE_ALLOWLIST: &[&str] = &[];

/// FILE-SIZE lint (operator directive seq-274): FAIL if any `implementation/**/*.rs` source file exceeds
/// [`MAX_SOURCE_BYTES`], EXCEPT the grandfathered [`FILE_SIZE_ALLOWLIST`]. Also FAILS on a STALE allowlist
/// entry (a file no longer over-limit or missing) so the grandfather set can only shrink. `repo` is the repo
/// root (CDZ_REPO_ROOT for the nix app). Mirrors `emoji_free_lint`'s enumerate-or-fail-loudly discipline.
pub fn file_size_lint(repo: &Path) -> Result<(), String> {
    file_size_lint_with(repo, FILE_SIZE_ALLOWLIST)
}

/// The testable core of [`file_size_lint`], parameterized on the allowlist so hermetic tests can supply a
/// synthetic one (the real [`FILE_SIZE_ALLOWLIST`] names paths that don't exist under a temp fixture).
fn file_size_lint_with(repo: &Path, allowlist: &[&str]) -> Result<(), String> {
    let root = repo.join("implementation");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs_files(&root, &mut files).map_err(|e| {
        format!(
            "cannot enumerate {} for the file-size lint: {e}",
            root.display()
        )
    })?;
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "no *.rs source files found under {} — the file-size lint would pass vacuously",
            root.display()
        ));
    }
    let allow: std::collections::BTreeSet<&str> = allowlist.iter().copied().collect();
    let mut seen_allowed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        let len = std::fs::metadata(file)
            .map_err(|e| format!("cannot stat {} for the file-size lint: {e}", file.display()))?
            .len();
        if len <= MAX_SOURCE_BYTES {
            continue;
        }
        let rel = file
            .strip_prefix(repo)
            .unwrap_or(file)
            .display()
            .to_string();
        if allow.contains(rel.as_str()) {
            seen_allowed.insert(rel);
        } else {
            offenders.push(format!(
                "{rel}: {len} bytes (over the {MAX_SOURCE_BYTES}-byte limit) — split it into smaller modules"
            ));
        }
    }
    let stale: Vec<&str> = allowlist
        .iter()
        .copied()
        .filter(|e| !seen_allowed.contains(*e))
        .collect();
    if offenders.is_empty() && stale.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    if !offenders.is_empty() {
        msg.push_str(&format!(
            "found {} source file(s) over the {MAX_SOURCE_BYTES}-byte (512 KiB) limit — GitHub stops \
             syntax-highlighting above ~512 KB, so split each into smaller modules. At:\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        ));
    }
    if !stale.is_empty() {
        if !msg.is_empty() {
            msg.push_str("\n\n");
        }
        msg.push_str(&format!(
            "{} STALE file-size-allowlist entr(ies) — now under the limit or missing; REMOVE from \
             FILE_SIZE_ALLOWLIST (the grandfather list must shrink as files are split):\n  {}",
            stale.len(),
            stale.join("\n  ")
        ));
    }
    Err(msg)
}

/// Lexically resolve `.`/`..` in `p` WITHOUT touching the filesystem (no symlink/existence dependency), so
/// a `#[path]` target normalizes even before the file exists in a build. Input is expected absolute.
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The crate root of `file` — the nearest ancestor directory (up to `repo`) holding a `Cargo.toml`.
fn crate_root_of(file: &Path, repo: &Path) -> Option<PathBuf> {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d.join("Cargo.toml").is_file() {
            return Some(d.to_path_buf());
        }
        if d == repo {
            break;
        }
        dir = d.parent();
    }
    None
}

/// Extract the string literal from a `#[path = "…"]` attribute line (the text between the first pair of
/// double quotes). `None` if there is no quoted value (so a non-attribute line is skipped).
fn extract_path_literal(attr_line: &str) -> Option<String> {
    let start = attr_line.find('"')? + 1;
    let rest = attr_line.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Cross-crate `#[path]` source-includes present at seq-275 adoption, GRANDFATHERED pending removal (each
/// keyed by the INCLUDING file's repo-relative path + the exact `#[path]` literal). SELF-EXPIRING: an entry
/// that is no longer a cross-crate include (removed / file gone) is STALE and FAILS the lint, so this list
/// can only shrink. The cdz-runtime -> cadenza-ast three are owned by v-runtime under seq-273; the cdz-num
/// -> cdz-runtime one is tracked with v-runtime. REMOVE an entry once its include is converted to a dep.
pub const PATH_INCLUDE_ALLOWLIST: &[(&str, &str)] = &[
    // The last remaining cross-crate #[path] include (v-runtime's, seq-275 sweep: extract bigint into a
    // shared no_std crate). Drop this entry when that extraction lands. (The cdz-runtime -> cadenza-ast
    // three were removed by seq-273 slice-1, #5931, and dropped from this allowlist accordingly.)
    (
        "implementation/seed/crates/cdz-num/src/lib.rs",
        "../../cdz-runtime/src/bigint.rs",
    ),
];

/// CROSS-CRATE `#[path]` SOURCE-INCLUDE lint (operator directive seq-275): FAIL on any `#[path = "…"]`
/// attribute whose target resolves OUTSIDE the including crate's own `src/` (i.e. into a SIBLING crate) —
/// exactly the source-share that breaks crates.io publishability ("i don't want to see any other cross
/// crate source includes … make those unpublishable to crates.io"). Same-crate `#[path]` (a file under the
/// crate's own root) is fine and left alone. EXCEPT the grandfathered [`PATH_INCLUDE_ALLOWLIST`]; a stale
/// allowlist entry FAILS so the set can only shrink as includes are converted to proper dependencies.
/// `repo` is the repo root (CDZ_REPO_ROOT for the nix app). Mirrors `emoji_free_lint`'s discipline.
pub fn cross_crate_path_include_lint(repo: &Path) -> Result<(), String> {
    cross_crate_path_include_lint_with(repo, PATH_INCLUDE_ALLOWLIST)
}

/// Testable core of [`cross_crate_path_include_lint`], parameterized on the allowlist.
fn cross_crate_path_include_lint_with(
    repo: &Path,
    allowlist: &[(&str, &str)],
) -> Result<(), String> {
    let root = repo.join("implementation");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs_files(&root, &mut files).map_err(|e| {
        format!(
            "cannot enumerate {} for the cross-crate #[path] lint: {e}",
            root.display()
        )
    })?;
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "no *.rs source files found under {} — the cross-crate #[path] lint would pass vacuously",
            root.display()
        ));
    }
    let allow: std::collections::BTreeSet<(&str, &str)> = allowlist.iter().copied().collect();
    let mut seen: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| {
            format!(
                "cannot read {} for the cross-crate #[path] lint: {e}",
                file.display()
            )
        })?;
        let file_rel = file
            .strip_prefix(repo)
            .unwrap_or(file)
            .display()
            .to_string();
        let Some(crate_root) = crate_root_of(file, repo) else {
            continue;
        };
        let base = file.parent().unwrap_or(file);
        for (i, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with("#[path") {
                continue; // an actual attribute at line-start, NOT a comment mentioning #[path]
            }
            let Some(lit) = extract_path_literal(line.trim_start()) else {
                continue;
            };
            let target = lexical_normalize(&base.join(&lit));
            if target.starts_with(&crate_root) {
                continue; // same-crate include — allowed
            }
            if allow.contains(&(file_rel.as_str(), lit.as_str())) {
                seen.insert((file_rel.clone(), lit.clone()));
            } else {
                let tgt = target.strip_prefix(repo).unwrap_or(&target).display();
                offenders.push(format!(
                    "{file_rel}:{}: #[path = \"{lit}\"] -> {tgt} (outside the crate's own src/ — a \
                     cross-crate SOURCE include breaks crates.io publishability; use a proper dependency)",
                    i + 1
                ));
            }
        }
    }
    let stale: Vec<String> = allowlist
        .iter()
        .filter(|(f, l)| !seen.contains(&((*f).to_string(), (*l).to_string())))
        .map(|(f, l)| format!("{f} -> {l}"))
        .collect();
    if offenders.is_empty() && stale.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    if !offenders.is_empty() {
        msg.push_str(&format!(
            "found {} cross-crate #[path] source-include(s) — the codebase forbids source-including a \
             sibling crate's file (it breaks crates.io publishability); convert each to a proper crate \
             dependency. At:\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        ));
    }
    if !stale.is_empty() {
        if !msg.is_empty() {
            msg.push_str("\n\n");
        }
        msg.push_str(&format!(
            "{} STALE #[path]-allowlist entr(ies) — no longer a cross-crate include; REMOVE from \
             PATH_INCLUDE_ALLOWLIST (the grandfather list must shrink as includes are removed):\n  {}",
            stale.len(),
            stale.join("\n  ")
        ));
    }
    Err(msg)
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

/// Parse `<verdict>\t<title>` lines (the baseline / harvested-verdict format) into `(title, verdict)`
/// pairs; `#`/blank lines and unparseable lines (no tab, or an unknown verdict tag) are skipped.
/// `split_once('\t')` keeps everything after the FIRST tab as the title (a stray tab stays in the title,
/// matching the baseline's own map-load). The shared reader for every `--compare`/regeneration consumer.
pub fn parse_verdicts(text: &str) -> Vec<(String, Verdict)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            l.split_once('\t')
                .and_then(|(v, d)| Verdict::parse(v).map(|verdict| (d.to_string(), verdict)))
        })
        .collect()
}

/// The result of comparing a run's verdicts against a committed `.gate-baseline*` — the whole-pass
/// baseline-regression fold, backend-independent, carrying the FIVE canonical invariants (blessed by
/// v-corpus-harness as the single source of truth; the semantics `gate --check`, `gate-syntax --check`,
/// the quote/rust/rust-async harvests, and the `xtask-check-baseline` leaf all grade through this):
///  1. REGRESSION: only `pass → not-pass` (pass→fail AND pass→todo) reds; `todo → pass` is `gained`
///     (additive, never reds); a steady pass/todo is quiet.
///  2. GAINED: a `not-pass → pass` case — reported, never failing (prompts a re-baseline).
///  3. FAILING gate-hole guard: a `todo`-baselined OR absent case that now `fail`s reds even though it
///     is not a pass→not-pass regression.
///  4. TRACKED KNOWN-FAIL: a `fail` verdict against an explicit `fail` baseline is a deliberate,
///     git-committed pin — reported (`tracked_fail`) for visibility but NOT a gate failure.
///  5. VANISHED: a baseline title with no current case reds only on a FULL run; a `subset` run
///     (`--files`/`--case`) skips it (the case lives in another selection).
///
/// A CONFLICTING duplicate baseline line (same title, different verdicts) is a hard integrity error
/// (`exit_code` 3); a BENIGN same-verdict duplicate (a `merge=union` artifact) is counted, harmless.
#[derive(Debug, Default, PartialEq)]
pub struct BaselineCompare {
    /// `pass → not-pass` regressions, formatted `title (was → now)`.
    pub regressed: Vec<String>,
    /// Baseline titles absent from this run (full run only).
    pub vanished: Vec<String>,
    /// Current fails not covered by a `pass`/`fail` baseline (the gate-hole guard).
    pub failing: Vec<String>,
    /// Current fails pinned by an explicit `fail` baseline — visible, not gate-redding.
    pub tracked_fail: Vec<String>,
    /// Cases that went `not-pass → pass` — additive, reported but never failing.
    pub gained: Vec<String>,
    /// A CONFLICTING duplicate title in the baseline (different verdicts) — a hard integrity error.
    pub conflict: Vec<String>,
    /// Count of BENIGN same-verdict duplicate lines (a `merge=union` artifact) — harmless.
    pub benign_dups: usize,
}

impl BaselineCompare {
    /// The process exit code: 3 on a conflicting-dup integrity error, 1 on any regression/vanished/
    /// failing, else 0 (`tracked_fail`/`gained`/`benign_dups` never red).
    pub fn exit_code(&self) -> i32 {
        if !self.conflict.is_empty() {
            3
        } else if !self.regressed.is_empty()
            || !self.vanished.is_empty()
            || !self.failing.is_empty()
        {
            1
        } else {
            0
        }
    }
}

/// Compare current `verdicts` against `baseline_text` — the PURE, I/O-free core carrying the five
/// invariants documented on [`BaselineCompare`]. `subset` is a `--files`/`--case` run (skips vanished).
/// The `BTreeMap` load makes the reported lists DETERMINISTIC (sorted) — a strict improvement over a
/// `HashMap` iteration, same cases/exit code. The canonical fold every consumer delegates to.
pub fn compare_verdicts_baseline(
    verdicts: &[(String, Verdict)],
    baseline_text: &str,
    subset: bool,
) -> BaselineCompare {
    use std::collections::BTreeMap;
    let mut out = BaselineCompare::default();
    // Parse the baseline into a title→verdict map, splitting BENIGN (same-verdict) from CONFLICTING
    // (different-verdict) duplicate lines as we go.
    let mut base: BTreeMap<String, Verdict> = BTreeMap::new();
    for line in baseline_text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            match base.insert(d.to_string(), verdict) {
                None => {}
                Some(prev) if prev == verdict => out.benign_dups += 1,
                Some(_) => out.conflict.push(d.to_string()),
            }
        }
    }
    if !out.conflict.is_empty() {
        out.conflict.sort();
        out.conflict.dedup();
        return out; // a conflicting baseline is an integrity error — do not compare against it.
    }

    let now: BTreeMap<&str, Verdict> = verdicts.iter().map(|(d, v)| (d.as_str(), *v)).collect();
    for (desc, &was) in &base {
        match now.get(desc.as_str()) {
            None => {
                if !subset {
                    out.vanished.push(desc.clone());
                }
            }
            Some(&is) if was == Verdict::Pass && is != Verdict::Pass => {
                out.regressed
                    .push(format!("{desc} ({} → {})", was.tag(), is.tag()));
            }
            Some(&is) if was != Verdict::Pass && is == Verdict::Pass => {
                out.gained.push(desc.clone())
            }
            Some(_) => {}
        }
    }
    for (d, v) in verdicts {
        if *v != Verdict::Fail {
            continue;
        }
        match base.get(d.as_str()) {
            // A `fail` baseline is a deliberate tracked known-fail — visible, not redding.
            Some(Verdict::Fail) => out.tracked_fail.push(d.clone()),
            // A `pass` baseline that now fails is already a regression (caught above).
            Some(Verdict::Pass) => {}
            // A `todo` baseline or an absent case that now fails reds — the gate-hole guard.
            _ => out.failing.push(d.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the whole-pass baseline-regression fold (compare_verdicts_baseline) — the 7 canonical invariant
    // tests, lifted verbatim from xtask/src/gate_syntax.rs (v-corpus-harness blessed them as the complete
    // behavior spec; both `gate --check` and `gate-syntax --check` now delegate to the fold under test).
    fn v(pairs: &[(&str, Verdict)]) -> Vec<(String, Verdict)> {
        pairs.iter().map(|(d, v)| (d.to_string(), *v)).collect()
    }

    #[test]
    fn pass_to_not_pass_is_the_only_regression() {
        // pass→fail and pass→todo REGRESS; todo→pass GAINS; a steady pass/todo is quiet.
        let baseline = "pass\ta\npass\tb\ntodo\tc\ntodo\td\n";
        let now = v(&[
            ("a", Verdict::Fail), // pass→fail: regressed
            ("b", Verdict::Todo), // pass→todo: regressed
            ("c", Verdict::Pass), // todo→pass: gained (additive)
            ("d", Verdict::Todo), // steady todo: quiet
        ]);
        let cmp = compare_verdicts_baseline(&now, baseline, false);
        assert_eq!(
            cmp.regressed.len(),
            2,
            "pass→fail and pass→todo both regress"
        );
        assert_eq!(cmp.gained, vec!["c".to_string()], "todo→pass is a gain");
        // `b` is a pass→todo regression, NOT a `failing` (it is not a Fail verdict).
        assert!(cmp.failing.is_empty());
        assert_eq!(cmp.exit_code(), 1);
    }

    #[test]
    fn failing_hole_guard_reds_a_todo_or_absent_case_that_now_fails() {
        // The v-nix gate-hole: a todo-baselined or ABSENT case that now FAILs must red even though it is
        // not a pass→not-pass regression.
        let baseline = "todo\ta\n"; // `b` is absent from the baseline (a fresh case)
        let now = v(&[("a", Verdict::Fail), ("b", Verdict::Fail)]);
        let cmp = compare_verdicts_baseline(&now, baseline, false);
        assert!(cmp.regressed.is_empty(), "neither was a baseline pass");
        assert_eq!(
            cmp.failing.len(),
            2,
            "a todo→fail AND an absent→fail both red (the gate-hole guard)"
        );
        assert_eq!(cmp.exit_code(), 1);
    }

    #[test]
    fn tracked_known_fail_is_visible_but_not_redding() {
        // A `fail` verdict against an explicit `fail` baseline is a deliberate pin — reported, not red.
        let baseline = "fail\ta\npass\tb\n";
        let now = v(&[("a", Verdict::Fail), ("b", Verdict::Pass)]);
        let cmp = compare_verdicts_baseline(&now, baseline, false);
        assert_eq!(cmp.tracked_fail, vec!["a".to_string()]);
        assert!(
            cmp.failing.is_empty(),
            "a fail+fail-baseline is NOT the gate-hole failing set"
        );
        assert!(cmp.regressed.is_empty());
        assert_eq!(
            cmp.exit_code(),
            0,
            "a tracked known-fail does not red the gate"
        );
    }

    #[test]
    fn vanished_reds_only_on_a_full_run() {
        // A baseline title with no current case is vanished on a FULL run, ignored on a SUBSET run.
        let baseline = "pass\ta\npass\tb\n";
        let now = v(&[("a", Verdict::Pass)]); // `b` not run
        assert_eq!(
            compare_verdicts_baseline(&now, baseline, false).vanished,
            vec!["b".to_string()],
            "full run flags the vanished case"
        );
        assert_eq!(
            compare_verdicts_baseline(&now, baseline, false).exit_code(),
            1
        );
        assert!(
            compare_verdicts_baseline(&now, baseline, true)
                .vanished
                .is_empty(),
            "subset run skips the vanished check"
        );
        assert_eq!(
            compare_verdicts_baseline(&now, baseline, true).exit_code(),
            0
        );
    }

    #[test]
    fn benign_dup_is_harmless_but_conflicting_dup_is_a_hard_error() {
        let now = v(&[("a", Verdict::Pass)]);
        // Benign: the same title+verdict twice (a merge=union artifact) — counted, not fatal.
        let benign = "pass\ta\npass\ta\n";
        let cmp = compare_verdicts_baseline(&now, benign, false);
        assert_eq!(cmp.benign_dups, 1);
        assert!(cmp.conflict.is_empty());
        assert_eq!(cmp.exit_code(), 0, "a benign dup does not red");
        // Conflicting: the same title with DIFFERENT verdicts — a hard integrity error (exit 3).
        let conflicting = "pass\ta\ntodo\ta\n";
        let cmp = compare_verdicts_baseline(&now, conflicting, false);
        assert_eq!(cmp.conflict, vec!["a".to_string()]);
        assert_eq!(cmp.exit_code(), 3, "a conflicting dup is exit 3");
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let baseline = "# header\n\npass\ta\n";
        let cmp = compare_verdicts_baseline(&v(&[("a", Verdict::Pass)]), baseline, false);
        assert_eq!(cmp.exit_code(), 0);
        assert_eq!(cmp.benign_dups, 0);
    }

    #[test]
    fn parse_verdicts_reads_the_harvested_format_and_skips_noise() {
        // The `--compare` aggregate entry parses `<verdict>\t<title>` lines (the concatenated per-case
        // nix verdicts), skipping `#`/blank/garbage lines — the same vocabulary as the baseline file.
        let text = "# header\n\npass\tsexp/01\ttrailing-ignored?\ntodo\tsexp/17\nfail\tml/03\ngarbage line\nnope\tx\n";
        // `split_once('\t')` keeps everything after the FIRST tab as the title (so a stray tab stays in
        // the title, matching the baseline's own map-load); the garbage line (no tab) and the unknown
        // verdict tag `nope` are dropped. (Verdict isn't Debug, so compare via its tag.)
        let got: Vec<(String, &str)> = parse_verdicts(text)
            .into_iter()
            .map(|(d, v)| (d, v.tag()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("sexp/01\ttrailing-ignored?".to_string(), "pass"),
                ("sexp/17".to_string(), "todo"),
                ("ml/03".to_string(), "fail"),
            ]
        );
    }

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

    /// `parse_records` is the corpus parser every command crate (roundtrip, prune-baselines) reads through;
    /// a silent regression drops or mis-pairs cases. Pin the tricky bits: `---` record boundaries, the
    /// two-value `module`/`peer`/`host-response` splits, the `expect`-closes-a-trial pairing of a pending
    /// `call`+`arg`s, the `(message …)` warn clause, and the per-call `live-objects known-leak` first-count.
    #[test]
    fn parse_records_pins_the_corpus_grammar() {
        let text = "\
case\tadds two\n\
program\t(fn add)\n\
module\thelper\t(lib prog)\n\
peer\twasi:io\t(peer prog)\n\
needs\tio\n\
host-response\tget\t42\n\
host-call\tget\n\
warns\tCDZ0201 (message \"deprecated\")\n\
wit-world\t(world w)\n\
component-name\tapp:main\n\
live-objects\tknown-leak\t3\t13\t0\n\
call\tadd\n\
arg\t1\n\
arg\t2\n\
expect\t3\n\
---\n\
case\tmethod drive\n\
program\t(fn next)\n\
call-method\tnext\n\
then-call\t0\n\
drop-handle\t1\n\
expect\tdone\n\
---\n";
        let recs = parse_records(text);
        assert_eq!(recs.len(), 2, "two `---`-terminated records");

        let r = &recs[0];
        assert_eq!(r.description, "adds two");
        assert_eq!(r.program, "(fn add)");
        assert_eq!(r.modules, vec![("helper".into(), "(lib prog)".into())]);
        assert_eq!(r.peers, vec![("wasi:io".into(), "(peer prog)".into())]);
        assert_eq!(r.needs, vec!["io".to_string()]);
        assert_eq!(r.host_responses, vec![("get".into(), "42".into())]);
        assert_eq!(r.host_calls, vec!["get".to_string()]);
        assert_eq!(r.warns, vec![("CDZ0201".into(), Some("deprecated".into()))]);
        assert_eq!(r.wit_world.as_deref(), Some("(world w)"));
        assert_eq!(r.component_name.as_deref(), Some("app:main"));
        // per-call `known-leak\t3\t13\t0` → this direct path takes the FIRST count.
        assert_eq!(r.live_objects, Some(3));
        assert_eq!(r.trials.len(), 1);
        let call = r.trials[0].call.as_ref().expect("the trial has a call");
        assert_eq!(r.trials[0].expect, "3");
        assert_eq!(call.export, "add");
        assert_eq!(call.args, vec!["1".to_string(), "2".to_string()]);
        assert_eq!(call.second_call, None);
        assert!(!call.drop_handle);
        assert_eq!(call.method, None);

        // A `(call-method)` case has NO export but still produces a Call (from `method` alone), carries a
        // nullary `then` second-call (`Some(vec![])`, distinct from `None`), and the `(drop)` flag.
        let m = &recs[1];
        assert_eq!(m.description, "method drive");
        let mc = m.trials[0].call.as_ref().expect("method drive has a call");
        assert_eq!(m.trials[0].expect, "done");
        assert_eq!(mc.export, "");
        assert!(mc.args.is_empty());
        assert_eq!(mc.method.as_deref(), Some("next"));
        assert_eq!(mc.second_call, Some(vec![]));
        assert!(mc.drop_handle);
    }

    /// The shared `(message …)` clause splitter (error/declines/warns diagnostic-text pins). Pin the head
    /// vs optional phrase, the empty-head `declines` form, and that a phrase with no opening quote is not
    /// asserted (`None`) rather than mis-parsed.
    #[test]
    fn split_message_clause_separates_code_from_optional_phrase() {
        assert_eq!(split_message_clause("CDZ0201"), ("CDZ0201", None));
        assert_eq!(
            split_message_clause("CDZ0201 (message \"malformed record\")"),
            ("CDZ0201", Some("malformed record"))
        );
        assert_eq!(
            split_message_clause("(message \"IEEE partial order\")"),
            ("", Some("IEEE partial order"))
        );
        // no opening quote → the clause is simply not asserted (never a bogus phrase).
        assert_eq!(split_message_clause("(message noquote)"), ("", None));
    }

    /// `first_line` is used to surface a launched tool's first stderr line; pin the empty and
    /// single-line (no trailing newline) edges so an empty slice can't panic.
    #[test]
    fn first_line_handles_empty_and_unterminated() {
        assert_eq!(first_line(b"hello\nworld"), "hello");
        assert_eq!(first_line(b"single"), "single");
        assert_eq!(first_line(b""), "");
    }

    /// Build a unique temp `<repo>/implementation/` fixture and write `name` with `bytes` bytes.
    /// Returns the [`TmpDir`]; hold it for the test's duration (`Drop` reaps the dir, even on panic).
    fn size_fixture(tag: &str, name: &str, bytes: usize) -> TmpDir {
        let fx = TmpDir::new(&format!("cdz-filesize-{tag}-"));
        fx.write(&format!("implementation/{name}"), vec![b'x'; bytes]);
        fx
    }

    #[test]
    fn file_size_lint_flags_oversized_non_allowlisted() {
        let fx = size_fixture("over", "huge.rs", (MAX_SOURCE_BYTES + 1) as usize);
        let err = file_size_lint_with(fx.dir(), &[]).unwrap_err();
        assert!(err.contains("huge.rs"), "names the offender: {err}");
        assert!(err.contains("split"), "tells you to split it: {err}");
    }

    #[test]
    fn file_size_lint_passes_under_limit_and_allowlisted() {
        // A small file passes; a huge one ON the allowlist passes (grandfathered, no stale entry).
        let fx = size_fixture("ok", "small.rs", 10);
        assert!(file_size_lint_with(fx.dir(), &[]).is_ok());

        let fx = size_fixture("grand", "huge.rs", (MAX_SOURCE_BYTES + 1) as usize);
        assert!(file_size_lint_with(fx.dir(), &["implementation/huge.rs"]).is_ok());
    }

    #[test]
    fn file_size_lint_flags_stale_allowlist_entry() {
        // An allowlisted file that is UNDER the limit (or missing) is a stale entry → must be removed.
        let fx = size_fixture("stale", "small.rs", 10);
        let err = file_size_lint_with(fx.dir(), &["implementation/small.rs"]).unwrap_err();
        assert!(
            err.contains("STALE"),
            "flags the stale allowlist entry: {err}"
        );
        assert!(err.contains("small.rs"), "names the stale entry: {err}");
    }

    /// Build a two-crate fixture: `<repo>/implementation/crates/{a,b}/` each with a `Cargo.toml` + `src/`.
    /// Crate A's `src/lib.rs` gets `a_lib_body`; crate B gets a `src/shared.rs`. Returns the [`TmpDir`].
    fn path_fixture(tag: &str, a_lib_body: &str) -> TmpDir {
        let fx = TmpDir::new(&format!("cdz-pathlint-{tag}-"));
        for c in ["a", "b"] {
            fx.write(
                &format!("implementation/crates/{c}/Cargo.toml"),
                "[package]\n",
            );
        }
        fx.write("implementation/crates/b/src/shared.rs", "pub fn f() {}\n");
        fx.write("implementation/crates/a/src/lib.rs", a_lib_body);
        fx
    }

    #[test]
    fn path_lint_flags_cross_crate_include() {
        // A's lib.rs source-includes B's file → cross-crate, must fail.
        let fx = path_fixture(
            "cross",
            "#[path = \"../../b/src/shared.rs\"]\nmod shared;\n",
        );
        let err = cross_crate_path_include_lint_with(fx.dir(), &[]).unwrap_err();
        assert!(err.contains("cross-crate"), "names the violation: {err}");
        assert!(err.contains("shared.rs"), "names the target: {err}");
    }

    #[test]
    fn path_lint_allows_same_crate_include_and_allowlisted() {
        // Same-crate #[path] (a file under A's own src/) is fine.
        let fx = path_fixture("same", "#[path = \"helper.rs\"]\nmod helper;\n");
        fx.write("implementation/crates/a/src/helper.rs", "\n");
        assert!(cross_crate_path_include_lint_with(fx.dir(), &[]).is_ok());

        // A grandfathered cross-crate include passes (and is not stale, since it is present).
        let fx = path_fixture(
            "allow",
            "#[path = \"../../b/src/shared.rs\"]\nmod shared;\n",
        );
        let allow = &[(
            "implementation/crates/a/src/lib.rs",
            "../../b/src/shared.rs",
        )];
        assert!(cross_crate_path_include_lint_with(fx.dir(), allow).is_ok());
    }

    #[test]
    fn path_lint_flags_stale_allowlist_entry() {
        // The allowlist names an include that no longer exists → stale, must be removed.
        let fx = path_fixture("pstale", "// no includes here\n");
        let allow = &[(
            "implementation/crates/a/src/lib.rs",
            "../../b/src/shared.rs",
        )];
        let err = cross_crate_path_include_lint_with(fx.dir(), allow).unwrap_err();
        assert!(
            err.contains("STALE"),
            "flags the stale allowlist entry: {err}"
        );
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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

//! The `cdz-corpus` command surface, as an EMBEDDABLE clap `Args` group + a `run` entry point.
//!
//! Factored out of the standalone `bin/cdz-corpus.rs` so the unified `cdz` binary can MOUNT it as
//! `cdz corpus records …` (the same flatten pattern `cdz` uses for the syntax/compiler
//! CLIs) WITHOUT a second binary on the PATH. The standalone `cdz-corpus` bin is now a thin shim over
//! [`run`]; xtask (which shells out to the standalone bin) is unaffected. Both entry points share one
//! implementation and one `--help`. `run` takes the already-parsed [`CorpusArgs`] and returns an
//! `ExitCode`, threading a `prog` name so a diagnostic points at the command the user actually typed.

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use cadenza_syntax::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_syntax::codec;
use cadenza_syntax::sexpr;

use crate::{Expect, Record};

/// The arguments to `cdz corpus` / `cdz-corpus` — read the executable-semantics corpus.
#[derive(clap::Args)]
pub struct CorpusArgs {
    #[command(subcommand)]
    command: CorpusCmd,
}

#[derive(clap::Subcommand)]
enum CorpusCmd {
    /// Parse corpus files and emit one normalized record per case.
    ///
    /// Default: the whole record stream (all cases, `---`-separated) to stdout. With `--out-dir DIR`:
    /// SHRED instead — for each input file write one directory per case under `DIR/<stem>/<NNNN>-<slug>/`
    /// holding the case's artifacts, each already in its CONSUMER's native form (`program.ast`,
    /// `module-*.ast`?, `wit-world.ast`?, `component-name`?, `test-run.ast`), plus a `DIR/<stem>/manifest`
    /// listing the case dirs in order. These are the per-case units the nix corpus pipeline caches on; see
    /// `design/DESIGN-corpus-nix-per-case-caching.md`.
    Records {
        /// Corpus `.sexp` files to read.
        #[arg(required = true)]
        files: Vec<String>,
        /// Shred each file into one per-case dir of binary-AST artifacts under `DIR/<stem>/` (+ a
        /// `manifest`), instead of writing the concatenated record stream to stdout.
        #[arg(long)]
        out_dir: Option<String>,
    },
    /// Check a committed `.gate-baseline` for TITLE DRIFT against the corpus — FAST, no compile/run.
    ///
    /// A baseline line is `verdict\tdescription`; a corpus case's description is its `(case "…")` /
    /// `(platform-case "…")` title. When a case is RENAMED or REMOVED but the baseline is not
    /// regenerated, the stale baseline title matches no case = a "VANISHED" entry, which reds a FULL
    /// `cargo xtask gate --check` / `gate-local` fleet-wide — but only after a ~15-30min gate. This lint
    /// catches that drift INSTANTLY by set-diffing the two title sets (no compiler, no runtime). It exits
    /// NON-ZERO on any VANISHED title (the red-causing drift) and only WARNS on MISSING titles (a
    /// case present in the corpus but absent from the baseline — a new/renamed case that a
    /// `cargo xtask gate --save` should record; not itself a `--check` red unless the case also fails).
    BaselineDrift {
        /// Corpus `.sexp` files whose case titles form the CURRENT set. Pass the COMPLETE set the
        /// baseline covers (the full `spec/semantics/*.sexp` glob) — a SUBSET makes every other file's
        /// baselined case look VANISHED (its title is genuinely absent from the files given), a false red.
        #[arg(required = true)]
        files: Vec<String>,
        /// The committed baseline file to check (e.g. `spec/semantics/.gate-baseline`).
        #[arg(long)]
        baseline: String,
        /// Also list every MISSING title (corpus case absent from the baseline). Off by default —
        /// missing titles are only WARNINGS (a `gate --save` records them), summarized as a count so a
        /// wired-in guard stays quiet; the full list can be long when the baseline is stale.
        #[arg(long)]
        list_missing: bool,
    },
}

/// Run a corpus command per `args`, returning the process exit code. `prog` names the tool in
/// diagnostics (`cdz-corpus` for the standalone bin, `cdz` for the unified one).
pub fn run(args: &CorpusArgs, prog: &str) -> ExitCode {
    let result = match &args.command {
        CorpusCmd::Records { files, out_dir } => match out_dir {
            Some(dir) => shred_records(files, dir),
            None => run_records(files),
        },
        CorpusCmd::BaselineDrift {
            files,
            baseline,
            list_missing,
        } => check_baseline_drift(files, baseline, *list_missing),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{prog}: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// `records FILE…`: read each corpus `.sexp` file, normalize its cases, and emit the flat record stream
/// to stdout (records from all files concatenated, in file then case order). A `(platform-case …)` file
/// emits the platform record stream; any other file emits the compiler-case stream. The genre is
/// auto-detected by the leading form's head — the two genres are disjoint (a file is one or the other).
fn run_records(files: &[String]) -> Result<(), String> {
    let mut out = String::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        if crate::is_platform_genre(&text) {
            let records = crate::read_platform(&text).map_err(|e| format!("{path}: {e}"))?;
            out.push_str(&crate::render_platform(&records));
        } else {
            let records = crate::read(&text).map_err(|e| format!("{path}: {e}"))?;
            out.push_str(&crate::render(&records));
        }
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| format!("writing stdout: {e}"))?;
    Ok(())
}

/// `baseline-drift --baseline B FILE…`: the fast title-drift GUARD. Collect the current corpus case
/// titles from `files`, read the committed baseline `B`, and report the two set-differences. Exits
/// NON-ZERO (via `Err`) iff any VANISHED title is found (a baseline entry with no matching case — the
/// exact drift that reds a full `gate --check`); MISSING titles (a corpus case absent from the baseline)
/// are printed as WARNINGS only (a `gate --save` records them; not a `--check` red on their own).
fn check_baseline_drift(
    files: &[String],
    baseline: &str,
    list_missing: bool,
) -> Result<(), String> {
    let mut corpus: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        corpus.extend(corpus_descriptions(&text).map_err(|e| format!("{path}: {e}"))?);
    }
    let bl_text =
        std::fs::read_to_string(baseline).map_err(|e| format!("reading {baseline}: {e}"))?;
    let baseline_descs = baseline_descriptions(&bl_text);
    let (vanished, missing) = baseline_drift(&corpus, &baseline_descs);

    // MISSING titles are warnings only (a `gate --save` records them) — summarized by count so a wired-in
    // guard stays quiet, with the full list gated behind `--list-missing`.
    if list_missing {
        for m in &missing {
            eprintln!(
                "baseline-drift: WARN missing from baseline (run `cargo xtask gate --save`): {m:?}"
            );
        }
    }
    for v in &vanished {
        eprintln!(
            "baseline-drift: VANISHED baseline title has no corpus case (reds `gate --check`): {v:?}"
        );
    }
    let missing_hint = if !missing.is_empty() && !list_missing {
        " (pass --list-missing to list them; `cargo xtask gate --save` records them)"
    } else {
        ""
    };
    if vanished.is_empty() {
        println!(
            "baseline-drift: OK — no vanished titles in {baseline} ({} corpus titles, {} baseline titles, {} missing-from-baseline{missing_hint})",
            corpus.len(),
            baseline_descs.len(),
            missing.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} VANISHED baseline title(s) in {baseline} — a renamed/removed case left the baseline stale; regenerate with `cargo xtask gate --save`",
            vanished.len()
        ))
    }
}

/// The case titles in one corpus file's text, genre-dispatched: a compiler-genre file's titles are its
/// `(case "…")` descriptions; a platform-genre file's are its `(platform-case "…")` titles. These are the
/// exact strings the shred writes as each case's `(description …)` and the gate records in the baseline.
fn corpus_descriptions(text: &str) -> Result<Vec<String>, String> {
    if crate::is_platform_genre(text) {
        Ok(crate::read_platform(text)?
            .into_iter()
            .map(|r| r.title)
            .collect())
    } else {
        Ok(crate::read(text)?
            .into_iter()
            .map(|r| r.description)
            .collect())
    }
}

/// The descriptions in a `.gate-baseline` file — the `description` half of each `verdict\tdescription`
/// line (`#`-comment and blank lines skipped), matching `cdz-corpus-grade::baseline_verdict`'s line shape.
fn baseline_descriptions(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_, d)| d.to_string()))
        .collect()
}

/// Pure title-set diff: `(vanished, missing)` where VANISHED = a baseline description absent from the
/// corpus set (the red-causing drift) and MISSING = a corpus title absent from the baseline. Both sorted
/// + de-duplicated (the corpus arrives as a `BTreeSet`; a baseline dup collapses). Pure so it is unit-testable.
fn baseline_drift(
    corpus: &std::collections::BTreeSet<String>,
    baseline_descs: &[String],
) -> (Vec<String>, Vec<String>) {
    let baseline: std::collections::BTreeSet<&str> =
        baseline_descs.iter().map(|s| s.as_str()).collect();
    let vanished: Vec<String> = baseline
        .iter()
        .filter(|d| !corpus.contains(**d))
        .map(|d| d.to_string())
        .collect();
    let missing: Vec<String> = corpus
        .iter()
        .filter(|d| !baseline.contains(d.as_str()))
        .cloned()
        .collect();
    (vanished, missing)
}

/// `records --out-dir DIR FILE…`: SHRED each corpus file into one directory per case under
/// `DIR/<stem>/<NNNN>-<slug>/`, each holding the case's artifacts already in their CONSUMER's native form
/// (`design/DESIGN-corpus-nix-per-case-caching.md`), plus a `DIR/<stem>/manifest` listing the case dirs
/// in order. The artifacts:
///
/// - `program.ast` — the normalized program as binary AST, fed straight to the compiler (`ast:main=…`).
/// - `module-<name>.ast` — one per sibling LIBRARY module (multi-file package cases), also a compiler
///   input (`ast:<name>=…`; the entry `(import "name")`s it).
/// - `wit-world.ast` — the imposed world as binary AST (the `(world …)` subtree), the compiler's native
///   `wit-world:<name>=…` input verbatim; omitted for the common synthesized-world case.
/// - `component-name` — the interface the world's guest exports under, as PLAIN TEXT (a `--component-name`
///   string, not an AST); omitted unless the case names one.
/// - `test-run.ast` — description + trials (call/args/expect) + host-calls/responses + warns, the
///   run/grade metadata consumed by the runner (exec), not the compiler.
///
/// The shred does every format transform ONCE here (extract the world subtree, encode each program), so
/// each artifact hands to its consumer with NO further conversion — the whole point per the operator's
/// "fewer transforms" steer. Splitting by consumer is the caching win: the build derivation keys on
/// {program+modules, wit-world, component-name} so a run-metadata edit (expected output, args, host tape)
/// never rebuilds; the exec derivation keys on {artifact, test-run} so it is compiler-independent.
///
/// Compiler-genre only: platform-genre files (`spec/platform`) are not part of this pipeline and are a
/// hard error under `--out-dir`.
fn shred_records(files: &[String], out_dir: &str) -> Result<(), String> {
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        if crate::is_platform_genre(&text) {
            return Err(format!(
                "{path}: --out-dir shreds the compiler corpus only; platform-genre files are out of scope"
            ));
        }
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("no file stem for {path}"))?;
        let dir = std::path::Path::new(out_dir).join(stem);
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let records = crate::read(&text).map_err(|e| format!("{path}: {e}"))?;
        let mut manifest = String::new();
        for (i, rec) in records.iter().enumerate() {
            let case = format!("{i:04}-{}", slug(&rec.description));
            let cdir = dir.join(&case);
            std::fs::create_dir_all(&cdir)
                .map_err(|e| format!("creating {}: {e}", cdir.display()))?;
            // program + modules + compile-unit are binary AST the reader ALREADY built (arena-direct, no
            // reparse) — write the bytes verbatim. test-run is built here (arena-direct via Builder).
            write_bytes(&cdir.join("program.ast"), &rec.program_ast)
                .map_err(|e| format!("{path} case {i} program: {e}"))?;
            for m in &rec.modules {
                write_bytes(
                    &cdir.join(format!("module-{}.ast", slug(&m.name))),
                    &m.program_ast,
                )
                .map_err(|e| format!("{path} case {i} module {}: {e}", m.name))?;
            }
            // PEER provider components (cross-component case): per peer, its program as `peer-<n>.ast` (a
            // STANDALONE component the nix build-drv compiles separately — glob `peer-*.ast`, NOT linked
            // like a module) + a `peer-<n>.iface` sidecar holding the interface name. The interface
            // (`cadenza:pkg/iface`) is NOT filename-safe (`:`/`/`), so the sidecar — not the stem — carries
            // it, and the build/exec reconstruct `--peer <iface>=<peer.wasm>` from {peer-<n>.ast,
            // peer-<n>.iface}. Indexed by declaration order (stable). Absent for a single-component case.
            for (n, p) in rec.peers.iter().enumerate() {
                write_bytes(&cdir.join(format!("peer-{n}.ast")), &p.program_ast)
                    .map_err(|e| format!("{path} case {i} peer {} ast: {e}", p.interface))?;
                std::fs::write(cdir.join(format!("peer-{n}.iface")), &p.interface)
                    .map_err(|e| format!("{path} case {i} peer {} iface: {e}", p.interface))?;
            }
            // The compile CONFIG, each in the compiler's NATIVE input form (no transform at compile time):
            // the world as its own `wit-world.ast` binary artifact, the interface name as plain text.
            if let Some(w) = &rec.wit_world_ast {
                write_bytes(&cdir.join("wit-world.ast"), w)
                    .map_err(|e| format!("{path} case {i} wit-world: {e}"))?;
            }
            if let Some(cn) = &rec.component_name {
                std::fs::write(cdir.join("component-name"), cn)
                    .map_err(|e| format!("{path} case {i} component-name: {e}"))?;
            }
            write_bytes(&cdir.join("test-run.ast"), &test_run_ast(rec))
                .map_err(|e| format!("{path} case {i} test-run: {e}"))?;
            // The case's primary outcome KIND as PLAIN TEXT — the one datum the nix build/exec ROUTER needs
            // to decide compiler-refusal (error/declines, build-phase) vs a run (output/trap, exec-phase),
            // so the router reads a bare word instead of decoding the binary `test-run.ast` (keeping the
            // compiler-free exec derivation from having to link a decoder). Derived from the first trial.
            std::fs::write(cdir.join("expect-kind"), expect_kind(rec))
                .map_err(|e| format!("{path} case {i} expect-kind: {e}"))?;
            // The ORACLE-TRIAL artifact — the same trials as `test-run.ast`, but with each VALUE (arg,
            // expected output, host-response) parsed to BINARY AST (not an opaque string leaf), so the
            // Lean differential oracle reads binary AST and never re-parses s-expr text. Additive: a
            // SIBLING file, so `test-run.ast` stays byte-identical (cdz-run --grade + the corpus gate
            // untouched); only the oracle-check consumer reads it. (Operator: emitted by the normal
            // shred, not a separate command.)
            write_bytes(&cdir.join("oracle-trial.ast"), &oracle_trials_ast(rec))
                .map_err(|e| format!("{path} case {i} oracle-trial: {e}"))?;
            manifest.push_str(&case);
            manifest.push('\n');
        }
        std::fs::write(dir.join("manifest"), &manifest)
            .map_err(|e| format!("writing {}/manifest: {e}", dir.display()))?;
    }
    Ok(())
}

/// The ORACLE-TRIAL artifact — a case's trials as BINARY AST for the Lean oracle. Unlike `test_run_ast`
/// (which stores each value as an opaque string LEAF for a text-reparsing runner), each trial VALUE
/// (arg, expected output, host-response) is PARSED from its value-form text into its binary-AST subtree
/// (`sexpr::read`, grafted) so the oracle reads values as binary AST and never re-parses s-expr text.
/// The expected outcome is carried too (the oracle asserts it internally). Shape:
///   (oracle-trials (trials (trial (call <export>)? (arg <value-ast>)*
///       (expect-value <value-ast> | expect-trap <reason> | expect-error <code> | expect-declines)) …)
///     (host-responses (response <op> <value-ast>) …)? )
fn oracle_trials_ast(rec: &Record) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("oracle-trials");
    let mut kids = vec![head];

    let trials_head = b.name("trials");
    let mut trials = vec![trials_head];
    for t in &rec.trials {
        let mut tk = vec![b.name("trial")];
        if let Some(c) = &t.call {
            let ex = str_leaf(&mut b, &c.export);
            tk.push(form(&mut b, "call", vec![ex]));
            for a in &c.args {
                let av = parse_value_ast(&mut b, a);
                tk.push(form(&mut b, "arg", vec![av]));
            }
        }
        let e = match &t.expect {
            Expect::Output(v) => {
                let av = parse_value_ast(&mut b, v);
                form(&mut b, "expect-value", vec![av])
            }
            Expect::Trap(reason) => {
                let r = str_leaf(&mut b, reason);
                form(&mut b, "expect-trap", vec![r])
            }
            Expect::Error(code, _) => {
                let cl = str_leaf(&mut b, code);
                form(&mut b, "expect-error", vec![cl])
            }
            Expect::Declines(_) => form(&mut b, "expect-declines", vec![]),
        };
        tk.push(e);
        trials.push(b.list(tk));
    }
    kids.push(b.list(trials));

    if !rec.host_responses.is_empty() {
        let mut hk = vec![b.name("host-responses")];
        for (op, v) in &rec.host_responses {
            let ol = str_leaf(&mut b, op);
            let vv = parse_value_ast(&mut b, v);
            hk.push(form(&mut b, "response", vec![ol, vv]));
        }
        kids.push(b.list(hk));
    }

    let root = b.list(kids);
    codec::encode(&b.finish(root))
}

/// Parse a value-form text (e.g. `41`, `(: 5 Int64)`, `(Some 5)`) into its binary-AST subtree, grafted
/// into `b`. On a parse failure (a value-form the s-expr reader can't take) FALL BACK to an opaque
/// string leaf, so the artifact still emits and the oracle marks that trial `Unsupported` rather than
/// the whole file's derivation failing.
fn parse_value_ast(b: &mut Builder, text: &str) -> StructId {
    match sexpr::read(text) {
        Ok(arena) => graft_value(b, &arena, arena.root),
        Err(_) => str_leaf(b, text),
    }
}

/// Copy the subtree rooted at `id` of `src` INTO builder `b`, returning its new root. Iterative
/// post-order (explicit stack) so a deep value can't overflow the native stack; leaves interned by
/// value. Mirrors `cadenza_syntax::doc_item::graft_subtree`.
fn graft_value(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    enum Job {
        Visit(StructId),
        EmitList(usize),
    }
    let mut jobs = vec![Job::Visit(id)];
    let mut results: Vec<StructId> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match src.get(sid) {
                cadenza_syntax::ast::Struct::Atom(lid) => {
                    let leaf = src.leaf(*lid).clone();
                    results.push(b.atom_leaf(leaf));
                }
                cadenza_syntax::ast::Struct::List(kids) => {
                    jobs.push(Job::EmitList(kids.len()));
                    for &k in kids.iter().rev() {
                        jobs.push(Job::Visit(k));
                    }
                }
            },
            Job::EmitList(n) => {
                let kids = results.split_off(results.len() - n);
                results.push(b.list(kids));
            }
        }
    }
    results.pop().expect("graft_value leaves a root")
}

/// Write a per-case binary-AST artifact (bytes the reader already built, or `test_run_ast` here) to disk.
fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// A case's primary outcome KIND — `output` | `trap` | `error` | `declines` — from its FIRST trial (a
/// case is one outcome kind; multi-trial cases repeat `output`). This is the build/exec ROUTER's cue:
/// `output`/`trap` are RUN outcomes (compile must succeed → graded at exec), `error`/`declines` are
/// COMPILE outcomes (the compiler must refuse → graded at build).
fn expect_kind(rec: &Record) -> &'static str {
    match rec.trials.first().map(|t| &t.expect) {
        Some(Expect::Output(_)) => "output",
        Some(Expect::Trap(_)) => "trap",
        Some(Expect::Error(..)) => "error",
        Some(Expect::Declines(_)) => "declines",
        None => "output", // a case always has ≥1 trial; default is harmless
    }
}

/// The TEST-RUN artifact — a case's run/grade metadata (description, trials, host tape, warns) as BINARY
/// AST, built arena-direct via `Builder` (no sexpr-text round-trip). Every text field is a string LEAF
/// (value-forms, codes, reasons, messages, op/export names), so an awkward character can never break the
/// tree; the runner reads each leaf as opaque text and re-parses value-forms itself when comparing.
/// Mirrors `render`'s field walk. Consumed by the exec phase, never the compiler.
fn test_run_ast(rec: &Record) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("test-run");
    let desc_leaf = str_leaf(&mut b, &rec.description);
    let desc = form(&mut b, "description", vec![desc_leaf]);
    let mut kids = vec![head, desc];

    let trials_head = b.name("trials");
    let mut trials = vec![trials_head];
    for t in &rec.trials {
        let mut tk = vec![b.name("trial")];
        if let Some(c) = &t.call {
            // A `(call-method <member>)` case has no export — emit a `(call-method <member>)` node the grade
            // path reaches on the value-resource (mirrors the direct-gate `--call-member`); otherwise the
            // ordinary `(call <export>)`.
            if let Some(member) = &c.method {
                let ml = str_leaf(&mut b, member);
                tk.push(form(&mut b, "call-method", vec![ml]));
            } else {
                let ex = str_leaf(&mut b, &c.export);
                tk.push(form(&mut b, "call", vec![ex]));
            }
            for a in &c.args {
                let al = str_leaf(&mut b, a);
                tk.push(form(&mut b, "arg", vec![al]));
            }
            // A `(then …)` two-call continuation: a `(then-call)` marker (present even for a nullary
            // second call) plus one `(then-arg <v>)` per second-call argument — so the grade path drives
            // the SAME closure handle twice (mirrors the direct-gate `--call-twice`/`--then-arg`).
            if let Some(second) = &c.second_call {
                tk.push(form(&mut b, "then-call", vec![]));
                for a in second {
                    let al = str_leaf(&mut b, a);
                    tk.push(form(&mut b, "then-arg", vec![al]));
                }
            }
            // A `(drop)` clause: a `(drop-handle)` marker so the grade path resource-drops the handle
            // before reading the heap balance (mirrors the direct-gate `--drop-handle`).
            if c.drop_handle {
                tk.push(form(&mut b, "drop-handle", vec![]));
            }
        }
        let e = expect_form(&mut b, &t.expect);
        tk.push(e);
        trials.push(b.list(tk));
    }
    kids.push(b.list(trials));

    if !rec.host_responses.is_empty() {
        let mut hk = vec![b.name("host-responses")];
        for (op, v) in &rec.host_responses {
            let ol = str_leaf(&mut b, op);
            let vl = str_leaf(&mut b, v);
            hk.push(form(&mut b, "response", vec![ol, vl]));
        }
        kids.push(b.list(hk));
    }
    if !rec.host_calls.is_empty() {
        let mut hk = vec![b.name("host-calls")];
        for op in &rec.host_calls {
            let ol = str_leaf(&mut b, op);
            hk.push(form(&mut b, "op", vec![ol]));
        }
        kids.push(b.list(hk));
    }
    if !rec.warns.is_empty() {
        let mut wk = vec![b.name("warns")];
        for (code, msg) in &rec.warns {
            let cl = str_leaf(&mut b, code);
            let mut leaves = vec![cl];
            if let Some(m) = msg {
                leaves.push(str_leaf(&mut b, m));
            }
            wk.push(form(&mut b, "warn", leaves));
        }
        kids.push(b.list(wk));
    }
    // `(live-objects <N>)` — the post-run heap-balance the case asserts (N as a string leaf). A
    // `(live-objects known-leak <N>)` marker prefixes the count with the literal `known-leak` (both graded
    // as assert == N; the marker records the opt-out intent). Under the opt-out default the nix exec runs
    // EVERY heap-importing case on `--runtime <debug-counters>` and cdz-run's `--grade` asserts == N
    // (heap case) / == 0 (absent + heap) / skips (no-heap). Absent for a case with no `(live-objects …)`.
    if let Some(n) = rec.live_objects {
        let mut leaves = Vec::new();
        if rec.live_objects_known_leak {
            leaves.push(str_leaf(&mut b, "known-leak"));
        }
        leaves.push(str_leaf(&mut b, &n.to_string()));
        kids.push(form(&mut b, "live-objects", leaves));
    }

    let root = b.list(kids);
    codec::encode(&b.finish(root))
}

/// One trial's expected outcome as a tagged form — the four `Expect` variants, fields as string leaves.
fn expect_form(b: &mut Builder, e: &Expect) -> StructId {
    match e {
        Expect::Output(v) => {
            let leaf = str_leaf(b, v);
            form(b, "expect-output", vec![leaf])
        }
        Expect::Error(code, msg) => {
            let cl = str_leaf(b, code);
            let mut leaves = vec![cl];
            if let Some(m) = msg {
                leaves.push(str_leaf(b, m));
            }
            form(b, "expect-error", leaves)
        }
        Expect::Trap(reason) => {
            let leaf = str_leaf(b, reason);
            form(b, "expect-trap", vec![leaf])
        }
        Expect::Declines(msg) => {
            let leaves = match msg {
                Some(m) => vec![str_leaf(b, m)],
                None => vec![],
            };
            form(b, "expect-declines", leaves)
        }
    }
}

/// A `(head child…)` form node built directly in the arena.
fn form(b: &mut Builder, head: &str, mut children: Vec<StructId>) -> StructId {
    let h = b.name(head);
    children.insert(0, h);
    b.list(children)
}

/// A string-leaf node (`"…"`) — the safe carrier for any text field.
fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(s)))
}

/// A filesystem-safe, deterministic slug from a case description: lowercase ASCII-alphanumerics kept,
/// every other run collapsed to a single `-`, trimmed, capped. Purely cosmetic — the `NNNN-` index
/// prefix already guarantees per-file uniqueness + order; the slug just makes the filename readable.
fn slug(desc: &str) -> String {
    let mut s = String::new();
    let mut prev_dash = false;
    for ch in desc.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            s.push('-');
            prev_dash = true;
        }
        if s.len() >= 48 {
            break;
        }
    }
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "case".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::sexpr;

    /// `baseline_descriptions` reads the description half of each `verdict\tdescription` line, skipping
    /// `#`-comments and blanks (matching the gate's baseline shape).
    #[test]
    fn baseline_descriptions_reads_the_description_column() {
        let bl = "# gate baseline\n\
                  pass\ta passing case\n\
                  \n\
                  todo\tan incomplete case\n\
                  fail\ta known-fail case\n";
        assert_eq!(
            baseline_descriptions(bl),
            vec![
                "a passing case".to_string(),
                "an incomplete case".to_string(),
                "a known-fail case".to_string(),
            ]
        );
    }

    /// `baseline_drift` reports VANISHED (baseline title absent from corpus — the red-causing drift) and
    /// MISSING (corpus title absent from baseline), both sorted; a matched title is neither.
    #[test]
    fn baseline_drift_flags_vanished_and_missing() {
        let corpus: std::collections::BTreeSet<String> = ["stays", "brand new case"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let baseline = vec![
            "stays".to_string(),
            "renamed OLD title".to_string(), // in baseline, not in corpus → vanished
        ];
        let (vanished, missing) = baseline_drift(&corpus, &baseline);
        assert_eq!(vanished, vec!["renamed OLD title".to_string()]);
        assert_eq!(missing, vec!["brand new case".to_string()]);

        // A baseline that exactly matches the corpus has no drift either way.
        let exact = vec!["stays".to_string(), "brand new case".to_string()];
        assert_eq!(
            baseline_drift(&corpus, &exact),
            (Vec::<String>::new(), Vec::<String>::new())
        );
    }

    /// `corpus_descriptions` extracts the `(case "…")` titles of a compiler-genre file (the strings the
    /// shred records + the baseline stores) — the current-title set the drift lint diffs against.
    #[test]
    fn corpus_descriptions_reads_compiler_case_titles() {
        let src = r#"(case "first case" (input 1) (output (: 1 Int64)))
(case "second case" (input 2) (output (: 2 Int64)))"#;
        let descs = corpus_descriptions(src).expect("reads");
        assert!(descs.contains(&"first case".to_string()), "got {descs:?}");
        assert!(descs.contains(&"second case".to_string()), "got {descs:?}");
    }

    /// Each per-case artifact is a WELL-FORMED binary AST: it decodes, and re-encoding the decode is
    /// stable. Drives real cases through `read` so the builders see the actual `Record` shape (output /
    /// error+message / call+arg). Checks `program.ast` (reader-built bytes decode to the SAME AST as the
    /// text `program`), `test-run.ast` (built here — decodes, carries the graded outcome), and that a
    /// synthesized-world case emits NO `compile-unit.ast` (`None`).
    #[test]
    fn shred_artifacts_are_well_formed_binary_ast() {
        let recs = crate::read(
            r#"(case "out" (input 42) (output (: 42 Int64)))
               (case "err" (input 1_) (error CDZ0201 (message "separator")))
               (case "run" (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                     (call main 41) (output (: 42 Int64)))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 3);
        for rec in &recs {
            // program.ast decodes AND is the binary of the SAME normalized program as the `program` text.
            // Compare against SINGLE-FORM `sexpr::read` (root = the `(do … (export …))` form itself) — the
            // shape `cdz convert --to binary` and the compiler consume; NOT `read_all`, which wraps a whole
            // corpus FILE's forms in an extra synthetic `(do …)` and would double-wrap this lone program.
            let prog = codec::decode(&rec.program_ast).expect("program.ast must decode");
            let prog_text = sexpr::read(&rec.program).expect("program text reparses");
            assert_eq!(
                sexpr::print(&prog),
                sexpr::print(&prog_text),
                "program.ast == program text AST (single-form root, compiler convention)"
            );
            // The root is the program form, NOT a synthetic document wrapper: its head is `do` and it
            // carries an `(export …)` directly among its children (not buried under a second `(do …)`).
            assert!(
                sexpr::print(&prog).starts_with("(do ") && sexpr::print(&prog).contains("(export "),
                "program.ast root is the runnable (do … (export …)) form: {}",
                sexpr::print(&prog)
            );
            // test-run.ast decodes (built arena-direct via Builder).
            let tr = test_run_ast(rec);
            let tr_ast = codec::decode(&tr).expect("test-run.ast must decode");
            assert_eq!(sexpr::print(&tr_ast), sexpr::print(&tr_ast)); // decodes to a stable AST
            // No world imposed → no wit-world artifact and no component-name.
            assert!(
                rec.wit_world_ast.is_none() && rec.component_name.is_none(),
                "synthesized-world case emits no wit-world / component-name"
            );
        }
        // test-run carries the graded outcome as string leaves (awkward chars safe): the error case pins
        // its code; the run case its call export.
        let err_tr = sexpr::print(&codec::decode(&test_run_ast(&recs[1])).unwrap());
        assert!(
            err_tr.contains("CDZ0201"),
            "error code in test-run: {err_tr}"
        );
        let run_tr = sexpr::print(&codec::decode(&test_run_ast(&recs[2])).unwrap());
        assert!(run_tr.contains("main"), "call export in test-run: {run_tr}");
    }

    /// `oracle-trials` emits each trial's VALUES as BINARY AST (parsed from value-form text, not opaque
    /// string leaves like `test_run_ast`): the expected output `(: 42 Int64)` is the PARSED ascription
    /// AST, the arg value is a parsed int, the error is a code leaf. Every artifact decodes.
    #[test]
    fn oracle_trial_ast_carries_values_as_binary_ast() {
        let recs = crate::read(
            r#"(case "out" (input 42) (output (: 42 Int64)))
               (case "err" (input 1_) (error CDZ0201 (message "separator")))
               (case "run" (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                     (call main 41) (output (: 42 Int64)))"#,
        )
        .unwrap();
        // `out`: the expected value is the PARSED ascription AST `(: 42 Int64)`, NOT a string leaf.
        let out = sexpr::print(&codec::decode(&oracle_trials_ast(&recs[0])).unwrap());
        assert!(
            out.contains("expect-value") && out.contains("(: 42 Int64)"),
            "expected value as parsed AST: {out}"
        );
        // `err`: the diagnostic code as a leaf under expect-error.
        let err = sexpr::print(&codec::decode(&oracle_trials_ast(&recs[1])).unwrap());
        assert!(
            err.contains("expect-error") && err.contains("CDZ0201"),
            "expect-error code: {err}"
        );
        // `run`: the call export + the arg value PARSED to an int AST (bare `41`, not a string).
        let run = sexpr::print(&codec::decode(&oracle_trials_ast(&recs[2])).unwrap());
        assert!(
            run.contains("call") && run.contains("(arg 41)"),
            "call + parsed arg AST: {run}"
        );
    }

    /// A `(live-objects N)` case's `test-run.ast` carries the balance as a `(live-objects <N>)` form (the
    /// marker the nix exec branches on + `cdz-run --grade` reads). A case without the clause emits none.
    #[test]
    fn shred_test_run_carries_live_objects() {
        let recs = crate::read(
            r#"(case "bal"
                 (input (do (def (main (: a Int64) (: b Int64)) (Int64.of (+ (BigInt.of a) (BigInt.of b)))) (export main)))
                 (call main (: 40 Int64) (: 2 Int64)) (output (: 42 Int64))
                 (live-objects 0))
               (case "nobal"
                 (input (do (def (main (: b Bool)) b) (export main)))
                 (call main (: true Bool)) (output (: true Bool)))"#,
        )
        .unwrap();
        let bal = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            bal.contains("(live-objects \"0\")"),
            "test-run carries the balance: {bal}"
        );
        let nobal = sexpr::print(&codec::decode(&test_run_ast(&recs[1])).unwrap());
        assert!(
            !nobal.contains("live-objects"),
            "a case without the clause emits no live-objects form: {nobal}"
        );
        // A `(live-objects known-leak N)` marker shreds to `(live-objects "known-leak" "N")`.
        let leak = crate::read(
            r#"(case "leak"
                 (input (do (type L (Cons (Tuple Int64 L)) Nil) (def (main) (L.Cons (tuple 1 (L.Nil ())))) (export main)))
                 (call main) (output (: (L.Cons (tuple 1 (L.Nil ()))) L))
                 (live-objects known-leak 2))"#,
        )
        .unwrap();
        let leak_tr = sexpr::print(&codec::decode(&test_run_ast(&leak[0])).unwrap());
        assert!(
            leak_tr.contains("(live-objects \"known-leak\" \"2\")"),
            "known-leak marker shreds distinctly: {leak_tr}"
        );
    }

    /// A `(then …)` two-call continuation and a `(drop)` clause shred into the `test-run.ast` as
    /// `(then-call)`/`(then-arg <v>)`/`(drop-handle)` trial nodes — so the nix GRADE path (which reads
    /// this AST) drives the closure the same way the direct gate does. A case with neither emits none.
    #[test]
    fn shred_test_run_carries_then_and_drop() {
        let recs = crate::read(
            r#"(case "twodrop"
                 (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
                 (call adder (: 10 Int64) (: 5 Int64))
                 (then (: 7 Int64))
                 (drop)
                 (output (: (tuple 15 17) (Tuple Int64 Int64))))
               (case "plain"
                 (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                 (call main (: 5 Int64)) (output (: 6 Int64)))"#,
        )
        .unwrap();
        let tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(tr.contains("(then-call)"), "then-call marker: {tr}");
        assert!(
            tr.contains("(then-arg \"7\")"),
            "then-arg carries the second-call arg: {tr}"
        );
        assert!(tr.contains("(drop-handle)"), "drop-handle marker: {tr}");
        let plain = sexpr::print(&codec::decode(&test_run_ast(&recs[1])).unwrap());
        assert!(
            !plain.contains("then-call") && !plain.contains("drop-handle"),
            "an ordinary one-call case emits no then/drop nodes: {plain}"
        );
    }

    /// A `(wit-world …)` + `(component-name …)` case emits the world as a NATIVE `wit-world.ast` — the
    /// `(world …)` subtree ITSELF (its root head is `world`, no `(wit-world …)`/`(compile-unit …)` wrapper),
    /// exactly the shape the compiler's `wit-world:<name>=` input reads — and the interface name as the plain
    /// `component_name` string. This is the whole "no transform at compile time" contract: the artifact is
    /// the compiler's native input verbatim.
    #[test]
    fn wit_world_case_emits_the_world_subtree_and_component_name() {
        let recs = crate::read(
            r#"(case "boundary"
                  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (d ("option" (s64))))))))))
                  (component-name "cadenza:demo/iface")
                  (input (do (def (f (: m (Record (: x Int64)))) (record (= d Option.None))) (export f)))
                  (call f (: (record (= x 0)) (Record (: x Int64))))
                  (output (: (record (= d (None unit))) (record (d (Option Int64))))))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 1);
        let rec = &recs[0];
        let world = codec::decode(rec.wit_world_ast.as_ref().expect("wit-world.ast present"))
            .expect("wit-world.ast decodes");
        let world_text = sexpr::print(&world);
        // Root IS the `(world …)` node — not wrapped in `(wit-world …)` or `(compile-unit …)`.
        assert!(
            world_text.starts_with("(world w ") && world_text.contains("(export iface"),
            "wit-world.ast root is the (world …) subtree verbatim: {world_text}"
        );
        assert!(
            !world_text.starts_with("(wit-world") && !world_text.contains("compile-unit"),
            "no wrapper node around the world: {world_text}"
        );
        assert_eq!(rec.component_name.as_deref(), Some("cadenza:demo/iface"));
    }
}

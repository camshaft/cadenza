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

use cadenza_syntax::ast::{Builder, Leaf, StructId};
use cadenza_syntax::codec;

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
    /// holding the case's BINARY-AST artifacts (`program.ast`, `module-*.ast`?, `compile-unit.ast`?,
    /// `test-run.ast`), plus a `DIR/<stem>/manifest` listing the case dirs in order. These are the
    /// per-case units the nix corpus pipeline caches on; see
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
}

/// Run a corpus command per `args`, returning the process exit code. `prog` names the tool in
/// diagnostics (`cdz-corpus` for the standalone bin, `cdz` for the unified one).
pub fn run(args: &CorpusArgs, prog: &str) -> ExitCode {
    let result = match &args.command {
        CorpusCmd::Records { files, out_dir } => match out_dir {
            Some(dir) => shred_records(files, dir),
            None => run_records(files),
        },
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

/// `records --out-dir DIR FILE…`: SHRED each corpus file into one directory per case under
/// `DIR/<stem>/<NNNN>-<slug>/`, each holding up to three BINARY-AST artifacts split by consumer
/// (`design/DESIGN-corpus-nix-per-case-caching.md`), plus a `DIR/<stem>/manifest` listing the case dirs
/// in order. The artifacts:
///
/// - `program.ast` — the normalized program, fed straight to the compiler (build).
/// - `module-<name>.ast` — one per sibling LIBRARY module (multi-file package cases), also a compiler
///   input (the entry `(import "name")`s it).
/// - `compile-unit.ast` — wit-world + component-name, the compilation config (also a compiler input);
///   omitted for the common synthesized-world case.
/// - `test-run.ast` — description + trials (call/args/expect) + host-calls/responses + warns, the
///   run/grade metadata consumed by the runner (exec), not the compiler.
///
/// Splitting by consumer is the caching win: the build derivation keys on {program+modules, compile-unit}
/// so a run-metadata edit (expected output, args, host tape) never rebuilds; the exec derivation keys on
/// {artifact, test-run} so it is compiler-independent. Each artifact is a real binary AST (via
/// `codec::encode`), not a text format — we already parse the `.sexp`, so we emit the parsed form.
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
            if let Some(cu) = &rec.compile_unit_ast {
                write_bytes(&cdir.join("compile-unit.ast"), cu)
                    .map_err(|e| format!("{path} case {i} compile-unit: {e}"))?;
            }
            write_bytes(&cdir.join("test-run.ast"), &test_run_ast(rec))
                .map_err(|e| format!("{path} case {i} test-run: {e}"))?;
            manifest.push_str(&case);
            manifest.push('\n');
        }
        std::fs::write(dir.join("manifest"), &manifest)
            .map_err(|e| format!("writing {}/manifest: {e}", dir.display()))?;
    }
    Ok(())
}

/// Write a per-case binary-AST artifact (bytes the reader already built, or `test_run_ast` here) to disk.
fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
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
            let ex = str_leaf(&mut b, &c.export);
            tk.push(form(&mut b, "call", vec![ex]));
            for a in &c.args {
                let al = str_leaf(&mut b, a);
                tk.push(form(&mut b, "arg", vec![al]));
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
            // program.ast decodes AND is the binary of the SAME normalized program as the `program` text
            // (one arena, two serializations — no round-trip through the other).
            let prog = codec::decode(&rec.program_ast).expect("program.ast must decode");
            let prog_text = sexpr::read_all(&rec.program).expect("program text reparses");
            assert_eq!(
                sexpr::print(&prog),
                sexpr::print(&prog_text),
                "program.ast == program text AST"
            );
            // test-run.ast decodes (built arena-direct via Builder).
            let tr = test_run_ast(rec);
            let tr_ast = codec::decode(&tr).expect("test-run.ast must decode");
            assert_eq!(sexpr::print(&tr_ast), sexpr::print(&tr_ast)); // decodes to a stable AST
            // No world imposed → no compile-unit artifact.
            assert!(
                rec.compile_unit_ast.is_none(),
                "synthesized-world case emits no compile-unit"
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
}

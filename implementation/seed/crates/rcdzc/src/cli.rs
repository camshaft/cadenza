//! The `rcdzc` compile DRIVER — the clap-free host-boundary core BOTH front-ends (the standalone
//! `cdz-compile` bin, via `rcdzc-cli`, and the unified `cdz` bin) drive. It reads named input
//! artifacts, applies the `--export` splice + `--entry`/`--component-name` injection, runs the pure
//! [`crate::compile`], and writes each produced artifact to a file. Diagnostics go to stderr; a nonzero
//! exit means at least one error diagnostic.
//!
//! This is the HOST boundary's FILESYSTEM + reporting half — the concerns the pure core deliberately
//! excludes so that core ports to the Cadenza self-host. ARG PARSING (the `clap` `CompileArgs` + the
//! trace-sink install) lives in the SEPARATE `rcdzc-cli` crate, keeping the compiler LIBRARY free of
//! `clap` (operator directive 2026-08-30: "The compiler should be a pure library"). `rcdzc-cli::run`
//! parses args into these functions' parsed-value arguments and calls [`run_with_specs`]; the `cdz`
//! front-end calls the same entry with its own parsed args. All compilation logic lives behind
//! `crate::compile`.
//!
//! Input specs (the strings [`run_with_specs`] reads): `path`, `name=path`, or `kind:name=path`; kind
//! defaults to `ast`, name to the file stem. `-o` (the `out` argument) is a DIRECTORY into which each
//! artifact is written as `<name>.<ext-for-kind>` — EXCEPT when exactly one artifact is produced and it
//! is not an existing directory, in which case it is the exact output FILE path. With no `-o`, artifacts
//! are written to the current directory.

use crate::{Artifact, OptLevel, Severity, Target, compile_with_opt_and_overflow};
use std::path::PathBuf;
use std::process::ExitCode;

/// The parsed-values compile core the front-ends drive (`rcdzc-cli::run` for the `cdz-compile` bin, the
/// `cdz` bin's own parser for `cdz compile`) — reads the named input artifacts
/// (from disk, or stdin for `-`), applies the `--export` splice + `--entry`/`--component-name` injection,
/// and runs the compile. This is the shared, clap-free entry: `rcdzc-cli::run` (the `cdz-compile` bin)
/// parses `clap` args into these values and calls it, and the `cdz` front-end drives it from its OWN
/// parsed arguments — the thin-`cdz` `!standalone` seam: `cdz` owns arg parsing; a `standalone` build
/// calls this in-process, a `!standalone` build delegates the same values to `cdz-compile`.
#[allow(clippy::too_many_arguments)]
pub fn run_with_specs(
    input_specs: &[String],
    targets: &[Target],
    out: Option<PathBuf>,
    entry: Option<&str>,
    export: Option<&str>,
    component_name: Option<&str>,
    opt_level: OptLevel,
    overflow: crate::db::OverflowSpec,
    emit_diagnostics: Option<&std::path::Path>,
    prog: &str,
) -> ExitCode {
    // Read each named input artifact — from disk, or from stdin when the path is `-` (so the bin
    // composes in a pipe: `… | rcdzc - -o -`). A `-` input takes the kind/name from its spec, both
    // defaulting to `ast`/`main` since a piped artifact has no file stem to name it after.
    let mut inputs: Vec<Artifact> = Vec::new();
    for spec in input_specs {
        let parsed = parse_input_spec(spec);
        let bytes = if parsed.path.as_os_str() == "-" {
            let mut buf = Vec::new();
            if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf) {
                eprintln!("{prog}: cannot read stdin: {e}");
                return ExitCode::FAILURE;
            }
            buf
        } else {
            match std::fs::read(&parsed.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{prog}: cannot read {}: {e}", parsed.path.display());
                    return ExitCode::FAILURE;
                }
            }
        };
        inputs.push(Artifact::new(parsed.kind, parsed.name, bytes));
    }

    // `--export <SYM>` = the two-stage SPLICE mode: FLAT-MERGE every `ast` input's top-level defs into ONE
    // program + append a single `(export <SYM>)`, replacing the whole input set with that one program. This
    // is the per-test shred compile (`rcdzc closure.cdzb test.cdzb --export sym`): the shared-closure
    // fragment + the per-test fragment concatenate into one standalone component (NOT a cross-component
    // package link — that is `--entry`, which `conflicts_with = "entry"` keeps mutually exclusive).
    if let Some(export_sym) = export {
        match splice_ast_inputs(&inputs, export_sym) {
            Ok(spliced) => inputs = vec![spliced],
            Err(e) => {
                eprintln!("{prog}: --export: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // A `--entry <NAME>` names the package entry file — inject it as a `KIND_ENTRY` artifact (its bytes
    // ARE the entry name), the same stream `compile()` reads the entry from (`DESIGN-package-linking.md`
    // §3c). Absent, a multi-`ast` package declines (no rule to pick the entry); a single-file compile
    // needs none.
    if let Some(entry) = entry {
        inputs.push(entry_artifact(entry));
    }
    // A `--component-name <INTERFACE>` names the interface a PROVIDER publishes its exports under — inject
    // it as a `KIND_COMPONENT_NAME` artifact (X4b).
    if let Some(iface) = component_name {
        inputs.push(component_name_artifact(iface));
    }

    run_prepared_with_overflow(
        inputs,
        targets,
        out,
        opt_level,
        overflow,
        emit_diagnostics,
        prog,
    )
}

// The `entry`/`component-name` INPUT-artifact BUILDERS are compile-boundary helpers the FRONT-END
// uses — moved to `cadenza-compile-abi`. Re-exported so `crate::cli::{entry_artifact,
// component_name_artifact}` (and `rcdzc::cli::…`) stay byte-stable for `run_with_specs`/`run`.
pub use cadenza_compile_abi::abi::{component_name_artifact, entry_artifact};

/// FLAT-MERGE the `ast` input artifacts into ONE `(do (def..)+ (export <sym>))` program artifact — the
/// `--export` two-stage splice. Each input's top-level items concatenate in order: a `(do item..)` root
/// contributes its items (the shared-closure fragment + the per-test fragment are each a no-export
/// `(do (def..)..)`), a bare single-form root contributes itself. A single `(export <sym>)` is appended so
/// the merged standalone component publishes exactly the test's boundary export. Errors when an input isn't
/// a decodable `ast` artifact (the merged program's own well-formedness — an undefined `<sym>`, a duplicate
/// def — is left to the compiler, which reports it precisely). Returns the merged `KIND_AST` artifact.
fn splice_ast_inputs(inputs: &[Artifact], export_sym: &str) -> Result<Artifact, String> {
    use crate::ast::Builder;
    let mut b = Builder::new();
    let mut items: Vec<crate::ast::StructId> = Vec::new();
    for art in inputs {
        if art.kind != Artifact::KIND_AST {
            return Err(format!(
                "input `{}` is kind `{}`, not `ast` — --export splices ast fragments only",
                art.name, art.kind
            ));
        }
        let src = crate::codec::decode(&art.bytes).ok_or_else(|| {
            format!(
                "input `{}` is not a decodable cadenza-ast program",
                art.name
            )
        })?;
        match src.as_form(src.root, "do") {
            // A `(do item..)` fragment contributes each of its items (deep-copied into the new arena).
            Some(do_items) => {
                let owned: Vec<crate::ast::StructId> = do_items.to_vec();
                for it in owned {
                    items.push(copy_subtree(&mut b, &src, it));
                }
            }
            // A bare single-form program contributes itself.
            None => items.push(copy_subtree(&mut b, &src, src.root)),
        }
    }
    // The single boundary export the standalone component publishes: `(export <sym>)`.
    let export_head = b.name("export");
    let export_name = b.name(export_sym);
    let export_form = b.list(vec![export_head, export_name]);
    items.push(export_form);
    // Wrap all items in one `(do …)` program root.
    let do_head = b.name("do");
    let mut children = Vec::with_capacity(items.len() + 1);
    children.push(do_head);
    children.extend(items);
    let root = b.list(children);
    Ok(Artifact::new(
        Artifact::KIND_AST,
        export_sym,
        crate::codec::encode(&b.finish(root)),
    ))
}

/// Deep-copy the subtree rooted at `id` of `src` into builder `b`, returning the new root id. Iterative
/// post-order so a deep program can't overflow the native stack. (A local twin of the same routine the
/// `cdz` doc/repl assemblers use over this shared `cadenza_ast` arena — no public graft exists to share.)
fn copy_subtree(
    b: &mut crate::ast::Builder,
    src: &crate::ast::Arenas,
    id: crate::ast::StructId,
) -> crate::ast::StructId {
    use crate::ast::Struct;
    enum Job {
        Visit(crate::ast::StructId),
        EmitList(usize),
    }
    let mut jobs = vec![Job::Visit(id)];
    let mut results: Vec<crate::ast::StructId> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match src.get(sid) {
                Struct::Atom(lid) => {
                    let leaf = src.leaf(*lid).clone();
                    let n = b.atom_leaf(leaf);
                    results.push(n);
                }
                Struct::List(kids) => {
                    jobs.push(Job::EmitList(kids.len()));
                    for &k in kids.iter().rev() {
                        jobs.push(Job::Visit(k));
                    }
                }
            },
            Job::EmitList(n) => {
                let kids = results.split_off(results.len() - n);
                let node = b.list(kids);
                results.push(node);
            }
        }
    }
    results.pop().expect("copy_subtree leaves a root")
}

/// Compile a set of ALREADY-BUILT input artifacts to the requested targets and write the outputs — the
/// host boundary's compile+report+write tail, exposed so a wrapping driver (the `cdz` bin) can
/// pre-build artifacts from SOURCE files (parsing them in-process with its front-end, injecting the
/// `ast` + `spans` artifacts) and reuse the identical output-writing behavior. `targets` is the
/// explicit `--target` list (empty ⇒ apply the default here). `out` is the `-o` destination. Uses the
/// default (empty) GLOBAL overflow policy; a driver with a `Project.cdz` overflow global uses
/// [`run_prepared_with_overflow`] directly, so every existing caller stays unchanged.
pub fn run_prepared(
    inputs: Vec<Artifact>,
    targets: &[Target],
    out: Option<PathBuf>,
    opt_level: OptLevel,
    emit_diagnostics: Option<&std::path::Path>,
    prog: &str,
) -> ExitCode {
    run_prepared_with_overflow(
        inputs,
        targets,
        out,
        opt_level,
        crate::db::OverflowSpec::default(),
        emit_diagnostics,
        prog,
    )
}

/// [`run_prepared`] parameterized by the GLOBAL overflow policy (`overflow`) — the sink a driver uses to
/// pass a `Project.cdz` `def overflow-signed`/`overflow-unsigned` global through to the compile (it
/// reaches `db.global_overflow` via [`crate::compile_with_opt_and_overflow`]). `run_prepared(..)` is
/// exactly this with `OverflowSpec::default()` (None/None → the built-in `Trap`).
pub fn run_prepared_with_overflow(
    inputs: Vec<Artifact>,
    targets: &[Target],
    out: Option<PathBuf>,
    opt_level: OptLevel,
    overflow: crate::db::OverflowSpec,
    emit_diagnostics: Option<&std::path::Path>,
    prog: &str,
) -> ExitCode {
    // Apply the target default here (so both `run` and an external driver get the same rule): explicit
    // targets win; else `[Wasm]` UNLESS a `sidecar` input drives the run (then its Emit requests name
    // the targets, and a default `wasm` would force an unwanted component for a query-only sidecar).
    // The default is the UNDECORATED `Wasm` component (debug excluded), so a non-interactive build that
    // names no debug target proceeds without asking whether to emit debug information. WHICH target to
    // emit is an open point resolvable more than one way; it carries this declared default (`[Wasm]`),
    // so a build reaching it without an explicit `--target` applies the default rather than halting.
    //= spec/capabilities/debug-information.md#whether-to-emit-debug-information-is-a-user-facing-choice
    //# Whether a derivation emits debug information MUST carry a declared default so that a non-interactive or autonomous build proceeds without asking.
    //= spec/capabilities/build-modes.md#an-open-point-carries-a-declared-default
    //# A specification point that a conforming generation could resolve in more than one way MUST carry a declared default that states the conforming choice to apply when the point is otherwise unresolved.
    //= spec/capabilities/build-modes.md#autonomous-mode-applies-a-declared-default-instead-of-asking
    //# An autonomous build MUST resolve a specification ambiguity by applying the point's declared default.
    let has_sidecar = inputs
        .iter()
        .any(|a| a.kind == crate::sidecar::KIND_SIDECAR);
    let targets: Vec<Target> = if !targets.is_empty() {
        targets.to_vec()
    } else if has_sidecar {
        Vec::new()
    } else {
        vec![Target::Wasm]
    };
    // Run the compile on a worker thread with a stack sized to reach the recursive-descent depth
    // guard, so pathologically deep input DECLINES (the guard trips) rather than overflowing the
    // native stack and aborting — the `decline-don't-crash` contract, made independent of whatever
    // stack the ambient thread happens to have. See `rcdzc::host`.
    let out_dest = out;
    let cli_out = &out_dest;
    let out = crate::run_with_compiler_stack(|| {
        compile_with_opt_and_overflow(&inputs, &targets, opt_level, overflow)
    });

    // `--emit-diagnostics <path>`: write the DIAGNOSTICS wire as a side artifact BEFORE reporting/writing,
    // UNCONDITIONALLY (even on an error/decline compile — the fault set is exactly what a caller wants
    // then), reusing `sidecar::diagnostics_wire` so it is byte-identical to the `Query::Diagnostics` result.
    // A write failure is a warning, not a compile failure — the flag is a side-channel, so the process
    // still exits with the NORMAL compile status below (it never gates). Powers the corpus C1 grade.
    if let Some(path) = emit_diagnostics {
        let wire = crate::sidecar::diagnostics_wire(&out.diagnostics);
        if let Err(e) = std::fs::write(path, &wire) {
            eprintln!(
                "{prog}: cannot write --emit-diagnostics {}: {e}",
                path.display()
            );
        }
    }

    // Report diagnostics (stderr). When the inputs carry a `spans` side-table (present whenever the run
    // compiled a SOURCE file — `cdz compile foo.cdz`), map each diagnostic's node to a source
    // `path:line:col` prefix, so `compile` gives the SAME located errors as `check` rather than leaking a
    // raw internal `(node N)` id. Without spans (a bare artifacts-in compile) the node id still rides
    // along for a caller that holds its own table — the historical behavior, unchanged.
    // Each `spans` input paired with its ARTIFACT NAME — the name is what the `link-map` keys a file by
    // (`FileSpan.path`), which `SpanData.module_path` (a debug basename) does not preserve, so a linked
    // demux must match on the name.
    let span_tables: Vec<(String, crate::spans::SpanData)> = inputs
        .iter()
        .filter(|a| a.kind == crate::spans::KIND_SPANS)
        .filter_map(|a| crate::spans::decode(&a.bytes).map(|s| (a.name.clone(), s)))
        .collect();
    // The package `link-map` (if any): a LINKED build reports a diagnostic's GLOBAL node id (post-splice,
    // offset by the file's base), but each file's span table is keyed by LOCAL (pre-link) ids — so a raw
    // lookup misses in every table and the error loses its `file:line:col`. The link-map demuxes: the
    // global id `n` falls in one file's `[base, base+count)` → `(path, n - base)` = the file + LOCAL id
    // its span table can resolve (`DESIGN-package-linking.md` §6). Empty for a single-file compile, whose
    // ids are already local (the direct lookup below handles it).
    let link_map: Vec<crate::link::FileSpan> = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::link::KIND_LINK_MAP)
        .map(|a| crate::link::decode_link_map(&a.bytes))
        .unwrap_or_default();
    // Locate a diagnostic's node as `(&span-table, start_byte)`. First try the LINKED demux (global id →
    // file + local id via the link-map), then fall back to a DIRECT lookup across the tables (a
    // single-file build, whose ids are already local). Returns the OWNING table (not just its path) so
    // line/col are read off the right file even if two files share a basename. `None` for a node no
    // table covers.
    let locate = |node: u32| -> Option<(&crate::spans::SpanData, u32)> {
        // Linked: find the file whose global range contains the node, then resolve the LOCAL id in the
        // span table with the matching artifact name.
        if let Some(fs) = link_map
            .iter()
            .find(|f| f.contains(crate::ast::StructId(node)))
        {
            let local = node - fs.struct_base;
            if let Some((_, s)) = span_tables.iter().find(|(name, _)| *name == fs.path)
                && let Some((start, _)) = s.range(crate::ast::StructId(local))
            {
                return Some((s, start));
            }
        }
        // Single-file (or a node the link-map didn't cover): the id is already local to some table.
        span_tables.iter().find_map(|(_, s)| {
            s.range(crate::ast::StructId(node))
                .map(|(start, _)| (s, start))
        })
    };
    // The START byte offset of a diagnostic's node in its source, if locatable — the sort key that
    // orders diagnostics as a reader scans top-to-bottom. Keyed by (module path, start) so a two-file
    // package still orders deterministically.
    let start_of = |d: &crate::Diagnostic| -> Option<(String, u32)> {
        d.node
            .and_then(|n| locate(n).map(|(s, start)| (s.module_path.clone(), start)))
    };
    // Report in SOURCE ORDER (by module path, then start byte), not fault-collection order — the tree
    // walk that gathers faults does not visit strictly left-to-right, so without this a reader sees an
    // error at column 22 before one at column 21, or a derived error above the line that caused it. A
    // diagnostic with no locatable span (no spans supplied, or a spanless synthesized node) sorts LAST,
    // keeping its relative order via the STABLE sort — so the ordering stays a deterministic function of
    // the source (`diagnostics.md` §Diagnostics Are Emitted In A Deterministic Order), now also legible.
    let mut ordered: Vec<&crate::Diagnostic> = out.diagnostics.iter().collect();
    ordered.sort_by(|a, b| match (start_of(a), start_of(b)) {
        (Some(ka), Some(kb)) => ka.cmp(&kb),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    // One LINE-START INDEX per span table, built ONCE — so rendering each diagnostic's `line:col` is a
    // binary search, not `line_at`/`col_at`'s O(byte_off) scan from the start of the source. A program
    // with MANY diagnostics (e.g. an unused-binding warning per def in a wide module) mapped each fault's
    // offset over the whole source → O(faults × source_len) = O(N²); the index makes it linear. Matched
    // to the located table by pointer identity (`locate` returns a borrow into `span_tables`).
    let line_starts: Vec<(&crate::spans::SpanData, crate::spans::LineStarts)> = span_tables
        .iter()
        .map(|(_, s)| (s, s.line_starts()))
        .collect();
    for d in ordered {
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        // Prefer a source location from the spans: locate the node (via the linked demux or the direct
        // lookup), then render `path:line:col` via the prebuilt per-table line-start index. Fall back to
        // `(node N)` when no spans were supplied, or to nothing when the diagnostic carries no node.
        let located = d.node.and_then(|n| {
            locate(n).map(|(s, start)| {
                let (line, col) = line_starts
                    .iter()
                    .find(|(t, _)| std::ptr::eq(*t, s))
                    .map(|(_, idx)| idx.line_col(start))
                    .unwrap_or_else(|| (s.line_at(start), s.col_at(start)));
                format!("{}:{}:{}", s.module_path, line, col)
            })
        });
        match (located, d.node) {
            (Some(loc), _) => match &d.code {
                Some(code) => eprintln!("{loc}: {sev} [{code}]: {}", d.message),
                None => eprintln!("{loc}: {sev}: {}", d.message),
            },
            (None, node) => {
                let at = node.map(|n| format!(" (node {n})")).unwrap_or_default();
                match &d.code {
                    Some(code) => eprintln!("{prog}: {sev} [{code}]{at}: {}", d.message),
                    None => eprintln!("{prog}: {sev}{at}: {}", d.message),
                }
            }
        }
    }

    // The package `link-map` (`kind == "link-map"`) is a diagnostics DEMUX companion, not a primary
    // output — it does not count toward the "single artifact ⇒ exact file" / `-o -` decisions (else a
    // plain `-o app.wasm` component build would flip to directory mode the moment a package emits one).
    // It is written only in DIRECTORY mode (as `link-map.txt`); a `-o FILE` / `-o -` build, which names
    // one output, skips it. The bytes-second `result-types` map (`KIND_RESULT_TYPES`) is the SAME kind of
    // metadata companion (it rides IN the component as a custom section; the standalone artifact is an
    // in-process convenience) — exclude it too, else EVERY wit-export build flips to directory mode the
    // moment the export result-type map is emitted (which broke `-o FILE` component builds → a compile
    // decline across the wit-world/typed-export family).
    let primary: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind != crate::link::KIND_LINK_MAP && a.kind != Artifact::KIND_RESULT_TYPES)
        .collect();

    // `-o -`: write the single produced artifact's bytes to stdout (so the bin composes in a pipe:
    // `… | rcdzc - -o - | cdz-run`). Only meaningful for a single artifact — a multi-artifact build
    // has no one stream to write, so that is an error rather than an ambiguous concatenation.
    if cli_out.as_deref().map(|p| p.as_os_str()) == Some(std::ffi::OsStr::new("-")) {
        match primary.as_slice() {
            [art] => {
                if let Err(e) = std::io::Write::write_all(&mut std::io::stdout(), &art.bytes) {
                    eprintln!("{prog}: cannot write stdout: {e}");
                    return ExitCode::FAILURE;
                }
            }
            [] => {} // no artifact (errors already reported); fall through to the exit status.
            many => {
                eprintln!(
                    "cdz: `-o -` writes ONE artifact to stdout, but {} were produced",
                    many.len()
                );
                return ExitCode::FAILURE;
            }
        }
        return if out.has_error() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    // Decide whether `-o` names an exact output FILE (a single PRIMARY artifact, not an existing
    // directory) or a DIRECTORY to write each `<name>.<ext>` into. The `link-map` companion does not
    // count — a lone component that also emits a `link-map` still writes to the exact `-o FILE`.
    let single_file_out: Option<&PathBuf> = match (cli_out, primary.as_slice()) {
        (Some(p), [_one]) if !p.is_dir() => Some(p),
        _ => None,
    };

    // A FAILED build writes NO output — like `cargo build`, which leaves no partial artifact on a compile
    // error. Without this, an errored build still wrote the `link-map` companion (`link-map.txt`) it
    // produced alongside the — absent — component, leaving a stray sidecar with no `.wasm` beside it (a
    // confusing partial state that also misleads a follow-up tool reading the map). Bail before the write
    // loop: the errors were already reported above, so just exit failure with a clean directory.
    if out.has_error() {
        return ExitCode::FAILURE;
    }

    // Write each produced artifact. In single-file (`-o FILE`) mode, write ONLY the primary artifact
    // there and skip the `link-map` companion (a `-o FILE` caller named one output). In directory mode,
    // write everything (the `link-map` lands as `link-map.txt` beside the outputs).
    for art in &out.artifacts {
        if single_file_out.is_some() && art.kind == crate::link::KIND_LINK_MAP {
            continue;
        }
        // The bytes-second `result-types` map is NEVER a file output: it rides IN the component (a
        // `cdz-result-type` custom section) + the standalone artifact is an in-process convenience. Skip it
        // in BOTH modes — in single-file mode it would OVERWRITE the component (same `-o FILE` path, written
        // after it); in directory mode a `<name>.result-types` file is a redundant stray. (Consumers read it
        // from `out.artifact` in-process or byte-scan the component section — never from a file.)
        if art.kind == Artifact::KIND_RESULT_TYPES {
            continue;
        }
        let path = match single_file_out {
            // Single artifact, `-o FILE`: write bytes to that exact path.
            Some(file) => file.clone(),
            // Otherwise: `<outdir>/<name>.<ext>`, outdir defaulting to the current directory.
            None => {
                let dir = cli_out.clone().unwrap_or_else(|| PathBuf::from("."));
                dir.join(format!("{}.{}", art.name, ext_for_kind(&art.kind)))
            }
        };
        if let Err(e) = std::fs::write(&path, &art.bytes) {
            eprintln!("{prog}: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("cdz: wrote {} ({} bytes)", path.display(), art.bytes.len());
    }

    if out.has_error() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// A parsed input spec: the artifact kind, its logical name, and the file to read it from.
struct InputSpec {
    kind: String,
    name: String,
    path: PathBuf,
}

/// Parse an input spec: `path`, `name=path`, or `kind:name=path`. Kind defaults to `ast`; name
/// defaults to the file stem.
fn parse_input_spec(spec: &str) -> InputSpec {
    // Split an optional `kind:` prefix — only when it looks like one (no path separator or `=` before
    // the colon), so a Windows-y or `name=path` spec is not mistaken for a kind.
    let (kind, rest) = match spec.split_once(':') {
        Some((k, r)) if !k.contains('/') && !k.contains('=') => (k.to_string(), r),
        _ => (Artifact::KIND_AST.to_string(), spec),
    };
    // Split an optional `name=` prefix.
    let (name, path) = match rest.split_once('=') {
        Some((n, p)) => (n.to_string(), PathBuf::from(p)),
        None => {
            let path = PathBuf::from(rest);
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("input")
                .to_string();
            (stem, path)
        }
    };
    InputSpec { kind, name, path }
}

/// The file extension a produced artifact of the given kind is written with.
fn ext_for_kind(kind: &str) -> &str {
    match kind {
        "component" => "wasm",
        // The `EmitTestsShred` MAIN provider component — written `main.wasm` (its artifact NAME is "main") so
        // `cdz test --emit-shred -o D` produces the fixed `D/main.wasm` the per-test targets `--peer`-link.
        "component-provider" => "wasm",
        // The `EmitTestsShred` cadenza-ast manifest — a `codec::encode`d value (the `(shred-manifest …)`
        // tree), written `manifest.cdzb` (binary cadenza-ast, decoded with `cdz convert --from binary`).
        "shred-manifest" => "cdzb",
        "rust" => "rs",
        // A detached DWARF sidecar (Mode S) is a bare `.wasm`-format core module of debug sections;
        // written with a `.dwarf` extension so it is distinct from the runnable `<name>.wasm`.
        "dwarf" => "dwarf",
        // Sidecar QUERY results are UTF-8 text (a rendered type, a newline-separated node-id list) —
        // written with a `.txt` extension. A `sidecar` INPUT is read generically as `kind:name=path`,
        // so no case is needed for it here (this maps only produced OUTPUT kinds). The package
        // `link-map` (a diagnostics demux table) is likewise UTF-8 text.
        "type-info" | "uses" | "link-map" => "txt",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Builder;

    /// Build a `KIND_AST` artifact for a `(do <name>…)` fragment — each name a bare top-level item, so the
    /// splice's do-child merge is exercised without needing full `(def …)` bodies (the splice copies items
    /// structurally; def well-formedness is the compiler's concern, not the splice's).
    fn do_fragment(name: &str, items: &[&str]) -> Artifact {
        let mut b = Builder::new();
        let mut kids = vec![b.name("do")];
        for it in items {
            kids.push(b.name(*it));
        }
        let root = b.list(kids);
        Artifact::new(
            Artifact::KIND_AST,
            name,
            crate::codec::encode(&b.finish(root)),
        )
    }

    /// `--export` splices every `ast` input's `(do …)` items into ONE `(do <all-items> (export <sym>))`
    /// program — the two-stage per-test compile (shared-closure fragment ++ per-test fragment ++ export).
    /// Pins: items concatenate IN ORDER across fragments, and exactly one `(export <sym>)` is appended last.
    #[test]
    fn export_splices_do_fragments_in_order_with_a_single_export() {
        let closure = do_fragment("closure", &["helper1", "helper2"]);
        let test = do_fragment("test", &["mytest"]);
        let merged = splice_ast_inputs(&[closure, test], "mytest").expect("splices");
        assert_eq!(merged.kind, Artifact::KIND_AST);
        let a = crate::codec::decode(&merged.bytes).expect("merged decodes as cadenza-ast");
        let items = a.as_form(a.root, "do").expect("merged root is a `(do …)`");
        assert_eq!(items.len(), 4, "2 closure items + 1 test item + 1 export");
        // The three source items concatenate in fragment/source order.
        assert_eq!(a.as_name(items[0]), Some("helper1"));
        assert_eq!(a.as_name(items[1]), Some("helper2"));
        assert_eq!(a.as_name(items[2]), Some("mytest"));
        // The final item is exactly `(export mytest)`.
        let export = a
            .as_form(items[3], "export")
            .expect("last item is `(export …)`");
        assert_eq!(export.len(), 1);
        assert_eq!(a.as_name(export[0]), Some("mytest"));
    }

    /// A non-`ast` input is rejected — `--export` splices ast fragments only (a wrong-kind input is a
    /// caller error, surfaced as a tool error rather than silently mis-spliced).
    #[test]
    fn export_rejects_a_non_ast_input() {
        let ast = do_fragment("f", &["a"]);
        let other = Artifact::new("wasm", "w", vec![0, 1, 2]);
        let err = splice_ast_inputs(&[ast, other], "a").expect_err("non-ast input rejected");
        assert!(err.contains("not `ast`"), "{err}");
    }
}

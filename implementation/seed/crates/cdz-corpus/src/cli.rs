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

use crate::{DiagQuality, Expect, Record, ReplMatch};

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
        /// QUOTE-WRAP the shred (requires `--out-dir`): instead of the ordinary per-case program, emit — per
        /// eligible case — ONE two-export round-trip COMPONENT (`program.ast`) with `encodeQuoted() ->
        /// list<u8>` (= `Ast.encode (quote E)`) and `decodeCheck(list<u8>) -> bool` (= `Ast.decode b ==
        /// quote E`), plus its imposed `wit-world.ast` + `component-name` (the operator-mandated §2 shape,
        /// 2026-08-30). A bespoke exec (later increment) is the caller: runs `encodeQuoted()`, threads the
        /// bytes into `decodeCheck(bytes)` ACROSS the caller boundary, defeating const-fold. `E` is each
        /// case's RAW `(input …)` form (`input_ast`). This shape currently DECLINES (a bare `list<u8>`
        /// result as one of MULTIPLE exports is not yet emittable, CDZ0900) → cases grade Todo until the
        /// operator-mandated WIT-boundary fix lands in the compiler (the pass IS that fix's acceptance
        /// gate). Eligibility: single-component cases only (multi-form wrapping is a later increment). See
        /// `design/DESIGN-quote-corpus-roundtrip-pass.md`.
        #[arg(long, requires = "out_dir")]
        quote_wrap: bool,
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
        /// MACHINE-READABLE: emit ONLY the missing-from-baseline integer to stdout, then exit 0 —
        /// nothing else (no prose, no warnings, no vanished guard). For a monitor cron that keys on the
        /// corpus-ahead-of-baseline count (v-fleet-tooling's baseline-drift-monitor) — parse the integer
        /// instead of grepping the prose line. Vanished detection is the DEFAULT mode + the `vanished-check`
        /// subcommand (its exit-3 contract); `--count` deliberately does not red on vanished.
        #[arg(long)]
        count: bool,
    },
    /// Check corpus files are in NATIVE compound-value form — FAST, no compile/run.
    ///
    /// Asserts `nativize_compound_source_skip_outputs(file) == file` for each `.sexp`: every INPUT-side
    /// compound value literal must already be in the native `#word` ctor form (`#list`/`#tuple`/`#record`/
    /// `#map`/`#set`), NOT the classic name-head `(list …)`/`(tuple …)`. The rewrite is IDEMPOTENT and
    /// `--skip-outputs` leaves `(output …)` expected values untouched, so this never entangles a render
    /// re-pin. A peer adding a classic-form input makes the file NON-idempotent — this lint FAILs and NAMES
    /// the file (a PR-time guard replacing by-hand drift catching; operator M3 native-form corpus). Exits
    /// NON-ZERO on any non-native file (run the nativize codemod to fix).
    NativizeCheck {
        /// Corpus `.sexp` files to check (typically the full `spec/semantics/*.sexp` glob).
        #[arg(required = true)]
        files: Vec<String>,
        /// APPLY the nativize codemod in place: rewrite each non-native file's classic name-head input
        /// compounds (`(list …)`/`(tuple …)`/…) to native `#ctor` form (`--skip-outputs` semantics —
        /// `(output …)` expected values untouched) and write it back. Idempotent; a file already native is
        /// left byte-identical. The one-command fix for the RECURRING M3 nativize red (a peer re-introducing
        /// a classic-form input); exits 0 after fixing. Without `--fix` the default is the check (exits
        /// non-zero on a non-native file). Re-run the check (or ML round-trip) after to confirm.
        #[arg(long)]
        fix: bool,
    },
    /// GUARD a corpus git-diff for `(live-objects …)` clause EDITS — the fresh-store-before-repin discipline.
    ///
    /// The debug live-objects census is corrupted by a STALE store (after a runtime-hash bump) or heavy
    /// PARALLEL-build contention — so a live-objects count "drift" seen under fleet load is SUSPECT and must
    /// be reconfirmed on a freshly-built isolated store before it is re-pinned (else a re-pin can MASK a real
    /// leak, or chase a spurious count). This guard git-diffs `spec/semantics` against `--base` and flags any
    /// changed `(live-objects …)` line with that reminder. Render-only `(output …)` edits are exempt
    /// (byte-deterministic). Advisory by default (exit 0 + warn); `--strict` exits non-zero on any hit.
    LiveObjectsGuard {
        /// The git ref to diff against (the base the re-pin is measured relative to).
        #[arg(long, default_value = "origin/main")]
        base: String,
        /// Exit NON-ZERO when a `(live-objects …)` edit is present (default: advisory warn, exit 0).
        #[arg(long)]
        strict: bool,
    },
    /// Check no case pins a CAPABILITY/implementation-limit code as an `(error …)` — FAST, no compile/run.
    ///
    /// The corpus is the impl-INDEPENDENT runnable spec (standing operator directive 2026-08-31): a program
    /// that SHOULD work per spec but the compiler does not YET implement must be a TODO — recorded as
    /// `(output <spec value>)` (grades Todo now, AUTO-LOCKS to Pass once implemented, no corpus edit),
    /// NEVER as `(error CDZ0900 …)`. `CDZ0900` (`Code::UnsupportedConstruct`) is the DECLINE umbrella for a
    /// not-yet-built construct; pinning it as an `(error …)` REJECTION asserts the program is ILL-FORMED when
    /// it is actually well-formed-but-unrealized — baking a transient compiler limitation into the spec (and
    /// forcing a corpus edit when the feature lands). This lint FLAGs any `(error <capability-limit code>)`
    /// pin. Currently ~0 (a going-forward guard). Exits NON-ZERO on any hit (convert to `(output V)`).
    /// (The bare `(declines)` marker was removed entirely — a should-work is a TODO `(output V)`.)
    CapabilityErrorCheck {
        /// Corpus `.sexp` files to check (typically the full `spec/semantics/*.sexp` glob).
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Flag VANISHED titles in a `.gate-baseline*` — baseline descriptions with no corpus case — FAST, no
    /// compile/run. The authoritative, reusable primitive behind a pre-commit baseline-guard (v-fleet-tooling).
    ///
    /// A vanished title reds a full `gate --check`/gate-local fleet-wide but only AFTER a ~15-30min gate; this
    /// wraps the same `corpusVanishedCheck` set-diff (baseline descriptions minus corpus `(case …)`/
    /// `(platform-case …)` titles, via `cdz-corpus records`) as a fast text-only check so a pre-commit hook can
    /// BLOCK a contaminated baseline BEFORE it lands. Catches the `#7176`/`#6835` class: a non-harvest bulk
    /// re-pin (or a title-changing conversion that did not co-remove the old title) that re-injects stale
    /// titles. Accepts MULTIPLE baselines so a hook can check all 3 staged `.gate-baseline{,-rust,-rust-async}`
    /// at once. A faithful `nix save-baseline` harvest has vanished==0 by construction, so this NEVER
    /// false-positives a real re-baseline. Exits NON-ZERO listing every vanished title.
    VanishedCheck {
        /// The `.gate-baseline*` file(s) to check (1+; e.g. all 3 staged backends).
        #[arg(required = true)]
        baselines: Vec<String>,
        /// The corpus `.sexp` files whose case titles form the CURRENT set. Pass the COMPLETE set the
        /// baseline covers (the full `spec/semantics/*.sexp` glob) — a SUBSET makes every other file's
        /// baselined case look vanished (a false positive).
        #[arg(long, required = true, num_args = 1..)]
        corpus: Vec<String>,
        /// Suppress the per-baseline `OK — … 0 vanished` lines; emit output ONLY on a vanished hit (for a
        /// terse pre-commit hook). Errors + the vanished lines still print.
        #[arg(long)]
        quiet: bool,
        /// REWRITE each baseline in place, DELETING every orphan-vanished `<verdict>\t<title>` entry line
        /// (title with no corpus case) — the one-command manual/backstop fix for the union-merge-driver
        /// orphan re-add hazard, instead of hand-editing. `#`-comment, blank, and tab-less lines are
        /// preserved verbatim; only real entry lines are pruned. FAIL-OPEN: if the corpus title set is
        /// EMPTY (a `--corpus` glob that matched no readable cases), pruning is REFUSED (exit 2) so a bad
        /// glob can never strip every entry. With `--prune` the exit is 0 (the orphans are now gone).
        #[arg(long)]
        prune: bool,
    },
}

/// Run a corpus command per `args`, returning the process exit code. `prog` names the tool in
/// diagnostics (`cdz-corpus` for the standalone bin, `cdz` for the unified one).
pub fn run(args: &CorpusArgs, prog: &str) -> ExitCode {
    // `vanished-check` uses DISTINCT exit codes (a stable contract for the pre-commit baseline-guard hook,
    // v-fleet-tooling #7197): 0 = OK (no vanished), 3 = a VANISHED title was DETECTED (the BLOCK signal),
    // 2 = a tooling/usage error (unreadable file, &c. — the hook FAILS OPEN on this, never blocking a commit
    // on a broken tool). This lets the hook key on exit 3 rather than a fragile output-string grep. The
    // `VANISHED baseline` output token is ALSO kept stable for the current grep-based hook.
    if let CorpusCmd::VanishedCheck {
        baselines,
        corpus,
        quiet,
        prune,
    } = &args.command
    {
        return match vanished_across(baselines, corpus, *quiet, *prune) {
            Ok(0) => ExitCode::SUCCESS,
            Ok(_) => ExitCode::from(3), // vanished detected → BLOCK
            Err(msg) => {
                eprintln!("{prog}: {msg}");
                ExitCode::from(2) // tooling/usage error → fail-open in the hook
            }
        };
    }
    let result = match &args.command {
        CorpusCmd::Records {
            files,
            out_dir,
            quote_wrap,
        } => match out_dir {
            Some(dir) => shred_records(files, dir, *quote_wrap),
            None => run_records(files),
        },
        CorpusCmd::BaselineDrift {
            files,
            baseline,
            list_missing,
            count,
        } => check_baseline_drift(files, baseline, *list_missing, *count),
        CorpusCmd::NativizeCheck { files, fix } => check_nativize_idempotence(files, *fix),
        CorpusCmd::LiveObjectsGuard { base, strict } => check_live_objects_edits(base, *strict),
        CorpusCmd::CapabilityErrorCheck { files } => check_capability_error_pins(files),
        CorpusCmd::VanishedCheck { .. } => {
            unreachable!("vanished-check is handled above with distinct exit codes")
        }
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
    count: bool,
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

    // `--count`: emit ONLY the corpus-ahead-of-baseline integer to stdout + exit 0 (a machine-readable
    // count query for a monitor cron). Suppress the prose/warnings AND the vanished guard — vanished
    // detection lives in the default mode + `vanished-check`, so a count query never reds on it.
    if count {
        println!("{}", missing.len());
        return Ok(());
    }

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

/// `vanished-check <baseline…> --corpus FILE…`: flag VANISHED titles (baseline descriptions with no corpus
/// case) across ONE OR MORE baselines — the reusable primitive for the pre-commit baseline-guard. Builds the
/// corpus title set ONCE (shared across baselines) and set-diffs each baseline against it, printing each
/// vanished title (token `VANISHED baseline`, kept STABLE for the hook's grep). Returns the TOTAL vanished
/// count on success, or `Err` on a tooling error (unreadable/unparseable file). The caller (`run`) maps this
/// to the DISTINCT exit codes the hook keys on: 0 (count 0) / 3 (count>0 = detected) / 2 (Err = fail-open).
/// A faithful harvest has vanished==0 by construction, so this never false-positives a real re-baseline; it
/// catches the `#7176`/`#6835` contamination class (a non-harvest bulk re-pin re-injecting stale titles, or a
/// title-changing conversion without co-removal). `quiet` suppresses the per-baseline OK lines.
fn vanished_across(
    baselines: &[String],
    corpus_files: &[String],
    quiet: bool,
    prune: bool,
) -> Result<usize, String> {
    let mut corpus: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in corpus_files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        corpus.extend(corpus_descriptions(&text).map_err(|e| format!("{path}: {e}"))?);
    }
    // FAIL-OPEN guard for `--prune`: an empty corpus set would make EVERY baseline entry look vanished, so
    // an in-place prune would wipe the file. Refuse (mapped to exit 2 by the caller) rather than delete.
    // Mirrors the `corpus-case-titles` crate's whole-corpus-or-nothing contract.
    if prune && corpus.is_empty() {
        return Err(
            "--prune refused: the corpus title set is EMPTY (the --corpus glob matched no readable \
             cases); pruning would strip every baseline entry. Fix the --corpus glob (pass the full \
             spec/semantics/*.sexp)."
                .to_string(),
        );
    }
    let mut total_vanished = 0usize;
    for bl in baselines {
        let text = std::fs::read_to_string(bl).map_err(|e| format!("reading {bl}: {e}"))?;
        // `--prune`: rewrite the file dropping orphan entry lines. This FIXES the drift (rather than
        // blocking on it), so a successful prune leaves `total_vanished` at 0 → the caller exits 0.
        if prune {
            let (new_text, removed) = prune_baseline(&text, &corpus);
            if removed.is_empty() {
                if !quiet {
                    println!("vanished-check: {bl}: 0 orphan title(s) — nothing to prune");
                }
            } else {
                std::fs::write(bl, &new_text).map_err(|e| format!("writing {bl}: {e}"))?;
                println!(
                    "vanished-check: {bl}: PRUNED {} orphan baseline title(s) with no corpus case:",
                    removed.len()
                );
                for r in &removed {
                    println!("  - pruned {r:?}");
                }
            }
            continue;
        }
        let descs = baseline_descriptions(&text);
        let (vanished, _missing) = baseline_drift(&corpus, &descs);
        if vanished.is_empty() {
            if !quiet {
                println!(
                    "vanished-check: OK — {bl}: 0 vanished ({} baseline titles vs {} corpus titles)",
                    descs.len(),
                    corpus.len()
                );
            }
        } else {
            for v in &vanished {
                // `VANISHED baseline` is the STABLE token the pre-commit hook greps (v-fleet-tooling #7197);
                // do not reword it without coordinating (a reword silently degrades the hook to fail-open).
                eprintln!(
                    "vanished-check: {bl}: VANISHED baseline title has no corpus case (reds `gate --check`): {v:?}"
                );
            }
            total_vanished += vanished.len();
        }
    }
    if total_vanished > 0 {
        eprintln!(
            "vanished-check: {total_vanished} vanished baseline title(s) across {} baseline file(s) — a \
             renamed/removed case left the baseline stale (a contaminated non-harvest bulk re-pin, cf \
             #7176/#6835; or a title-changing conversion without co-removal). Regenerate via `nix run \
             .#save-baseline` or drop the stale title(s) — do NOT land a bulk baseline diff from a \
             non-harvest source.",
            baselines.len()
        );
    }
    Ok(total_vanished)
}

/// `nativize-check FILE…`: assert each corpus file is ALREADY in native compound-value input form — i.e.
/// `nativize_compound_source_skip_outputs` is a NO-OP on it. A file the rewrite CHANGES contains a classic
/// name-head input compound (`(list …)`/`(tuple …)`/…) a peer added; reported by name. `--skip-outputs`
/// leaves `(output …)` expected values untouched, so this is orthogonal to any render re-pin. Exits
/// NON-ZERO on any non-native file (fix: run the nativize codemod). The PR-time guard for operator M3's
/// native-form corpus, replacing by-hand drift catching.
/// A `; nativize-allow-classic[: reason]` directive comment — the explicit per-file exemption from the
/// nativize idempotence check (see the call site). Returns the trimmed reason (or `""`) when a comment line
/// carries the directive, else `None`. A COMMENT (not a form), so the corpus reader ignores it; only this
/// text-level check reads it. Kept a plain substring on a `;`-comment line so it is trivially greppable and
/// removable (the exemption is interim — drop the directive when the contested form is reconciled upstream).
fn nativize_allow_directive(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let after_semi = line.trim_start().strip_prefix(';')?.trim_start();
        let rest = after_semi.strip_prefix("nativize-allow-classic")?;
        Some(
            rest.trim_start()
                .strip_prefix(':')
                .map(str::trim)
                .unwrap_or("")
                .to_string(),
        )
    })
}

fn check_nativize_idempotence(files: &[String], fix: bool) -> Result<(), String> {
    let mut non_native: Vec<String> = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let nativized = sexpr::nativize_compound_source_skip_outputs(&text)
            .map_err(|e| format!("{path}: nativize failed: {e:?}"))?;
        if nativized != text {
            // EXPLICIT per-file escape hatch: a `; nativize-allow-classic[: reason]` directive exempts the
            // file from the idempotence assertion (reported ALLOWED/visible, never silent) — for a file that
            // legitimately carries a classic-head form that is a CTOR APPLICATION (`Apply{SetNew}`/etc.), NOT
            // a nativizable literal, which the codemod (head-name-based, no compile) can't discriminate and
            // whose own tests treat as a literal. The contested `(set …)` v-syntax↔rcdzc semantic is
            // reconciled elsewhere; this is the interim, least-committal, REVERSIBLE exemption (asserts
            // nothing about the semantics). concierge-greenlit 2026-09-02 for 19-sets's #7969 soundness case.
            if let Some(reason) = nativize_allow_directive(&text) {
                println!(
                    "nativize-check: ALLOWED (explicit `; nativize-allow-classic` directive) {path}{}",
                    if reason.is_empty() {
                        String::new()
                    } else {
                        format!(": {reason}")
                    }
                );
                continue;
            }
            non_native.push(path.clone());
            // `--fix`: APPLY the codemod in place (write the nativized text). The check side just records
            // the file; the write happens here so a dry check never mutates.
            if fix {
                std::fs::write(path, &nativized)
                    .map_err(|e| format!("writing nativized {path}: {e}"))?;
                println!(
                    "nativize-check: FIXED {path} — classic-form input compounds → native #ctor"
                );
            }
        }
    }
    if non_native.is_empty() {
        println!(
            "nativize-check: OK — all {} file(s) in native #ctor compound-value input form",
            files.len()
        );
        return Ok(());
    }
    if fix {
        // Every non-native file was just rewritten in place → success (the codemod applied). Re-run the
        // plain check (or the ML round-trip) to confirm; the rewrite is idempotent so a re-check is clean.
        println!(
            "nativize-check: nativized {} file(s) in place — re-run the check to confirm",
            non_native.len()
        );
        return Ok(());
    }
    for p in &non_native {
        eprintln!(
            "nativize-check: NON-NATIVE input compound in {p} — a classic name-head (list …)/(tuple …)/… ; run the nativize codemod (--fix)"
        );
    }
    Err(format!(
        "{} file(s) have classic-form input compounds — corpus must use native #ctor form (operator M3)",
        non_native.len()
    ))
}

/// Diagnostic codes that are CAPABILITY / implementation-LIMITS (a not-yet-built construct), NOT semantic
/// spec-errors. Pinning one as `(error …)` is the anti-pattern the operator directive forbids (a well-formed-
/// but-unrealized program pinned as an ill-formed REJECTION). `CDZ0900` (`Code::UnsupportedConstruct`) is the
/// compiler's DECLINE umbrella (emitted for a not-yet-built construct); a should-work case pins its VALUE as
/// `(output V)` (Todo now), and it must never be an `(error …)`. Kept as a DATA const
/// so the set can grow if new capability-limit codes surface (semantic-error codes — CDZ0201/0203/0304/0101/…
/// — are DELIBERATELY absent: those ARE the spec and correctly pin as `(error …)`).
const CAPABILITY_LIMIT_CODES: &[&str] = &["CDZ0900"];

/// The `(desc, code)` of every case in `records` that pins a [`CAPABILITY_LIMIT_CODES`] code as an
/// `Expect::Error` (the anti-pattern). Pure over parsed records so it is unit-testable; a semantic-error
/// `(error CDZ0201 …)` is correctly NOT flagged (only a capability-limit code pinned as `(error …)` is).
fn capability_error_hits(records: &[Record]) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for rec in records {
        for trial in &rec.trials {
            if let Expect::Error(code, ..) = &trial.expect
                && CAPABILITY_LIMIT_CODES.contains(&code.as_str())
            {
                hits.push((rec.description.clone(), code.clone()));
            }
        }
    }
    hits
}

/// `capability-error-check FILE…`: flag any case that pins a [`CAPABILITY_LIMIT_CODES`] code as an
/// `(error …)`. Parser-based (uses `crate::read`, so comments / multi-line are handled correctly — only a
/// genuine `Expect::Error` with a capability-limit code is flagged). A hit should be converted to
/// `(output <spec value>)` (Todo-now, auto-Pass-when-implemented).
fn check_capability_error_pins(files: &[String]) -> Result<(), String> {
    let mut hits: Vec<(String, String, String)> = Vec::new(); // (file, case description, code)
    for path in files {
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        // Only compiler-case files pin `(error …)`; platform-genre files are a distinct genre (skip).
        if crate::is_platform_genre(&text) {
            continue;
        }
        let records = crate::read(&text).map_err(|e| format!("{path}: {e}"))?;
        for (desc, code) in capability_error_hits(&records) {
            hits.push((path.clone(), desc, code));
        }
    }
    if hits.is_empty() {
        println!(
            "capability-error-check: OK — no case pins a capability-limit code ({}) as an (error …) in {} file(s)",
            CAPABILITY_LIMIT_CODES.join("/"),
            files.len()
        );
        Ok(())
    } else {
        for (path, desc, code) in &hits {
            eprintln!(
                "capability-error-check: {path}: case {desc:?} pins {code} as an (error …) — {code} is a \
                 not-yet-implemented DECLINE umbrella, not a semantic error. The corpus is the impl-independent \
                 spec: record this as (output <spec value>) [Todo now, auto-Pass when implemented], \
                 NEVER (error {code} …) (operator directive: should-work-unimplemented = Todo)"
            );
        }
        Err(format!(
            "{} case(s) pin a capability-limit code as an (error …) — convert to (output V) (operator: corpus is the impl-independent spec)",
            hits.len()
        ))
    }
}

/// Scan a `git diff --unified=0` for changed `(live-objects …)` clause lines, returning `(file, line)` per
/// hit. A `+++ b/<path>` header sets the current file; a `+`/`-` body line (not the `+++`/`---` headers)
/// containing `(live-objects` is a re-pin edit. Pure over the diff text so it is unit-testable.
fn scan_live_objects_edits(diff: &str) -> Vec<(String, String)> {
    let mut cur_file = String::new();
    let mut hits = Vec::new();
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("+++ b/") {
            cur_file = f.to_string();
        } else if (line.starts_with('+') || line.starts_with('-'))
            && !line.starts_with("+++")
            && !line.starts_with("---")
            && line.contains("(live-objects")
        {
            hits.push((cur_file.clone(), line[1..].trim().to_string()));
        }
    }
    hits
}

/// `live-objects-guard --base REF [--strict]`: flag `(live-objects …)` clause edits in a `spec/semantics`
/// git-diff so a live-objects re-pin is reconfirmed on a fresh isolated store before landing (the census is
/// corrupted by a stale store / build contention). Advisory (exit 0) unless `--strict`.
fn check_live_objects_edits(base: &str, strict: bool) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["diff", "--unified=0", base, "--", "spec/semantics"])
        .output()
        .map_err(|e| format!("git diff {base} failed to run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git diff {base} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let hits = scan_live_objects_edits(&String::from_utf8_lossy(&out.stdout));
    if hits.is_empty() {
        println!("live-objects-guard: OK — no (live-objects …) clause edits vs {base}");
        return Ok(());
    }
    for (f, l) in &hits {
        eprintln!("live-objects-guard: (live-objects …) EDIT in {f}: {l}");
    }
    eprintln!(
        "live-objects-guard: RECONFIRM each count on a FRESHLY-BUILT isolated store (not under fleet \
         load) before landing — the debug live-objects census is corrupted by a stale store (post \
         runtime-hash bump) OR heavy parallel-build contention; a re-pin off a suspect count can MASK a \
         real leak. Render-only (output …) edits are exempt (byte-deterministic)."
    );
    if strict {
        Err(format!(
            "{} (live-objects …) edit(s) vs {base} — reconfirm on a fresh isolated store (or re-run without --strict once confirmed)",
            hits.len()
        ))
    } else {
        Ok(())
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

/// Pure baseline rewrite for `--prune`: return `(new_text, removed_titles)` where `new_text` is `text`
/// with every orphan-VANISHED entry line (a `<verdict>\t<title>` whose `title` is absent from `corpus`)
/// DELETED, and everything else — `#`-comment lines, blank lines, and any tab-less line — kept verbatim.
/// The trailing-newline shape of `text` is preserved. Pure so it is unit-testable; the empty-corpus
/// fail-open guard lives at the call site (`corpus` here is assumed the complete set).
fn prune_baseline(
    text: &str,
    corpus: &std::collections::BTreeSet<String>,
) -> (String, Vec<String>) {
    let ends_with_newline = text.ends_with('\n');
    let mut out = String::new();
    let mut removed = Vec::new();
    for line in text.lines() {
        // Only a real entry line (`<verdict>\t<title>`, not `#`-comment / blank) is prunable; a tab-less
        // line has no title column and is left alone (matches `baseline_descriptions`'s entry shape).
        let orphan_title = (!line.starts_with('#') && !line.is_empty())
            .then(|| line.split_once('\t'))
            .flatten()
            .map(|(_, title)| title)
            .filter(|title| !corpus.contains(*title));
        match orphan_title {
            Some(title) => removed.push(title.to_string()),
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    // `str::lines()` drops a final trailing newline; we appended one per kept line. If the original had no
    // trailing newline, remove the one we over-added so the file's shape is byte-preserved.
    if !ends_with_newline && out.ends_with('\n') {
        out.pop();
    }
    (out, removed)
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
fn shred_records(files: &[String], out_dir: &str, quote_wrap: bool) -> Result<(), String> {
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
        if quote_wrap {
            shred_quote_wrap(&dir, &records).map_err(|e| format!("{path}: {e}"))?;
            continue;
        }
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

/// The QUOTE-WRAP shred (`records --out-dir --quote-wrap`): for each ELIGIBLE case, emit TWO single-export
/// program artifacts — `encode.ast` (`(do (def (enc) (Ast.encode (quote E))) (export enc))`) and
/// `check.ast` (`(do (def (chk (: b Bytes)) (match (Ast.decode b) ((Ok a) (= a (quote E))) ((Err _)
/// false))) (export chk))`) — plus a plain-text `description` and a `manifest` listing the eligible case
/// dirs in order. The bespoke exec (later increment) is the CALLER: it runs `encode.enc()` → binary-AST
/// bytes, threads them into `check.chk(bytes)` (assert true), then `check.chk(corrupt(bytes))` (assert
/// false) — the round-trip across the component boundary, anti-const-fold by construction (the two are
/// SEPARATE components, so `check` decodes bytes it cannot see through, forcing runtime `Ast.decode`).
///
/// TWO SEPARATE single-export components — the operator-chosen "B" shape (2026-08-30) — NOT one two-export
/// program: a bare `list<u8>`/`Bytes` result as one of MULTIPLE exports is not emittable (CDZ0900 "cannot
/// emit that typed export"), and a record-wrapped two-export program hits the multi-member typed-interface
/// gap; whereas each of these crosses a heap boundary as its program's SOLE export (the supported path).
/// Keeps the base-corpus `NNNN-slug` index so a case cross-references its sibling; INELIGIBLE cases
/// (sibling-module / peer package cases) are skipped. See `design/DESIGN-quote-corpus-roundtrip-pass.md` §6.
fn shred_quote_wrap(dir: &std::path::Path, records: &[Record]) -> Result<(), String> {
    let mut manifest = String::new();
    let world = quote_wrap_wit_world();
    for (i, rec) in records.iter().enumerate() {
        if !quote_wrap_eligible(rec) {
            continue;
        }
        let program = quote_wrap_program(&rec.input_ast)
            .ok_or_else(|| format!("case {i} {:?}: input_ast did not decode", rec.description))?;
        let case = format!("{i:04}-{}", slug(&rec.description));
        let cdir = dir.join(&case);
        std::fs::create_dir_all(&cdir).map_err(|e| format!("creating {}: {e}", cdir.display()))?;
        write_bytes(&cdir.join("program.ast"), &program)
            .map_err(|e| format!("case {i} program: {e}"))?;
        // The imposed WIT world declaring the two typed exports (`encode-quoted`/`decode-check`), + the
        // interface the guest exports under. FIXED per case (independent of E). Needed because a heap
        // value crosses as one of MULTIPLE exports only under a DECLARED world (a synthesized world caps
        // at a single heap export).
        write_bytes(&cdir.join("wit-world.ast"), &world)
            .map_err(|e| format!("case {i} wit-world: {e}"))?;
        std::fs::write(cdir.join("component-name"), QUOTE_WRAP_COMPONENT_NAME)
            .map_err(|e| format!("case {i} component-name: {e}"))?;
        std::fs::write(cdir.join("description"), &rec.description)
            .map_err(|e| format!("case {i} description: {e}"))?;
        manifest.push_str(&case);
        manifest.push('\n');
    }
    std::fs::write(dir.join("manifest"), &manifest)
        .map_err(|e| format!("writing {}/manifest: {e}", dir.display()))?;
    Ok(())
}

/// Whether a case is eligible for the quote-wrap pass. v1 covers a SINGLE-component case: its `(input …)`
/// is one quotable form (always true — a parsed input is one AST node) with NO sibling library modules and
/// NO cross-component peers (a multi-file / multi-component case is not a single quotable form; wrapping it
/// as one enclosing form is a later increment, doc §3 clause 3 / §6 increment 5). `expect-kind` is
/// irrelevant — the pass quotes E's SYNTAX and never evaluates it, so value / trap / error / declines
/// inputs are all eligible (doc §3).
fn quote_wrap_eligible(rec: &Record) -> bool {
    rec.modules.is_empty() && rec.peers.is_empty()
}

/// The interface the quote-wrap guest exports under (the `component-name`), qualifying the two exports as
/// `cadenza:quote/roundtrip#encode-quoted` / `#decode-check`.
const QUOTE_WRAP_COMPONENT_NAME: &str = "cadenza:quote/roundtrip";

/// Synthesize the QUOTE-WRAP round-trip program for a raw input form `E` (binary AST `input_ast`): ONE
/// component with TWO exports (the operator-mandated §2 shape, 2026-08-30), as binary AST. `None` iff `E`
/// does not decode.
///
/// ```text
/// (do
///   (def (encodeQuoted) ((. Ast encode) (quote E)))
///   (def (decodeCheck (: bytes Bytes))
///     (match ((. Ast decode) bytes)
///       ((Ok a)  (= a (quote E)))
///       ((Err _) false)))
///   (export encodeQuoted)
///   (export decodeCheck))
/// ```
/// `encodeQuoted` returns the binary-AST bytes of `quote E`; `decodeCheck` decodes the bytes the CALLER
/// passes back and asserts the decoded AST equals `quote E`. Splitting encode + decode across two exports
/// (the caller threads the bytes back in) is what keeps the codec round-trip from being a single
/// const-foldable expression — the anti-const-fold goal (§2). The exports map to the wit-world members
/// `encode-quoted`/`decode-check` (camelCase → kebab, like the `onMessage`→`on-message` precedent).
///
/// This shape currently DECLINES to compile: a bare `list<u8>`/`Bytes` result as one of MULTIPLE exports
/// is not yet emittable (CDZ0900 "cannot emit that typed export") — an operator-mandated WIT-boundary fix
/// routed to v-rust-backend (the operator: "do not work around compiler bugs; this is how things get
/// fixed"). Until it lands, each case grades Todo — the pass IS the acceptance gate for that fix.
///
/// The scaffolding is built in the PARSER's shapes so it name-resolves once the WIT gap is closed:
/// `Ast.encode`/`Ast.decode` as member-access `(. Ast …)` (a `Name("Ast.encode")` leaf resolves UNBOUND),
/// `false` as a `Leaf::Bool` (not `Name("false")`); both print identically to the wrong forms, so the
/// tests check the member-access spelling + the Bool leaf structurally. `E` is grafted VERBATIM.
fn quote_wrap_program(input_ast: &[u8]) -> Option<Vec<u8>> {
    let e_arena = codec::decode(input_ast)?;
    let mut b = Builder::new();

    // (def (encodeQuoted) ((. Ast encode) (quote E)))
    let quoted_e1 = {
        let e = graft_value(&mut b, &e_arena, e_arena.root);
        form(&mut b, "quote", vec![e])
    };
    let enc_body = {
        let member = ast_member(&mut b, "encode");
        b.list(vec![member, quoted_e1])
    };
    let enc_sig = {
        let n = b.name("encodeQuoted");
        b.list(vec![n])
    };
    let enc_def = form(&mut b, "def", vec![enc_sig, enc_body]);

    // (def (decodeCheck (: bytes Bytes)) (match ((. Ast decode) bytes) ((Ok a) (= a (quote E))) ((Err _) false)))
    let dec_sig = {
        let name = b.name("decodeCheck");
        let param = {
            let bytes = b.name("bytes");
            let ty = b.name("Bytes");
            form(&mut b, ":", vec![bytes, ty])
        };
        b.list(vec![name, param])
    };
    let scrut = {
        let member = ast_member(&mut b, "decode");
        let bytes = b.name("bytes");
        b.list(vec![member, bytes])
    };
    let ok_arm = {
        let pat = {
            let a = b.name("a");
            form(&mut b, "Ok", vec![a])
        };
        let quoted_e2 = {
            let e = graft_value(&mut b, &e_arena, e_arena.root);
            form(&mut b, "quote", vec![e])
        };
        let a_ref = b.name("a");
        let eq = form(&mut b, "=", vec![a_ref, quoted_e2]);
        b.list(vec![pat, eq])
    };
    let err_arm = {
        let pat = {
            let wild = b.name("_");
            form(&mut b, "Err", vec![wild])
        };
        let f = b.atom_leaf(Leaf::Bool(false));
        b.list(vec![pat, f])
    };
    let dec_body = form(&mut b, "match", vec![scrut, ok_arm, err_arm]);
    let dec_def = form(&mut b, "def", vec![dec_sig, dec_body]);

    let exp_enc = {
        let n = b.name("encodeQuoted");
        form(&mut b, "export", vec![n])
    };
    let exp_dec = {
        let n = b.name("decodeCheck");
        form(&mut b, "export", vec![n])
    };

    let root = form(&mut b, "do", vec![enc_def, dec_def, exp_enc, exp_dec]);
    Some(codec::encode(&b.finish(root)))
}

/// The imposed WIT world for the two-export round-trip component (FIXED — independent of `E`), as binary
/// AST: one interface `roundtrip` exporting two typed members with a bare `list<u8>` boundary —
/// ```text
/// (world w (export roundtrip
///   (member encode-quoted (func (result (list (u8)))))
///   (member decode-check  (func (param bytes (list (u8))) (result (bool))))))
/// ```
/// The guest's `encodeQuoted`/`decodeCheck` exports cross under this world (component-name
/// `cadenza:quote/roundtrip`). The `wit-world.ast` artifact's root IS the `(world …)` subtree verbatim —
/// the compiler's native `wit-world:<name>=` input (no wrapper), exactly like the corpus reader's world
/// shred. The bare `list<u8>` member types are the operator-mandated §2 shape (the WIT-gap fix v-rust-
/// backend is closing lets this cross; until then the compile declines → Todo).
fn quote_wrap_wit_world() -> Vec<u8> {
    let mut b = Builder::new();
    // (list (u8)) / (bool)
    let list_u8 = |b: &mut Builder| {
        let u8t = form(b, "u8", vec![]);
        form(b, "list", vec![u8t])
    };
    // (member encode-quoted (func (result (list (u8)))))
    let enc_member = {
        let lu8 = list_u8(&mut b);
        let result = form(&mut b, "result", vec![lu8]);
        let func = form(&mut b, "func", vec![result]);
        let name = b.name("encode-quoted");
        form(&mut b, "member", vec![name, func])
    };
    // (member decode-check (func (param bytes (list (u8))) (result (bool))))
    let dec_member = {
        let lu8 = list_u8(&mut b);
        let bytes = b.name("bytes");
        let param = form(&mut b, "param", vec![bytes, lu8]);
        let bool_ty = form(&mut b, "bool", vec![]);
        let result = form(&mut b, "result", vec![bool_ty]);
        let func = form(&mut b, "func", vec![param, result]);
        let name = b.name("decode-check");
        form(&mut b, "member", vec![name, func])
    };
    // (export roundtrip <enc_member> <dec_member>)
    let export = {
        let iface = b.name("roundtrip");
        form(&mut b, "export", vec![iface, enc_member, dec_member])
    };
    // (world w <export>)
    let root = {
        let w = b.name("w");
        form(&mut b, "world", vec![w, export])
    };
    codec::encode(&b.finish(root))
}

/// The ORACLE-TRIAL artifact — a case's trials as BINARY AST for the Lean oracle. Unlike `test_run_ast`
/// (which stores each value as an opaque string LEAF for a text-reparsing runner), each trial VALUE
/// (arg, expected output, host-response) is PARSED from its value-form text into its binary-AST subtree
/// (`sexpr::read`, grafted) so the oracle reads values as binary AST and never re-parses s-expr text.
/// The expected outcome is carried too (the oracle asserts it internally). Shape:
///   (oracle-trials (trials (trial (call <export>)? (arg <value-ast>)*
///       (expect-value <value-ast> | expect-trap <reason> | expect-error <code>)) …)
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
            Expect::OutputByteLen(n) => {
                let leaf = str_leaf(&mut b, &n.to_string());
                form(&mut b, "expect-output-byte-len", vec![leaf])
            }
            Expect::Trap(reason) => {
                let r = str_leaf(&mut b, reason);
                form(&mut b, "expect-trap", vec![r])
            }
            Expect::Error(code, ..) => {
                let cl = str_leaf(&mut b, code);
                form(&mut b, "expect-error", vec![cl])
            }
            Expect::Warning(code, ..) => {
                let cl = str_leaf(&mut b, code);
                form(&mut b, "expect-warning", vec![cl])
            }
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
        // A byte-len pin is a RUN outcome (compile must succeed, value must escape) → the `output` router.
        Some(Expect::OutputByteLen(_)) => "output",
        Some(Expect::Trap(_)) => "trap",
        Some(Expect::Error(..)) => "error",
        // A warning case COMPILES (must succeed → produce an artifact) AND emits a warning — a COMPILE
        // outcome graded from the diagnostic (grade_compile_warning), distinct from `error` (compile must
        // REFUSE). The exec router handles `warning` as compile-must-succeed + grade-from-diag (no run).
        Some(Expect::Warning(..)) => "warning",
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
        // The DIAGNOSTIC-QUALITY facets as trial-level clauses (siblings of the expect form) — `(fix …)`,
        // `(no-fix)`, `(count N)` — which `cdz_corpus_grade::decode_trial` reads back into a `DiagExpect`.
        // Authored NESTED inside `(error …)`/`(warning …)`, lifted to trial level here.
        if let Some(d) = &t.diag {
            push_diag_forms(&mut b, &mut tk, d);
        }
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
    // `(live-objects <N>)` — a CLEAN case's post-run residual the case asserts (N as a string leaf; N=0 =
    // fully reclaimed). The nix exec runs EVERY heap-importing case on `--runtime <debug-counters>` and
    // cdz-run's `--grade` asserts == N (heap case) / == 0 (absent + heap) / skips (no-heap). A KNOWN-LEAK
    // case (seq-15 pure-binary marker) shreds to `(live-objects "known-leak")` with NO count — it is
    // accepted-as-leaking and NOT count-checked. Absent for a case with no `(live-objects …)`.
    if rec.live_objects_known_leak {
        let leaf = str_leaf(&mut b, "known-leak");
        kids.push(form(&mut b, "live-objects", vec![leaf]));
    } else if let Some(n) = rec.live_objects {
        // Per-call positional CLEAN residuals each become a leaf (`(live-objects "0" "0" "0")`); a uniform
        // residual is the single leaf. `decode_test_run` mirrors this (2+ counts ⇒ per-call, one ⇒ uniform).
        let mut leaves = Vec::new();
        match &rec.live_objects_per_call {
            Some(counts) => {
                for c in counts {
                    leaves.push(str_leaf(&mut b, &c.to_string()));
                }
            }
            None => leaves.push(str_leaf(&mut b, &n.to_string())),
        }
        kids.push(form(&mut b, "live-objects", leaves));
    }
    // `(no-other-errors)` — the case-level no-cascade flag carried verbatim to the grade side (a bare
    // form, no children); `decode_test_run` sets `TestRun::no_other_errors` from it.
    if rec.no_other_errors {
        kids.push(form(&mut b, "no-other-errors", vec![]));
    }
    // `(diagnostic-quality)` — the bare C1 opt-in marker carried verbatim to the grade side (like
    // `(no-other-errors)`); `decode_test_run` reads it into `TestRun::diagnostic_quality`.
    if rec.diagnostic_quality {
        kids.push(form(&mut b, "diagnostic-quality", vec![]));
    }
    // `(no-diagnostic-quality)` — the C1 opt-OUT escape hatch carried to the grade side (suppresses the
    // default-on §1 lint); `decode_test_run` reads it into `TestRun::diagnostic_quality_opt_out`.
    if rec.diagnostic_quality_opt_out {
        kids.push(form(&mut b, "no-diagnostic-quality", vec![]));
    }
    // `(no-diagnostic "phrase")` — each case-level program-scoped absence pin carried verbatim (the phrase
    // as one string leaf); `decode_test_run` collects them into `TestRun::no_diagnostic`. One form per pin.
    for phrase in &rec.no_diagnostic {
        let leaf = str_leaf(&mut b, phrase);
        kids.push(form(&mut b, "no-diagnostic", vec![leaf]));
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
        // `(expect-output-byte-len N)` — the size-only pin; N rides as its decimal-text leaf.
        Expect::OutputByteLen(n) => {
            let leaf = str_leaf(b, &n.to_string());
            form(b, "expect-output-byte-len", vec![leaf])
        }
        Expect::Error(code, msg, not_msg) => {
            let cl = str_leaf(b, code);
            let mut leaves = vec![cl];
            for m in msg {
                leaves.push(str_leaf(b, m));
            }
            // seq-29 message-ABSENCE pins ride as `(not "phrase")` sub-forms — distinguishable from the bare
            // string message-substring leaves so the decoder can partition them (the grade fails if the
            // diagnostic CONTAINS any of them).
            for n in not_msg {
                let nl = str_leaf(b, n);
                leaves.push(form(b, "not", vec![nl]));
            }
            form(b, "expect-error", leaves)
        }
        Expect::Warning(code, msg, not_msg) => {
            let cl = str_leaf(b, code);
            let mut leaves = vec![cl];
            for m in msg {
                leaves.push(str_leaf(b, m));
            }
            for n in not_msg {
                let nl = str_leaf(b, n);
                leaves.push(form(b, "not", vec![nl]));
            }
            form(b, "expect-warning", leaves)
        }
        Expect::Trap(reason) => {
            let leaf = str_leaf(b, reason);
            form(b, "expect-trap", vec![leaf])
        }
    }
}

/// Push a trial's DIAGNOSTIC-QUALITY facets as clause forms onto `tk` (the trial's child list) — `(fix
/// (kind K)? (replacement "r")|(replacement-contains "s")? (verified|unverified)?)`, `(no-fix)`, and
/// `(count N)`. The mirror of `cdz_corpus_grade::decode_trial`'s read of these clauses; `(count 1)` is
/// emitted plainly (the `(once)` shorthand is a parse-time convenience, canonicalized to `count 1`).
fn push_diag_forms(b: &mut Builder, tk: &mut Vec<StructId>, d: &DiagQuality) {
    if let Some(fx) = &d.fix {
        let mut fk: Vec<StructId> = Vec::new();
        if let Some(kind) = &fx.kind {
            let kl = str_leaf(b, kind);
            fk.push(form(b, "kind", vec![kl]));
        }
        match &fx.replacement {
            Some(ReplMatch::Exact(r)) => {
                let rl = str_leaf(b, r);
                fk.push(form(b, "replacement", vec![rl]));
            }
            Some(ReplMatch::Contains(s)) => {
                let sl = str_leaf(b, s);
                fk.push(form(b, "replacement-contains", vec![sl]));
            }
            None => {}
        }
        match fx.verified {
            Some(true) => fk.push(form(b, "verified", vec![])),
            Some(false) => fk.push(form(b, "unverified", vec![])),
            None => {}
        }
        tk.push(form(b, "fix", fk));
    }
    if d.no_fix {
        tk.push(form(b, "no-fix", vec![]));
    }
    if let Some(n) = d.count {
        let nl = str_leaf(b, &n.to_string());
        tk.push(form(b, "count", vec![nl]));
    }
    // `(exact-code)` — lift the nested C1 fence flag to a trial-level marker the grade side reads into
    // `GTrial.exact_code` (→ `grade_compile_error(exact=true)`: a wrong/uncoded code FAILs).
    if d.exact_code {
        tk.push(form(b, "exact-code", vec![]));
    }
}

/// A member-access `(. Ast <member>)` node — the shape the parser produces for a dotted reference like
/// `Ast.encode` / `Ast.decode` (a bare `Name("Ast.encode")` leaf would resolve UNBOUND). Applied to an
/// argument by wrapping in a list: `((. Ast encode) x)`.
fn ast_member(b: &mut Builder, member: &str) -> StructId {
    let ast = b.name("Ast");
    let m = b.name(member);
    form(b, ".", vec![ast, m])
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

    /// The `; nativize-allow-classic[: reason]` per-file exemption directive: detected on a comment line
    /// (with or without a reason), returns the trimmed reason (or ""); absent / non-directive → None. This
    /// is the interim escape hatch exempting a file with an intentional classic-head CTOR-APPLICATION (e.g.
    /// 19-sets's #7969 `(set …)`) from the nativize idempotence assertion.
    #[test]
    fn nativize_allow_directive_detects_the_per_file_exemption() {
        assert_eq!(
            nativize_allow_directive(
                "; nativize-allow-classic: (set …) ctor #7969\n(case \"x\" …)"
            ),
            Some("(set …) ctor #7969".to_string())
        );
        // Bare directive (no reason) → empty string.
        assert_eq!(
            nativize_allow_directive(";   nativize-allow-classic\n; more"),
            Some(String::new())
        );
        // Not present → None; a plain comment mentioning the word elsewhere on a non-directive line is fine.
        assert_eq!(
            nativize_allow_directive("; Sets — a collection\n(case …)"),
            None
        );
        // Must be a `;`-comment-anchored directive, not a bare substring in prose.
        assert_eq!(
            nativize_allow_directive("; this file talks about nativize-allow-classic informally"),
            None
        );
    }

    #[test]
    fn scan_live_objects_edits_flags_clause_changes_not_output_or_context() {
        let diff = "\
diff --git a/spec/semantics/28-wit-abi-boundary.sexp b/spec/semantics/28-wit-abi-boundary.sexp
+++ b/spec/semantics/28-wit-abi-boundary.sexp
@@ -1231,1 +1231,1 @@
-  (live-objects known-leak 1))
+  (live-objects 0))
+  (output #record((= a 1)))
diff --git a/spec/semantics/19-sets.sexp b/spec/semantics/19-sets.sexp
+++ b/spec/semantics/19-sets.sexp
@@ -1,1 +1,1 @@
-  (live-objects 0))
+  (live-objects known-leak 12))
   (live-objects 3))
";
        let hits = scan_live_objects_edits(diff);
        // FOUR live-objects EDIT lines (- old + + new in each of file 28 and file 19); the `(output …)`
        // change and the unchanged-context `(live-objects 3)` line (leading space, no +/-) are NOT flagged.
        assert_eq!(hits.len(), 4);
        assert!(
            hits.iter()
                .any(|(f, l)| f.ends_with("28-wit-abi-boundary.sexp")
                    && l == "(live-objects known-leak 1))")
        );
        assert!(
            hits.iter()
                .any(|(f, l)| f.ends_with("28-wit-abi-boundary.sexp") && l == "(live-objects 0))")
        );
        assert!(
            hits.iter()
                .any(|(f, l)| f.ends_with("19-sets.sexp") && l == "(live-objects 0))")
        );
        assert!(
            hits.iter()
                .any(|(f, l)| f.ends_with("19-sets.sexp") && l == "(live-objects known-leak 12))")
        );
        assert!(hits.iter().all(|(_, l)| !l.contains("(output")));
    }

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

    /// `vanished_across` (the pre-commit baseline-guard primitive) returns the TOTAL vanished-title count
    /// across N baselines (0 = clean, >0 = detected → the caller maps to exit 3), and `Err` only on a tooling
    /// error (unreadable file → exit 2, fail-open). This is the #7176/#6835 contamination guard.
    #[test]
    fn vanished_across_counts_stale_titles_and_errs_only_on_io() {
        let dir = std::env::temp_dir().join(format!("vanished-check-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p.to_str().unwrap().to_string()
        };
        // Corpus has cases "alpha" + "beta".
        let corpus = w(
            "corpus.sexp",
            "(case \"alpha\" (input 1) (output (: 1 Int64)))\n\
             (case \"beta\" (input 2) (output (: 2 Int64)))\n",
        );
        // Clean baseline: only current corpus titles.
        let clean = w("clean.gate-baseline", "pass\talpha\ntodo\tbeta\n");
        // Stale baseline: carries "gamma" which no corpus case matches → vanished.
        let stale = w("stale.gate-baseline", "pass\talpha\npass\tgamma\n");

        let corpus_files = [corpus];
        // All-clean → Ok(0).
        assert_eq!(
            vanished_across(std::slice::from_ref(&clean), &corpus_files, false, false).unwrap(),
            0
        );
        // A mix → Ok(count) counting only the stale baseline's vanished title ("gamma").
        assert_eq!(
            vanished_across(&[clean, stale.clone()], &corpus_files, true, false).unwrap(),
            1
        );
        // A single stale baseline → Ok(1) (vanished is NOT an Err — the caller maps count>0 to exit 3).
        assert_eq!(
            vanished_across(std::slice::from_ref(&stale), &corpus_files, false, false).unwrap(),
            1
        );
        // A tooling error (nonexistent baseline file) → Err (the caller maps to exit 2, fail-open).
        let missing = dir.join("does-not-exist.gate-baseline");
        assert!(
            vanished_across(
                &[missing.to_str().unwrap().to_string()],
                &corpus_files,
                false,
                false
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `prune_baseline` (the pure core of `--prune`) DELETES orphan-vanished entry lines and preserves
    /// everything else BYTE-for-byte: the `#` header, the blank line, a live entry, and the trailing
    /// newline all survive; only the `gamma` orphan (no corpus case) is removed and reported.
    #[test]
    fn prune_baseline_drops_only_orphan_entries_preserving_layout() {
        let corpus: std::collections::BTreeSet<String> =
            ["alpha", "beta"].iter().map(|s| s.to_string()).collect();
        let text = "# gate baseline — verdict\\tdescription\n\
                    pass\talpha\n\
                    \n\
                    pass\tgamma\n\
                    todo\tbeta\n";
        let (out, removed) = prune_baseline(text, &corpus);
        assert_eq!(removed, vec!["gamma".to_string()]);
        assert_eq!(
            out,
            "# gate baseline — verdict\\tdescription\n\
             pass\talpha\n\
             \n\
             todo\tbeta\n"
        );
    }

    /// A baseline with no orphan is returned UNCHANGED (byte-identical), and a file with NO trailing
    /// newline keeps that shape after a prune (no spurious newline appended).
    #[test]
    fn prune_baseline_is_a_noop_when_clean_and_preserves_no_trailing_newline() {
        let corpus: std::collections::BTreeSet<String> =
            ["alpha", "beta"].iter().map(|s| s.to_string()).collect();

        let clean = "pass\talpha\ntodo\tbeta\n";
        let (out, removed) = prune_baseline(clean, &corpus);
        assert!(removed.is_empty());
        assert_eq!(out, clean);

        // No trailing newline + one orphan to drop: the surviving line keeps its no-newline shape.
        let no_nl = "pass\talpha\npass\tgamma";
        let (out, removed) = prune_baseline(no_nl, &corpus);
        assert_eq!(removed, vec!["gamma".to_string()]);
        assert_eq!(out, "pass\talpha");
    }

    /// `--prune` via `vanished_across`: an empty corpus set is REFUSED (fail-open, `Err` → exit 2) so a
    /// bad glob can never wipe a baseline; a real prune rewrites the file in place and returns Ok(0).
    #[test]
    fn vanished_across_prune_fails_open_on_empty_corpus_and_rewrites_in_place() {
        let dir = std::env::temp_dir().join(format!("vanished-prune-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p.to_str().unwrap().to_string()
        };
        let corpus = w(
            "corpus.sexp",
            "(case \"alpha\" (input 1) (output (: 1 Int64)))\n",
        );
        let empty_corpus = w("empty.sexp", "; no cases here\n");
        let bl = w("b.gate-baseline", "pass\talpha\npass\tgamma\n");

        // FAIL-OPEN: an empty corpus title set refuses the prune (and leaves the file untouched).
        assert!(
            vanished_across(
                std::slice::from_ref(&bl),
                std::slice::from_ref(&empty_corpus),
                true,
                true
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&bl).unwrap(),
            "pass\talpha\npass\tgamma\n",
            "a refused prune must not modify the file"
        );

        // Real prune: rewrites the file dropping the orphan, returns Ok(0) (the drift is FIXED, not blocked).
        assert_eq!(
            vanished_across(
                std::slice::from_ref(&bl),
                std::slice::from_ref(&corpus),
                true,
                true
            )
            .unwrap(),
            0
        );
        assert_eq!(std::fs::read_to_string(&bl).unwrap(), "pass\talpha\n");
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn capability_error_check_flags_error_cdz0900_only() {
        // (error CDZ0900) = the anti-pattern (a not-yet-built umbrella pinned as an ill-formed REJECTION) → FLAG.
        // (error CDZ0201) = a genuine semantic spec-error → NOT flagged (that IS the spec).
        // (output …) for a should-work case → NOT flagged (grades Todo now, auto-Pass when implemented).
        let recs = crate::read(
            r#"(case "wrongly pins the not-yet umbrella as an error" (input 1_) (error CDZ0900))
               (case "a genuine malformed spec error" (input 1_) (error CDZ0201))
               (case "should-work recorded as output" (input (do (def (main) 0) (export main))) (output (: 0 Int64)))"#,
        )
        .unwrap();
        let hits = capability_error_hits(&recs);
        assert_eq!(
            hits.len(),
            1,
            "only the (error CDZ0900) case is flagged: {hits:?}"
        );
        assert_eq!(hits[0].0, "wrongly pins the not-yet umbrella as an error");
        assert_eq!(hits[0].1, "CDZ0900");
    }

    #[test]
    fn not_message_reaches_shredded_test_run() {
        let recs = crate::read(
            r#"(case "err" (input 1_) (error CDZ0201 (message "malformed") (not "internal error")))"#,
        )
        .unwrap();
        let err_tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            err_tr.contains(r#"(not "internal error")"#) && err_tr.contains(r#""malformed""#),
            "expect-error carries both the message and the (not …) absence pin: {err_tr}"
        );
    }

    /// A CASE-LEVEL `(no-diagnostic "phrase")` (program-scoped cross-kind absence) is PARSED onto the Record
    /// and SHREDDED verbatim into `test-run.ast` — one form per pin, repeatable — exactly what
    /// `cdz_corpus_grade::decode_test_run` collects into `TestRun::no_diagnostic`. Distinct from a trial's
    /// `(not …)`; this rides alongside the trials at the case level (like `(no-other-errors)`).
    #[test]
    fn no_diagnostic_reaches_shredded_test_run() {
        let recs = crate::read(
            r#"(case "cross" (input 1_)
                 (error CDZ0201)
                 (no-diagnostic "needs a heap walk")
                 (no-diagnostic "unused binding"))"#,
        )
        .unwrap();
        assert_eq!(
            recs[0].no_diagnostic,
            vec![
                "needs a heap walk".to_string(),
                "unused binding".to_string()
            ],
            "both (no-diagnostic …) pins parse onto the Record in order"
        );
        let tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            tr.contains(r#"(no-diagnostic "needs a heap walk")"#)
                && tr.contains(r#"(no-diagnostic "unused binding")"#),
            "both pins shred into the test-run as their own forms: {tr}"
        );
    }

    /// A bare `(no-diagnostic-quality)` marker (the C1 opt-OUT hatch, post default-flip) parses onto the
    /// Record (case-level AND file-level) and shreds into the test-run — read into
    /// `TestRun::diagnostic_quality_opt_out`. Absence leaves it off (the norm: §1-enforced by default).
    #[test]
    fn no_diagnostic_quality_opt_out_marker_reaches_shredded_test_run() {
        // Case-level.
        let recs = crate::read(r#"(case "q" (input 1_) (error CDZ0201) (no-diagnostic-quality))"#)
            .unwrap();
        assert!(
            recs[0].diagnostic_quality_opt_out,
            "case-level opt-out parses"
        );
        let tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            tr.contains("(no-diagnostic-quality)"),
            "opt-out shreds into the test-run: {tr}"
        );
        // File-level (a bare top-level form) opts out EVERY case in the file.
        let fl = crate::read(
            r#"(no-diagnostic-quality)
               (case "a" (input 1_) (error CDZ0201))
               (case "b" (input 2_) (error CDZ0201))"#,
        )
        .unwrap();
        assert!(
            fl.iter().all(|r| r.diagnostic_quality_opt_out),
            "file-level opt-out enrolls every case"
        );
        // Absent → off (the norm: default §1-enforced).
        let plain = crate::read(r#"(case "p" (input 1_) (error CDZ0201))"#).unwrap();
        assert!(!plain[0].diagnostic_quality_opt_out);
    }

    /// A bare `(diagnostic-quality)` marker (the C1 opt-in) parses onto the Record and shreds into the
    /// test-run as its own form — exactly what `cdz_corpus_grade::decode_test_run` reads into
    /// `TestRun::diagnostic_quality`. Its ABSENCE leaves the flag off (opt-in, no accidental enrollment).
    #[test]
    fn diagnostic_quality_marker_reaches_shredded_test_run() {
        let recs =
            crate::read(r#"(case "q" (input 1_) (error CDZ0201) (diagnostic-quality))"#).unwrap();
        assert!(
            recs[0].diagnostic_quality,
            "the marker parses onto the Record"
        );
        let tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            tr.contains("(diagnostic-quality)"),
            "the marker shreds into the test-run as its own form: {tr}"
        );
        // Absent → off (opt-in).
        let plain = crate::read(r#"(case "p" (input 1_) (error CDZ0201))"#).unwrap();
        assert!(!plain[0].diagnostic_quality);
    }

    /// The DIAGNOSTIC-QUALITY facets + the `(warning …)` result kind reach the shredded `test-run.ast` as
    /// trial-level clauses — exactly the wire `cdz_corpus_grade::decode_trial` reads. The two crates share
    /// the sexp wire (no type dep), so this pins the emitter half of that contract.
    #[test]
    fn diag_quality_facets_reach_the_shredded_test_run() {
        let recs = crate::read(
            r#"(case "fix" (input 1_)
                 (error CDZ0201 (fix (kind replace) (replacement "1") (verified)) (count 2)))
               (case "warn" (input (do (def (main) 0) (export main)))
                 (warning CDZ0305 (message "dead") (no-fix)))"#,
        )
        .unwrap();
        let fix_tr = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(fix_tr.contains("expect-error"), "error kind: {fix_tr}");
        assert!(fix_tr.contains("(fix"), "fix clause: {fix_tr}");
        assert!(
            fix_tr.contains("replacement") && fix_tr.contains("verified"),
            "fix facets: {fix_tr}"
        );
        assert!(
            fix_tr.contains("(count") && fix_tr.contains('2'),
            "count: {fix_tr}"
        );

        let warn_tr = sexpr::print(&codec::decode(&test_run_ast(&recs[1])).unwrap());
        assert!(
            warn_tr.contains("expect-warning") && warn_tr.contains("CDZ0305"),
            "warning: {warn_tr}"
        );
        assert!(warn_tr.contains("no-fix"), "no-fix clause: {warn_tr}");
        // The warning case ROUTES as its own kind for the exec router.
        assert_eq!(expect_kind(&recs[1]), "warning");
    }

    /// C1 fence: a `(error CODE (exact-code))` case lifts the nested `(exact-code)` to a trial-level
    /// `(exact-code)` clause in the shredded `test-run.ast` — which `cdz_corpus_grade::decode_trial` reads
    /// into `GTrial.exact_code` (→ `grade_compile_error(exact=true)`: a wrong/uncoded code FAILs). A case
    /// WITHOUT it stays clause-free (the default lenient wrong-code→Todo).
    #[test]
    fn exact_code_fence_reaches_the_shredded_test_run() {
        let recs = crate::read(
            r#"(case "fenced" (input 1_) (error CDZ0201 (exact-code)))
               (case "lenient" (input 1_) (error CDZ0201))"#,
        )
        .unwrap();
        let fenced = sexpr::print(&codec::decode(&test_run_ast(&recs[0])).unwrap());
        assert!(
            fenced.contains("(exact-code)"),
            "exact-code lifts to a trial-level clause: {fenced}"
        );
        let lenient = sexpr::print(&codec::decode(&test_run_ast(&recs[1])).unwrap());
        assert!(
            !lenient.contains("exact-code"),
            "a case without (exact-code) stays fence-free: {lenient}"
        );
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
        // seq-15 PURE-BINARY: a `(live-objects known-leak)` marker shreds to a count-free `(live-objects
        // "known-leak")` — the leak magnitude is not carried (not count-checked). A legacy count-bearing
        // marker shreds identically (the count is dropped).
        let leak = crate::read(
            r#"(case "leak"
                 (input (do (type L (Cons (Tuple Int64 L)) Nil) (def (main) (L.Cons (tuple 1 (L.Nil ())))) (export main)))
                 (call main) (output (: (L.Cons (tuple 1 (L.Nil ()))) L))
                 (live-objects known-leak 2))"#,
        )
        .unwrap();
        let leak_tr = sexpr::print(&codec::decode(&test_run_ast(&leak[0])).unwrap());
        assert!(
            leak_tr.contains("(live-objects \"known-leak\")"),
            "known-leak marker shreds count-free: {leak_tr}"
        );
        assert!(
            !leak_tr.contains("\"known-leak\" \"2\""),
            "the count is dropped: {leak_tr}"
        );
        // A CLEAN PER-CALL positional clause shreds to one leaf per count (`(live-objects "0" "0" "0")`), so
        // the nix GRADE path (which reads this AST) balances EACH call against its own residual.
        let percall = crate::read(
            r#"(case "percall"
                 (input (do (def (main (: r Int64)) r) (export main)))
                 (call main (: 1 Int64)) (output (: 1 Int64))
                 (call main (: 4 Int64)) (output (: 4 Int64))
                 (call main (: 0 Int64)) (output (: 0 Int64))
                 (live-objects 0 0 0))"#,
        )
        .unwrap();
        let pc_tr = sexpr::print(&codec::decode(&test_run_ast(&percall[0])).unwrap());
        assert!(
            pc_tr.contains("(live-objects \"0\" \"0\" \"0\")"),
            "per-call positional counts shred as one leaf each: {pc_tr}"
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

    /// `input_ast` carries the RAW `(input …)` form VERBATIM — NOT the normalized `(do (def (main) …)
    /// (export main))` program. A bare scalar input `42` shreds `input_ast` as the lone `42` leaf (the
    /// form `--quote-wrap` reifies), while `program_ast` is the wrapped runnable program.
    #[test]
    fn input_ast_is_the_raw_input_form_not_the_normalized_program() {
        let recs = crate::read(r#"(case "scalar" (input 42) (output (: 42 Int64)))"#).unwrap();
        let raw = sexpr::print(&codec::decode(&recs[0].input_ast).expect("input_ast decodes"));
        assert_eq!(raw.trim(), "42", "input_ast is the raw input form: {raw}");
        // The normalized program is the wrapped runnable form — DISTINCT from the raw input.
        let prog = sexpr::print(&codec::decode(&recs[0].program_ast).unwrap());
        assert!(
            prog.contains("(def (main)") && prog.contains("(export main)"),
            "program_ast is the normalized runnable program: {prog}"
        );
    }

    /// Whether the AST rooted at `id` in `a` contains a `Leaf::Bool(false)` atom — the STRUCTURAL check
    /// that the `((Err _) false)` arm carries a real boolean leaf, NOT a `Name("false")` (which resolves
    /// UNBOUND at compile). Both PRINT as `false`, so a text assertion cannot tell them apart.
    fn contains_bool_false(a: &Arenas, id: StructId) -> bool {
        match a.get(id) {
            cadenza_syntax::ast::Struct::Atom(lid) => {
                matches!(a.leaf(*lid), Leaf::Bool(false))
            }
            cadenza_syntax::ast::Struct::List(kids) => {
                kids.iter().any(|&k| contains_bool_false(a, k))
            }
        }
    }

    /// `quote_wrap_program` synthesizes the §2 ONE-component TWO-export round-trip program for a SCALAR
    /// input: exports `encodeQuoted`/`decodeCheck`, quotes the raw input TWICE (encode side + the
    /// equality check), and builds the scaffolding in the PARSER's shapes so it name-resolves once the WIT
    /// gap is closed: `Ast.encode`/`Ast.decode` as member-access `(. Ast …)` (a `Name("Ast.encode")` leaf
    /// resolves unbound), `false` as a `Leaf::Bool` (not `Name("false")`). Both wrong forms print
    /// identically, so these check the member-access SPELLING + the Bool leaf STRUCTURALLY.
    #[test]
    fn quote_wrap_program_synthesizes_two_export_round_trip() {
        let recs = crate::read(r#"(case "scalar" (input 42) (output (: 42 Int64)))"#).unwrap();
        let prog = quote_wrap_program(&recs[0].input_ast).expect("synthesizes");
        let arena = codec::decode(&prog).expect("program.ast decodes");
        let text = sexpr::print(&arena);
        // Two exports in ONE program.
        assert!(
            text.contains("(export encodeQuoted)") && text.contains("(export decodeCheck)"),
            "both exports present in one program: {text}"
        );
        // MEMBER-ACCESS spelling `(. Ast encode)` / `(. Ast decode)`.
        assert!(
            text.contains("((. Ast encode) (quote 42))"),
            "encode is a member-access applied to (quote E): {text}"
        );
        assert!(
            text.contains("((. Ast decode) bytes)"),
            "decode is a member-access applied to the Bytes param: {text}"
        );
        assert!(
            text.contains("(: bytes Bytes)"),
            "decodeCheck takes a Bytes param: {text}"
        );
        // Quoted TWICE (encode side + equality check).
        assert_eq!(
            text.matches("(quote 42)").count(),
            2,
            "quotes the raw input on both the encode and equality sides: {text}"
        );
        // The `((Err _) false)` arm carries a genuine Bool leaf, not a `Name("false")`.
        assert!(
            contains_bool_false(&arena, arena.root),
            "the Err arm's `false` is a Leaf::Bool, not a Name: {text}"
        );
    }

    /// `quote_wrap_wit_world` synthesizes the imposed world declaring the two typed exports with a bare
    /// `list<u8>` boundary: interface `roundtrip` with members `encode-quoted` (`func() -> list<u8>`) and
    /// `decode-check` (`func(list<u8>) -> bool`). Root IS the `(world …)` subtree verbatim (no wrapper),
    /// the compiler's native `wit-world:` input shape. FIXED — independent of E.
    #[test]
    fn quote_wrap_wit_world_declares_the_two_typed_exports() {
        let world = quote_wrap_wit_world();
        let text = sexpr::print(&codec::decode(&world).expect("wit-world.ast decodes"));
        assert!(
            text.starts_with("(world w ") && text.contains("(export roundtrip"),
            "root is the (world …) subtree, exporting the roundtrip interface: {text}"
        );
        assert!(
            text.contains("(member encode-quoted (func (result (list (u8)))))"),
            "encode-quoted member: () -> list<u8>: {text}"
        );
        assert!(
            text.contains("(member decode-check (func (param bytes (list (u8))) (result (bool))))"),
            "decode-check member: (list<u8>) -> bool: {text}"
        );
    }

    /// An ARITHMETIC form input quotes as a COMPOUND `(quote (+ 1 2))` (twice) — the reifier's compound
    /// path (synthesized front-end; whether `quote` reifies it is a downstream compile property).
    #[test]
    fn quote_wrap_synthesizes_for_an_arithmetic_form() {
        let recs = crate::read(r#"(case "arith" (input (+ 1 2)) (output (: 3 Int64)))"#).unwrap();
        let text =
            sexpr::print(&codec::decode(&quote_wrap_program(&recs[0].input_ast).unwrap()).unwrap());
        assert_eq!(
            text.matches("(quote (+ 1 2))").count(),
            2,
            "quotes the compound arithmetic form on both sides: {text}"
        );
    }

    /// A COLLECTION-LITERAL input is STILL synthesized (eligibility is syntactic — the program emits
    /// `(quote #list(…))`); `quote` currently DECLINES on a collection literal, so at compile the case
    /// grades Todo, NOT Fail (doc §3 clause 2 / §5). The synthesis itself never declines — it just wraps.
    #[test]
    fn quote_wrap_synthesizes_for_a_collection_literal_that_will_decline() {
        let recs = crate::read(
            r#"(case "coll" (input #list(1 2 3)) (output (: #list(1 2 3) (List Int64))))"#,
        )
        .unwrap();
        let text =
            sexpr::print(&codec::decode(&quote_wrap_program(&recs[0].input_ast).unwrap()).unwrap());
        assert!(
            text.contains("(quote #list(1 2 3))"),
            "the collection literal is quoted verbatim (declines downstream, graded Todo): {text}"
        );
    }

    /// A FULL-PROGRAM input `(do (def (main) …) (export main))` is quoted AS A WHOLE — the raw input form,
    /// exports and all, is what `E` is (the pass quotes the syntax, not a normalized wrapper).
    #[test]
    fn quote_wrap_quotes_a_full_program_input_verbatim() {
        let recs = crate::read(
            r#"(case "prog" (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                 (call main 41) (output (: 42 Int64)))"#,
        )
        .unwrap();
        let text =
            sexpr::print(&codec::decode(&quote_wrap_program(&recs[0].input_ast).unwrap()).unwrap());
        assert!(
            text.contains("(quote (do (def (main (: x Int64)) (+ x 1)) (export main)))"),
            "the full program input is quoted verbatim: {text}"
        );
    }

    /// Eligibility: a single-component case is eligible regardless of its expect-kind (value / error /
    /// trap), while a sibling-MODULE package case and a cross-component PEER case are NOT (multi-form
    /// wrapping is a later increment).
    #[test]
    fn quote_wrap_eligibility_covers_single_component_cases_only() {
        let single = crate::read(
            r#"(case "val" (input 42) (output (: 42 Int64)))
               (case "err" (input 1_) (error CDZ0201 (message "separator")))"#,
        )
        .unwrap();
        assert!(
            single.iter().all(quote_wrap_eligible),
            "value + error single-component cases are eligible"
        );
        // A multi-file PACKAGE case (sibling module) — NOT eligible in v1.
        let pkg = crate::read(
            r#"(case "pkg" (module "lib" (do (def (answer) 42) (export answer)))
                 (input (do (import "lib" (answer)) (def (main) (answer)) (export main)))
                 (call main) (output (: 42 Int64)))"#,
        )
        .unwrap();
        assert!(
            !quote_wrap_eligible(&pkg[0]),
            "a sibling-module package case is ineligible in v1 (multi-form)"
        );
    }

    /// The end-to-end shred: `shred_quote_wrap` writes one dir PER ELIGIBLE case (`program.ast` +
    /// `wit-world.ast` + `component-name` + `description`) and a `manifest` listing them in order, KEEPING
    /// the base-corpus `NNNN-slug` index and SKIPPING an ineligible package case (a gap where it sat).
    #[test]
    fn shred_quote_wrap_emits_eligible_dirs_and_manifest() {
        let recs = crate::read(
            r#"(case "scalar" (input 42) (output (: 42 Int64)))
               (case "pkg" (module "lib" (do (def (answer) 42) (export answer)))
                 (input (do (import "lib" (answer)) (def (main) (answer)) (export main)))
                 (call main) (output (: 42 Int64)))
               (case "arith" (input (+ 1 2)) (output (: 3 Int64)))"#,
        )
        .unwrap();
        let tmp = std::env::temp_dir().join(format!("qwshred-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        shred_quote_wrap(&tmp, &recs).expect("shred");
        let manifest = std::fs::read_to_string(tmp.join("manifest")).unwrap();
        let cases: Vec<&str> = manifest.lines().collect();
        // Two eligible cases (scalar #0, arith #2); the package case #1 is skipped.
        assert_eq!(cases.len(), 2, "manifest: {manifest:?}");
        assert!(
            cases[0].starts_with("0000-"),
            "scalar keeps index 0: {cases:?}"
        );
        assert!(
            cases[1].starts_with("0002-"),
            "arith keeps its base index 2 (package #1 skipped): {cases:?}"
        );
        // The emitted program.ast decodes and is the two-export round-trip program.
        let prog = sexpr::print(
            &codec::decode(&std::fs::read(tmp.join(cases[0]).join("program.ast")).unwrap())
                .unwrap(),
        );
        assert!(
            prog.contains("(export encodeQuoted)")
                && prog.contains("(export decodeCheck)")
                && prog.contains("((. Ast encode) (quote 42))")
                && prog.contains("((. Ast decode) bytes)"),
            "case 0 program.ast is the two-export round-trip program: {prog}"
        );
        // The imposed wit-world + component-name are emitted alongside.
        let world = sexpr::print(
            &codec::decode(&std::fs::read(tmp.join(cases[0]).join("wit-world.ast")).unwrap())
                .unwrap(),
        );
        assert!(
            world.contains("(export roundtrip") && world.contains("(member encode-quoted"),
            "case 0 wit-world.ast declares the roundtrip interface: {world}"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.join(cases[0]).join("component-name")).unwrap(),
            "cadenza:quote/roundtrip"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.join(cases[0]).join("description")).unwrap(),
            "scalar"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}

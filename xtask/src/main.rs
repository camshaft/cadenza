//! xtask — the one interface for driving the workspace.
//!
//! Deliberately minimal: today it builds the value-heap runtime component, content-addresses it,
//! and stores it. The runtime is derived FIRST and keyed by its SHA-256 (the recorded hashing
//! choice, options/hashing-and-encoding/), which is what the host resolves a program's required
//! runtime against (reproducible-derivation.md §Derivation Is A Function Of Source And Toolchain).
//!
//! The source-generation choreography (WIT envelope, opcode/frame tables) that used to live here
//! was stripped out — its outputs are now frozen, hand-maintained sources in the seed crates. Recover
//! the generators from git history if we decide to re-derive them.
//!
//! Every command is parsed with clap — typed subcommands, generated `--help`, and an error on an
//! unknown subcommand/flag. xtask pulls in NO workspace crate as a library; it choreographs the
//! built binaries (cdz-syntax / rcdzc / cdz-run) so every command exercises the real tools.
//!
//! Commands:
//!   setup       one-time: symlink .claude/{skills,commands} → the tracked skills/ + commands/
//!   build       build the value-heap runtime component + content-address it into the store
//!   run         compile-and-run one program end-to-end (surface → AST → component → result)
//!   emit        the compile-only half of `run` — write the component, don't run it
//!   gate        run the corpus and grade each case (--case, --save, --check baseline)
//!   check       omnibus health check: fmt + build + test + clippy(-D warnings) + wasm-runtime + gate
//!   roundtrip   every corpus program round-trips through the syntax surfaces
//!   fmt         format program file(s) through the printer (--check for CI)
//!
//! Global `--profile <name>` chooses the cargo profile the pipeline tools are built under; it
//! defaults to `release-debug` (optimized, so the corpus gate is fast). Pass `--profile dev` for a
//! quick unoptimized build when iterating on the tools themselves.

use cdz_corpus_grade::{TrapCode, canonical_output_value, classify, is_ice_signature};
use cdz_rust_render::*;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};
use xtask_support::{
    Call, CorpusRecord, Verdict, compare_verdicts_baseline, content_address, default_corpus_files,
    first_line, hash_tree, launch_fail, read_corpus, serialize_baseline, split_message_clause,
};

/// The one interface for driving the Cadenza seed workspace. Every knob is a typed flag; there are
/// no environment-variable knobs.
#[derive(Parser)]
#[command(
    name = "xtask",
    about = "The one interface for driving the Cadenza workspace."
)]
struct Cli {
    /// The cargo profile the pipeline tools (cdz-syntax / rcdzc / cdz-run) are built under. Defaults
    /// to `release-debug` (optimized, so the corpus gate is fast); pass `--profile dev` for a quick
    /// unoptimized build when iterating on the tools themselves.
    #[arg(long, global = true, default_value = "release-debug")]
    profile: String,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the value-heap runtime component, content-address it, and store it under `--store`.
    Build {
        /// Content-addressed store directory. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Compile a Cadenza program and run it, printing the result — the whole pipeline end-to-end:
    /// surface → binary AST (cadenza-syntax) → component (rcdzc) → run (cdz-run).
    Run {
        /// The Cadenza program file to compile and run.
        file: PathBuf,
        /// The input surface. Defaults to `sexpr` (what `.cdz`/`.sexp` files carry).
        #[arg(long, default_value = "sexpr")]
        from: String,
        /// Content-addressed store the runtime is resolved from. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Run one or more corpus files: compile+run each case through the pipeline and compare the
    /// result against the recorded outcome. Reports pass / todo (a case the compiler can't yet
    /// handle) / fail (a real disagreement). Exits non-zero only on a fail.
    Gate {
        /// The corpus `.sexp` files to run. [default: all of spec/semantics/*.sexp]
        files: Vec<PathBuf>,
        /// Content-addressed store the runtime is resolved from. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
        /// Run only cases whose description contains this substring, printing each one's normalized
        /// program, expected result, and actual outcome — the single-case debug loop.
        #[arg(long)]
        case: Option<String>,
        /// Save the current per-case verdicts as the committed baseline, then exit.
        #[arg(long, conflicts_with = "check")]
        save: bool,
        /// Compare the current verdicts to the baseline; fail on any case that REGRESSED
        /// (pass→not-pass), even while totals shift. Reports regressions and newly-passing cases.
        #[arg(long)]
        check: bool,
        /// Which backend to drive each case through: `wasm` (default — the historical cdz-run path) or
        /// `rust` (emit `--target rust`, compile with `rustc`, run). The Rust path grades the SAME
        /// corpus expectations against the Rust backend, catching a non-compiling artifact or a wrong
        /// answer. Slower (a `rustc` invocation per case), so it is opt-in and has its own baseline.
        #[arg(long, default_value = "wasm")]
        target: GateTargetArg,
        /// OPTIMIZATION-LEVEL-EQUIVALENCE sweep: compile AND RUN each corpus program at O0/O1/O2/O3 and
        /// assert the OBSERVABLE RUN OUTCOME (value / trap-kind / decline-code) is equivalent across all
        /// four (the tiered-opt invariant — every level is observably identical). NOT a byte diff: the
        /// wasm emit is not byte-deterministic run-to-run, so the observable outcome is the real invariant.
        /// A level that changes the outcome is a candidate miscompile (hard fail). Honors `--target`:
        /// `wasm` (default) sweeps the wasm pipeline, `rust`/`rust-async` the rustc pipeline (each level
        /// threaded through as `--opt-level`). Ignores `--save`/`--check` (no baseline — a same-run
        /// cross-level diff).
        #[arg(long, conflicts_with_all = ["save", "check"])]
        opt_sweep: bool,
        /// Run only shard `I` of `N` (1-based), format `I/N`: partition the corpus files deterministically
        /// (round-robin over the sorted file list) into `N` groups and run only group `I`. Splits the
        /// long full-corpus gate into parallel CI jobs each short enough to finish before a runner reclaim
        /// (the nightly rust gate). With `--check`, the baseline compare is SCOPED to the cases this shard
        /// runs — it flags regressions among them but does NOT treat the other shards' baseline cases as
        /// "vanished" (they are simply in another shard). Rejected with `--save` (a shard would clobber the
        /// baseline with a partial snapshot).
        #[arg(long, conflicts_with_all = ["save", "opt_sweep"])]
        shard: Option<String>,
        /// GUARDED-ALL: route EVERY case through the debug-counters runtime (not just the
        /// `(live-objects …)`-clause cases) so its generation guard fires corpus-wide — the deterministic
        /// under-retain / use-after-free witness for verifying a global escape-analysis / RC change. On
        /// free the debug runtime bumps the node generation odd + retains the freed shell, so a later
        /// access of a freed cell TRAPS (→ a fail); the shipped/release runtime deallocates with no guard
        /// (a UAF there is silent UB, and the release live-objects census reports 0 vacuously = a
        /// false-clean). Forces the IN-PROCESS path (like `--check`) and FAIL-FASTs at entry if the store's
        /// debug runtime is missing or stale (never a silent false-clean). Combine with `--check` for a
        /// guarded regression run. Prefer this over any "release-trap" run for memory-safety verification.
        #[arg(long)]
        guarded_all: bool,
    },
    /// The parser/printer golden-corpus grader (DESIGN-parser-test-corpus.md §4): grade each
    /// `spec/syntax/<surface>/<case>/` directory against the reference `cdz` tool — the case's structural
    /// parse tree (`cdz convert --to sexpr --structural`) vs `tree.sexp`, and its canonical format
    /// (`cdz fmt --stdout`) vs `format.<ext>`-or-`input`. Additive `pass`/`todo`/`fail` verdicts compared
    /// against `spec/syntax/.gate-baseline` (only pass→not-pass regresses; a full run also reds a vanished
    /// case). This is the syntax sibling of `gate`; a `--files`/`--case` run is a subset (skips vanished).
    GateSyntax {
        /// Case directories to grade (relative to `spec/syntax/` or absolute). [default: the whole corpus]
        files: Vec<PathBuf>,
        /// Grade only cases whose title (`<surface>/<name>`) contains this substring.
        #[arg(long)]
        case: Option<String>,
        /// Save the current per-case verdicts as `spec/syntax/.gate-baseline`, then exit.
        #[arg(long, conflicts_with = "check")]
        save: bool,
        /// Compare to the baseline; fail on a regression (pass→not-pass), a vanished case, or a fail
        /// not covered by the baseline.
        #[arg(long)]
        check: bool,
        /// Fold PRE-HARVESTED `<verdict>\t<title>` verdicts from this file against the baseline WITHOUT
        /// re-grading via `cdz` (the per-case nix aggregate's entry). Ignores `--files`/`--case`.
        #[arg(long, conflicts_with_all = ["save", "check"])]
        compare: Option<PathBuf>,
        /// Override the baseline path (default `spec/syntax/.gate-baseline`). The per-case nix aggregate
        /// passes this since `xtask` runs outside a repo tree. Applies to `--check`/`--compare`/`--save`.
        #[arg(long)]
        baseline: Option<PathBuf>,
    },
    /// The omnibus health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
    /// the wasm runtime build, and the behavior gate. Each step's output is captured to a log file
    /// (`target/xtask-logs/`); the console shows one ✓ per step, and the first failing step prints
    /// the whole log + its path.
    Check,
    /// Compile a Cadenza program to a component and write it out (the compile-only half of `run`).
    Emit {
        /// The Cadenza program file to compile.
        file: PathBuf,
        /// The input surface. [default: sexpr]
        #[arg(long, default_value = "sexpr")]
        from: String,
        /// Where to write the component. [default: <file>.wasm]
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Generate the structured value-heap runtime-ABI table the wasm backend consumes. Reads the
    /// runtime's `wit/runtime.wit` (the ABI's source of truth) with `wit-parser` and writes
    /// `crates/rcdzc/src/backend/wasm/runtime_abi.rs` — every declared op as `{ name, params, result }`
    /// core-signature data. The compiler builds its per-program import section from this rather than
    /// pasting opaque envelope blobs. Run after changing the runtime WIT; the output is committed.
    ///
    /// This is ALSO the runtime-HASH SYNC command (the flag-day PATH-B convention, operator 2026-08-29):
    /// it recomputes the three committed content-address hashes — `REQUIRED_RUNTIME_HASH` /
    /// `DEBUG_RUNTIME_HASH` / `REQUIRED_NFC_HASH` — from the freshly built runtime and rewrites them into
    /// `runtime_abi.rs`. So after a runtime change, `cargo xtask codegen` is the ONE command that re-syncs
    /// the committed hash; there is deliberately no separate `sync-hashes` (it would only duplicate this).
    /// `--check` (below) plus the nix `*-hash-parity` checks fail on any committed-vs-freshly-computed drift.
    Codegen {
        /// Don't write; regenerate in memory and exit non-zero if the committed file is out of date.
        /// This is the STALENESS GATE (wired into `xtask check`): it makes a forgotten regeneration a
        /// hard failure rather than a silent drift, so the generated ABI can never fall behind the WIT.
        #[arg(long)]
        check: bool,
    },
    /// Build the browser-facing compiler wasm for the interactive guide and stage it (plus the
    /// value-heap runtime) into `guide/src/wasm/`. Runs `wasm-pack build --target web` on `cdz-wasm`
    /// then the guide's `stage-wasm.mjs`. Run `build` first so the runtime the compiler pins is in the
    /// store. This is what `guide/`'s `npm run wasm` calls, and what CI runs before building the site.
    GuideWasm {
        /// Content-addressed store the runtime is staged from. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Orchestrate the autonomous-agent fleet: bring agents up as named tmux windows, tear them
    /// down, inspect the board, add/remove agents, and route inbox messages. The durable manifest is
    /// `.claude/fleet/registry.json`; see `.claude/fleet/AGENTS-fleet.md` for the agent contract.
    Fleet {
        #[command(subcommand)]
        cmd: fleet::FleetCmd,
    },
    /// Any UNRECOGNIZED subcommand is forwarded to the nix app of the same name:
    /// `cargo xtask <cmd> [args…]` → `nix run <worktree-flake>#<cmd> -- [args…]`. This is the all-nix
    /// COMPAT bridge (operator 2026-08-28: `cargo run` migrates to nix as tools land) for xtask
    /// subcommands decomposed into standalone `apps.<cmd>` (v-xtask-decompose): once a `Cmd` arm is
    /// removed + `apps.<cmd>` exists, `cargo xtask <cmd>` transparently routes to the nix app instead of
    /// clap-erroring. If no such app exists nix reports it (that IS the unknown-command feedback).
    /// Phased by app-existence; no cargo-shadowing shim needed for this compat.
    #[command(external_subcommand)]
    External(Vec<String>),
}

fn main() {
    let paths = Paths::resolve();
    let cli = Cli::parse();
    let profile = cli.profile.as_str();
    match cli.command {
        // `build` builds the runtime component under its own release settings (cargo component), not
        // the tool profile — so the profile flag doesn't apply to it.
        Cmd::Build { store } => build(&paths, store),
        Cmd::Run { file, from, store } => run(&paths, profile, &file, &from, store),
        Cmd::Gate {
            files,
            store,
            case,
            save,
            check,
            target,
            opt_sweep,
            shard,
            guarded_all,
        } => {
            let shard = shard
                .as_deref()
                .map(parse_shard)
                .transpose()
                .unwrap_or_else(|e| {
                    eprintln!("xtask gate --shard: {e}");
                    std::process::exit(2);
                });
            let gate_opts = GateOpts {
                files,
                store,
                case,
                save,
                check,
                target: match target {
                    GateTargetArg::Wasm => GateTarget::Wasm,
                    GateTargetArg::Rust => GateTarget::Rust,
                    GateTargetArg::RustAsync => GateTarget::RustAsync,
                    GateTargetArg::Cadenza => GateTarget::Cadenza,
                },
                shard,
                guarded_all,
            };
            if opt_sweep {
                gate_opt_sweep(&paths, profile, &gate_opts);
            } else {
                gate(&paths, profile, gate_opts);
            }
        }
        Cmd::GateSyntax {
            files,
            case,
            save,
            check,
            compare,
            baseline,
        } => {
            let opts = gate_syntax::GateSyntaxOpts {
                files,
                case,
                save,
                check,
                compare,
                baseline,
            };
            std::process::exit(gate_syntax::gate_syntax(&paths, &opts));
        }
        Cmd::Check => check(&paths, profile),
        Cmd::Emit { file, from, out } => emit(&paths, profile, &file, &from, out),
        Cmd::Codegen { check } => codegen::run(&paths, check),
        Cmd::GuideWasm { store } => guide_wasm(&paths, store),
        Cmd::Fleet { cmd } => fleet::run(&paths, cmd),
        Cmd::External(args) => run_external_subcommand(&args),
    }
}

/// Build the `nix run` argv that forwards an unrecognized `cargo xtask <cmd> [args…]` to the nix app of
/// the same name: `[cmd, rest…]` → `["run", "<flake>#<cmd>", "--", rest…]` (the `--` only when there are
/// args, so a bare `cargo xtask <cmd>` doesn't pass an empty `--`). Pure so the mapping is unit-tested.
fn nix_run_external_argv(args: &[String], flake: &str) -> Vec<String> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let mut v = vec!["run".to_string(), format!("{flake}#{cmd}")];
    if args.len() > 1 {
        v.push("--".to_string());
        v.extend(args[1..].iter().cloned());
    }
    v
}

/// Forward an unrecognized `cargo xtask <cmd> [args…]` to `nix run <worktree-flake>#<cmd> -- [args…]` —
/// the all-nix compat bridge for decomposed xtask subcommands (see `Cmd::External`). Resolves the flake
/// as the current worktree toplevel (falls back to `.`), then `exec`s nix (replacing this process, so the
/// nix app's exit code is the caller's). On a genuine typo the app won't exist and nix reports it — that
/// is the "unknown command" feedback. Never returns on success.
fn run_external_subcommand(args: &[String]) -> ! {
    use std::os::unix::process::CommandExt;
    let cmd = args.first().cloned().unwrap_or_default();
    if cmd.is_empty() {
        eprintln!("xtask: no subcommand given.");
        std::process::exit(2);
    }
    let flake = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string());
    let argv = nix_run_external_argv(args, &flake);
    let nix = fleet::nix_binary();
    eprintln!(
        "xtask: '{cmd}' is not a built-in subcommand — forwarding to nix ({nix} {})",
        argv.join(" ")
    );
    let e = std::process::Command::new(&nix).args(&argv).exec();
    eprintln!("xtask: could not exec {nix} ({e}) — is nix installed + on PATH?");
    std::process::exit(127);
}

mod codegen;
mod fleet;
mod gate_syntax;

/// The workspace directory anchors, resolved once from this crate's manifest location. xtask lives
/// at `<repo>/xtask`, so the repo root is the manifest's parent and the seed workspace is the fixed
/// `<repo>/implementation/seed` beneath it. Every path derives from these two — no fragile
/// `.parent().parent()` chains, and correct inside a git worktree (each worktree's manifest dir
/// resolves to that worktree's own root).
struct Paths {
    /// `<repo>` — the workspace root (parent of `<repo>/xtask`).
    pub(crate) repo: PathBuf,
    /// `<repo>/implementation/seed` — the seed toolchain root that holds `crates/`.
    pub(crate) seed: PathBuf,
}

impl Paths {
    fn resolve() -> Self {
        // `CDZ_REPO_ROOT` override (the nix-native decomposition path, v-xtask-decompose): the default
        // resolution below bakes `CARGO_MANIFEST_DIR` at COMPILE time, which is the real worktree only
        // for a `cargo`-built binary run in place. A nix-built `xtask` (crane) bakes the build-sandbox
        // path — nonexistent at runtime — so a relocatable nix app (`nix run .#roundtrip` &c.) can't
        // self-locate the repo. The per-subcommand nix apps therefore pass `CDZ_REPO_ROOT=<worktree>`
        // (from `git rev-parse --show-toplevel`) and this honors it. Unset (the `cargo xtask` path) →
        // the CARGO_MANIFEST_DIR fallback, so existing behavior is byte-for-byte unchanged.
        let repo = match std::env::var_os("CDZ_REPO_ROOT") {
            Some(root) => PathBuf::from(root),
            None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("xtask crate has a parent (the repo root)")
                .to_path_buf(),
        };
        let seed = repo.join("implementation/seed");
        Paths { repo, seed }
    }
}

/// Store `bytes` at `dest` in the content-addressed store via a same-directory temp file + atomic `rename`,
/// so a rebuild that re-stores an entry is idempotent no matter how the store was previously populated. Two
/// reasons a plain `fs::write(dest, ..)` is wrong here: (1) `target/cadenza-store` entries can be SYMLINKS
/// into the READ-ONLY nix component store (a devshell staging flow links `<hash>.wasm` → the nix store);
/// `fs::write` FOLLOWS such a symlink and hits `EACCES` writing into the read-only nix store — this bites the
/// NFC entry first, whose content hash is usually UNCHANGED across a runtime-hash bump so its stale symlink
/// pre-exists. (2) the store is READ CONCURRENTLY across the fleet (programs resolve the runtime by content
/// hash out of this dir), so a remove-then-write would expose an absent-target window; an atomic `rename`
/// replaces the destination directory entry (regular file OR stale symlink) in one step, never observed
/// absent or half-written. The final on-disk name is EXACTLY `dest` (the content-addressed `<hash>.wasm` a
/// composed program imports), and a missing prior entry (fresh store / first build) is fine.
fn store_atomic_write(dest: &std::path::Path, bytes: &[u8]) {
    let parent = dest.parent().unwrap_or_else(|| std::path::Path::new("."));
    let fname = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("store-entry");
    let tmp = parent.join(format!(
        ".{fname}.tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    // Fresh, unique temp path (pid+nanos); clear any lingering entry so the write itself never follows a
    // stale symlink, then atomically rename onto `dest` (replacing a prior file or nix-store symlink).
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(&tmp, bytes)
        .unwrap_or_else(|e| panic!("store: write temp {}: {e}", tmp.display()));
    std::fs::rename(&tmp, dest)
        .unwrap_or_else(|e| panic!("store: rename {} → {}: {e}", tmp.display(), dest.display()));
}

fn build(paths: &Paths, store: Option<PathBuf>) {
    // Acquire the fleet-wide build/check concurrency lease FIRST (operator-mandated, 2026-07-20 host
    // hang). `cargo xtask build` recompiles the wasm runtime + build-std from source — a heavy multi-
    // core build that EVERY agent runs each `fleet sync` tick to refresh its store. On a synchronized
    // trunk-advance wake, ~42 agents ran it AT ONCE with nothing capping the count, oversubscribing the
    // scheduler to thousands of rustc/linker threads and HARD-HANGING the 64-core box (EC2 power-cycle).
    // `check`/`gate` were already capped by this same lease pool; `build` was the unleased gap. Sharing
    // ONE pool (not a second independent budget) is deliberate: build and check are both heavy `cargo`
    // workloads competing for the same cores, so K total is the honest cap. pr-sync's gate-batch build
    // sets CDZ_CHECK_PRIORITY=1 to take an uncapped priority slot (the merge queue never waits behind
    // one-off agent store rebuilds). Held for the whole build; released on return (RAII drop). Fail-open.
    let priority = std::env::var("CDZ_CHECK_PRIORITY").is_ok_and(|v| v == "1" || v == "true");
    let _lease = fleet::acquire_check_lease(&paths.repo, priority);

    let store = store.unwrap_or_else(|| paths.repo.join("target/cadenza-store"));
    std::fs::create_dir_all(&store).expect("create store dir");

    // Build the NFC component (`cdz-nfc`, FINDING#23) FIRST: its content address is stamped INLINE into each
    // heap's `cadenza:nfc/normalize` import (`stamp_nfc_into_heap`), making the heap self-describing about its
    // NFC dependency — so a runtime resolves NFC purely from the import name, with NO `runtime.toml` / mapping
    // file read at run time (operator directive 2026-08-23: no mapping passed to executables). So we need the
    // NFC hash BEFORE building/stamping the heaps. NFC carries the heavy `unicode-normalization` tables so the
    // tagless core runtime does not. Canonicalize (strip `producers`) before hashing so the hash matches
    // `REQUIRED_NFC_HASH` and is reproducible cross-host; store it in the SAME CAS (`<store>/<nfc-hash>.wasm`).
    println!("== xtask: building the value-heap runtime + its NFC dependency ==");
    let sh = Shell::new().expect("open a shell for the component build");

    let nfc_wasm = build_component(&sh, &paths.seed, "cdz-nfc", "cdz_nfc");
    let nfc_bytes = canonicalize_runtime(&nfc_wasm);
    let nfc_hash = content_address(&nfc_bytes);
    let nfc_stored = store.join(format!("{nfc_hash}.wasm"));
    store_atomic_write(&nfc_stored, &nfc_bytes);
    println!("   nfc component content address: {nfc_hash}");
    println!("   stored → {}", nfc_stored.display());

    // Build BOTH heap runtimes (wasm32), STAMP each with the NFC address inline, then CANONICALIZE (strip the
    // tool-version `producers` sections) + content-address + store: the RELEASE runtime (what a shipped program
    // pins + composes) and the DEBUG-COUNTERS runtime (the same code with the `live-objects` leak counter — the
    // Perceus leak-check harness composes it, located by content address, never rebuilt). The stored artifact
    // IS the stamped+stripped bytes, so a composed program's imported hash matches the file on disk. The DEBUG
    // build is fully processed BEFORE the RELEASE build overwrites the shared `cdz_runtime.wasm` output path.
    let debug_wasm = build_component_with_features(
        &sh,
        &paths.seed,
        "cdz-runtime",
        "cdz_runtime",
        &["debug-counters"],
    );
    let debug_stamped = stamp_nfc_into_heap(&paths.repo, &debug_wasm, &nfc_hash);
    let debug_bytes = canonicalize_runtime(&debug_stamped);
    let debug_hash = content_address(&debug_bytes);
    store_atomic_write(&store.join(format!("{debug_hash}.wasm")), &debug_bytes);
    println!("   debug-counters runtime content address: {debug_hash}");

    let runtime_wasm = build_component(&sh, &paths.seed, "cdz-runtime", "cdz_runtime");
    let runtime_stamped = stamp_nfc_into_heap(&paths.repo, &runtime_wasm, &nfc_hash);
    let runtime_bytes = canonicalize_runtime(&runtime_stamped);
    let runtime_hash = content_address(&runtime_bytes);
    println!("   runtime content address: {runtime_hash}");
    let runtime_stored = store.join(format!("{runtime_hash}.wasm"));
    store_atomic_write(&runtime_stored, &runtime_bytes);
    println!("   stored → {}", runtime_stored.display());

    // Native-build fidelity guard (WARN only). A nix-built `cdz` pins the committed (nix-canonical)
    // `REQUIRED_RUNTIME_HASH`, so a program it compiles imports its runtime BY that hash. A NATIVE
    // `cargo xtask build` can produce a divergent release runtime hash (ambient vs pinned-hermetic
    // toolchain/opt — the "native runtime hash is unfaithful" class), which a nix-`cdz`-compiled program
    // cannot resolve → a cryptic "no runtime of content address …" at RUN time (cost v-inference + v-corpus-
    // harness real debugging time). Surface it loudly HERE. This changes NOTHING (not the emitted bytes,
    // the store contents, or the committed const) — a pure build-time divergence notice, so it is safe
    // under either frozen runtime-hash flag-day resolution. REJECT-only `--case` gating is unaffected
    // (it needs no runtime); only RUNTIME (execute) cases require the canonical store.
    if let Some(committed) = parse_committed_runtime_hash(&paths.repo)
        && committed != runtime_hash
    {
        eprintln!(
            "\n⚠ cargo xtask build: native release runtime hash ({runtime_hash}) != committed canonical\n\
             ⚠ REQUIRED_RUNTIME_HASH ({committed}) — OK for REJECT `--case`, NOT for RUNTIME (execute)\n\
             ⚠ cases (a nix-built `cdz` pins the canonical hash). Gate RUNTIME cases via the nix store:\n\
             ⚠     CDZ_STORE=$(nix build --no-link --print-out-paths .#store)\n"
        );
    }

    // A small manifest listing the stored heap runtimes — INFORMATIONAL only (a nix-build / store-listing
    // artifact), NOT read by any executable at run time. There is no `nfc = "<hash>"` mapping line anymore:
    // the NFC dependency is resolved from each heap's self-describing inline import, not from here (operator
    // directive: no mapping file passed to executables).
    let manifest = format!(
        "# Cadenza content-addressed store — the value-heap runtime builds (informational; the NFC\n\
         # dependency is resolved from each heap's self-describing inline import, not from this file).\n\
         runtime = \"{runtime_hash}\"\n\
         debug_runtime = \"{debug_hash}\"\n"
    );
    store_atomic_write(&store.join("runtime.toml"), manifest.as_bytes());

    println!("\n== xtask: done ==");
    println!("   store:   {}", store.display());
    println!("   runtime: {runtime_hash}");
    println!("   debug:   {debug_hash}");
}

/// Build the browser compiler wasm for the guide and stage it. Two steps, each delegated to its tool:
///   1. `wasm-pack build --target web --release` on `crates/cdz-wasm` → the JS glue + wasm in `pkg/`.
///   2. the guide's `scripts/stage-wasm.mjs` → copies `pkg/` and the pinned value-heap runtime into
///      `guide/src/wasm/`, where Vite picks them up.
///
/// Run `cargo xtask build` first so the store holds the runtime whose hash the compiler pins.
fn guide_wasm(paths: &Paths, store: Option<PathBuf>) {
    // Take the fleet-wide build/check concurrency lease (host-hang fix, 2026-07-20): `wasm-pack build
    // --release` is a heavy compile competing for the same cores as `build`/`check`/`bench`, which share
    // this pool. Held for the whole build (RAII drop). Fail-open. Not spawned by any lease-holder, so no
    // deadlock. (`gate` deliberately stays UNLEASED — `check` spawns it as a child while holding a lease,
    // so leasing it would self-deadlock; `check` already caps that path.)
    let priority = std::env::var("CDZ_CHECK_PRIORITY").is_ok_and(|v| v == "1" || v == "true");
    let _lease = fleet::acquire_check_lease(&paths.repo, priority);

    let sh = Shell::new().expect("open a shell for the guide wasm build");
    let crate_dir = paths.seed.join("crates/cdz-wasm");
    let guide = paths.repo.join("guide");
    let store = store.unwrap_or_else(|| paths.repo.join("target/cadenza-store"));

    println!("== xtask: building the guide compiler wasm (wasm-pack --target web) ==");
    {
        let _pushed = sh.push_dir(&crate_dir);
        // Raise the wasm linear-memory STACK from LLVM's ~1 MB default to 16 MB. The compiler's
        // recursive-descent passes recurse one frame per nesting level and rely on reaching the semantic
        // depth guard (`rcdzc::db::DESCENT_DEPTH_LIMIT`) before the stack is exhausted. Natively
        // `rcdzc::host::run_with_compiler_stack` spawns a 64 MB-stack thread to guarantee that, but on
        // wasm32 that path runs INLINE (a thread-per-compile leaks the long-lived instance's memory), so
        // the wasm stack ITSELF must be large enough. At 1 MB a moderately nested program (e.g. a
        // `(module …)` inside a `def` body) overflows and traps `memory access out of bounds`; 16 MB
        // clears the guide's inputs with wide margin while staying far below the 4 GiB wasm32 address
        // space. The wasm-correct analogue of the native 64 MB thread — keep the two in lockstep.
        if let Err(e) = cmd!(sh, "wasm-pack build --target web --release")
            .env("RUSTFLAGS", "-C link-arg=-zstack-size=16777216")
            .run()
        {
            eprintln!(
                "wasm-pack build failed for cdz-wasm: {e}\n\
                 (install it with `cargo install wasm-pack`; needs the wasm32-unknown-unknown target)"
            );
            std::process::exit(1);
        }
    }

    println!("== xtask: staging pkg/ + runtime into guide/src/wasm/ ==");
    let script = guide.join("scripts/stage-wasm.mjs");
    let store_str = store.to_string_lossy().to_string();
    {
        let _pushed = sh.push_dir(&guide);
        // The stage script reads an optional CADENZA_STORE to locate the pinned runtime.
        if let Err(e) = cmd!(sh, "node {script}")
            .env("CADENZA_STORE", &store_str)
            .run()
        {
            eprintln!("staging failed: {e}\n(is node ≥20.19 on PATH?)");
            std::process::exit(1);
        }
    }
    println!("\n== xtask: guide wasm ready — `cd guide && npm run dev` ==");
}

/// Compile a Cadenza program and run it — the whole pipeline end-to-end, delegating each stage to
/// its binary (xtask pulls in none of them as a library; it only choreographs):
///   1. `cdz-syntax` — the program's surface (sexpr/ml) → binary AST.
///   2. `rcdzc`      — binary AST → a wasm component.
///   3. `cdz-run`    — instantiate + run the component; its stdout is the result.
///
/// The three are wired as a real OS PIPE (each stage's stdout is the next's stdin) — NO temp files,
/// so concurrent `xtask run` invocations never share or clobber state, and each stage's own stderr
/// (a parse error, a diagnostic) inherits straight to the terminal. The tools are built ONCE first,
/// then the built binaries are piped, so the three piped stages don't contend on cargo's build lock.
/// cdz-run's stdout — the program's result — is inherited to this process's stdout.
fn run(paths: &Paths, profile: &str, file: &Path, from: &str, store: Option<PathBuf>) {
    use std::process::{Command, Stdio};

    if !file.exists() {
        eprintln!("xtask run: no such file: {}", file.display());
        std::process::exit(1);
    }

    // Build the three tools once, so the pipe below runs finished binaries rather than three
    // concurrent `cargo run`s racing the build lock.
    let tools = build_tools(paths, profile);

    // ── The pipe: cdz-syntax <file> | rcdzc - -o - | cdz-run - ──
    // Stage 1 reads the program file and writes binary AST to stdout.
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));

    // Stage 2 reads AST from stage 1's stdout, writes the component to stdout.
    let mut rcdzc = Command::new(&tools.rcdzc)
        .args(["compile", "-", "-o", "-"])
        .stdin(Stdio::from(
            syntax.stdout.take().expect("cdz-syntax stdout"),
        ))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));

    // Stage 3 reads the component from stage 2's stdout, runs it, and prints the result to OUR
    // stdout (inherited). The store the runtime is resolved from is forwarded when given.
    let mut run = Command::new(&tools.run);
    run.arg("-")
        .stdin(Stdio::from(rcdzc.stdout.take().expect("rcdzc stdout")));
    if let Some(dir) = &store {
        run.arg("--store").arg(dir);
    }
    let run = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));

    // Wait on every stage under ONE shared wall-clock deadline, killing any survivor at the deadline.
    // A bare `.wait()` here (the previous code) had NO bound, so a hung stage wedged `run` FOREVER — a
    // compile-hang in rcdzc, or cdz-run blocked resolving a COLD/empty store in a fresh worktree, would
    // never return. That was the gap behind "`cargo xtask run` hangs in fresh worktrees" (v-effects via
    // concierge 2026-08-04): the gate's per-case run path was already bounded (`wait_with_timeout` in
    // `run_program_wasm`), so `xtask gate` worked while `xtask run` hung. This gives `run` the SAME
    // `run_timeout()` bound. Unlike `wait_with_timeout` (which pipes+captures), the stages keep their
    // inherited/piped stdio so the result still streams live to the terminal — `wait_stages_with_timeout`
    // only polls exit + kills. It reports the first not-yet-exited stage on timeout; pipeline order in
    // the array preserves "first failing stage sets the exit code".
    // Read the timeout ONCE and reuse it for both the enforced deadline and the message, so a
    // `CDZ_RUN_TIMEOUT_SECS` change mid-run can't make the "did not finish within {N}s" text disagree
    // with the deadline actually enforced (github-liaison/Copilot PR#2037 review).
    let timeout = run_timeout();
    match wait_stages_with_timeout(
        [("cdz-syntax", syntax), ("rcdzc", rcdzc), ("cdz-run", run)],
        timeout,
    ) {
        Ok(statuses) => {
            // Every stage exited within the deadline — first failing stage (pipeline order) sets the code.
            for status in statuses {
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        Err(StageWait::Timeout(stage)) => {
            eprintln!(
                "xtask run: '{stage}' did not finish within {}s — killed (hang). Raise \
                 CDZ_RUN_TIMEOUT_SECS if this is a legitimately long run; a fresh-worktree hang usually \
                 means a cold store — run `cargo xtask build` first.",
                timeout.as_secs()
            );
            std::process::exit(1);
        }
        Err(StageWait::WaitError(stage, e)) => {
            eprintln!("xtask run: {stage} did not complete: {e}");
            std::process::exit(1);
        }
    }
}

/// Why `wait_stages_with_timeout` failed: a stage exceeded the deadline (its name), or a `try_wait`
/// errored on a stage (its name + the io error). Distinguishes the hang (kill + named) from an OS-level
/// wait failure (surfaced), so the caller can message each precisely.
#[derive(Debug)]
enum StageWait {
    Timeout(String),
    WaitError(String, std::io::Error),
}

/// Poll a fixed set of named pipeline children to exit under ONE shared wall-clock `timeout`, KILLING
/// any still-running stage at the deadline. Returns each stage's `ExitStatus` in the SAME ORDER on
/// success (so the caller's "first failing stage sets the exit code" holds by index), or the first
/// stage that timed out / errored. The children keep whatever stdio they were spawned with (this does
/// NOT touch their pipes — unlike `wait_with_timeout`, which drains+captures), so an inherited-stdout
/// pipeline still streams live. Same try_wait/kill/deadline shape as `wait_with_timeout`, generalized to
/// N stages; unit-tested with real sleeper children so the hang-kill path has coverage `run()` can't get
/// (its stages are real built tools). `N` is a const generic so the caller's array size is preserved.
fn wait_stages_with_timeout<const N: usize>(
    children: [(&str, std::process::Child); N],
    timeout: std::time::Duration,
) -> Result<[std::process::ExitStatus; N], StageWait> {
    let mut stages: Vec<(&str, std::process::Child, Option<std::process::ExitStatus>)> =
        children.into_iter().map(|(n, c)| (n, c, None)).collect();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let mut all_done = true;
        for (stage, child, status) in stages.iter_mut() {
            if status.is_none() {
                match child.try_wait() {
                    Ok(Some(s)) => *status = Some(s),
                    Ok(None) => all_done = false,
                    Err(e) => return Err(StageWait::WaitError(stage.to_string(), e)),
                }
            }
        }
        if all_done {
            break;
        }
        if std::time::Instant::now() >= deadline {
            // Deadline hit — kill + reap EVERY still-running stage (leave no orphan), then report the
            // FIRST one that hadn't exited (pipeline order → the earliest-blocking stage is named).
            let mut first_hung: Option<String> = None;
            for (stage, child, status) in stages.iter_mut() {
                if status.is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                    first_hung.get_or_insert_with(|| stage.to_string());
                }
            }
            return Err(StageWait::Timeout(
                first_hung.unwrap_or_else(|| "unknown".to_string()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Every stage exited — collect statuses in the original (pipeline) order.
    let out: Vec<std::process::ExitStatus> =
        stages.into_iter().map(|(_, _, s)| s.unwrap()).collect();
    Ok(out.try_into().expect("N stages in → N statuses out"))
}

/// One child's harvested outcome from [`wait_children_until`]: `Ok(Some(status))` = exited (read
/// success/fail), `Ok(None)` = hit the deadline and was killed (a hang, no verdict), `Err` = a
/// `try_wait` error. Aliased so the waiter's return type stays legible (clippy type-complexity).
type ChildOutcome = std::io::Result<Option<std::process::ExitStatus>>;

/// Poll a DYNAMIC set of named children to exit under ONE shared wall-clock deadline, collecting EVERY
/// child's outcome independently — the concurrent-gate waiter for scope-split parallel batching (model a,
/// slice ii). Unlike [`wait_stages_with_timeout`] (fixed `[_; N]`, pipeline semantics: returns the first
/// failure by index, kills all on the first timeout), this is for INDEPENDENT peers: a runtime `Vec` of
/// lanes, and each lane's result stands alone (a green lane still lands even if a sibling lane reds or
/// hangs). Returns, per child in input order, `Ok(Some(status))` (exited — caller reads success/fail),
/// `Ok(None)` (this child hit the deadline and was KILLED — a hang, no verdict), or `Err` (a `try_wait`
/// error). The children keep their spawned stdio (caller pipes/captures as it likes). Every still-running
/// child is killed+reaped at the deadline so no nix builder orphans. `pub(crate)` so `fleet.rs`'s
/// concurrent gate calls it. The children must already be SPAWNED (running concurrently at the OS level)
/// before this is called — that's where the parallelism comes from; this only harvests them.
pub(crate) fn wait_children_until(
    children: Vec<(String, std::process::Child)>,
    timeout: std::time::Duration,
) -> Vec<(String, ChildOutcome)> {
    let deadline = std::time::Instant::now() + timeout;
    let mut slots: Vec<(String, Option<std::process::Child>, Option<ChildOutcome>)> = children
        .into_iter()
        .map(|(n, c)| (n, Some(c), None))
        .collect();
    loop {
        let mut all_done = true;
        for (_name, child, result) in slots.iter_mut() {
            if result.is_some() {
                continue;
            }
            match child.as_mut().unwrap().try_wait() {
                Ok(Some(s)) => *result = Some(Ok(Some(s))),
                Ok(None) => all_done = false,
                Err(e) => *result = Some(Err(e)),
            }
        }
        if all_done || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Deadline (or all-done): any child still running is a hang → kill+reap it and record None (timed out).
    for (_name, child, result) in slots.iter_mut() {
        if result.is_none() {
            if let Some(mut c) = child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            *result = Some(Ok(None));
        }
    }
    slots
        .into_iter()
        .map(|(n, _, r)| (n, r.expect("every slot resolved above")))
        .collect()
}

/// Do a set of time-SPANS actually OVERLAP — the sanity-check the concurrent gate must pass (concierge
/// 2026-08-10): lanes gated "in parallel" only WIN wall-clock if their runs truly overlap in time; if a
/// lock / lease / single nix-builder-slot silently serialized them (span B starts only after span A
/// ended), the parallelism is a NO-OP and we'd be reporting false throughput. Each span is
/// `(start_ms, end_ms)` on a shared monotonic clock (elapsed millis from one origin — pure, so the check
/// is unit-tested without real `Instant`s). Returns true iff SOME two spans overlap. A single span (or
/// none) → false (nothing to overlap). A degenerate zero-width span never contributes an overlap.
pub(crate) fn spans_overlap(spans: &[(u128, u128)]) -> bool {
    let mut xs: Vec<(u128, u128)> = spans.iter().copied().filter(|(s, e)| e > s).collect();
    xs.sort_by_key(|(s, _)| *s);
    let mut max_end: Option<u128> = None;
    for (s, e) in xs {
        if let Some(m) = max_end
            && s < m
        {
            return true; // this span starts before an earlier one ended → real overlap
        }
        max_end = Some(max_end.map_or(e, |m| m.max(e)));
    }
    false
}

/// Per-invocation wall-clock ceiling for a single compile/run child (`cdz compile`, `cdz-run`, …).
/// A compile-hang bug (the known `and/or/not` and `Any`-parse-if hangs) makes an individual case spin
/// FOREVER at high CPU; under gate-batch's merged tree these accumulate (10→64→170+ live procs) and
/// starve the host, and they never self-terminate. Bounding each child at a hard deadline turns an
/// infinite hang into a single FAIL(hang) — the blast radius of any one compile-hang is one case, not
/// the whole gate + the whole host. 120s is generous vs a normal case (<1s) yet well under the ~35min
/// full-gate window; override with `CDZ_RUN_TIMEOUT_SECS` for a slow host or a debugging session.
fn run_timeout() -> std::time::Duration {
    let secs = std::env::var("CDZ_RUN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(120);
    std::time::Duration::from_secs(secs)
}

/// Wall-clock cap for a whole `cdz test <suite>` SUITE run (not a single case). Distinct from
/// `run_timeout` (per-corpus-case): a suite legitimately runs MINUTES (the compiler-ml sweep compiles +
/// runs ~73 cases), so the cap is GENEROUS — the point is to convert a TRUE HANG (e.g. `cf-corpus-all-pass`
/// spinning on a known compile-hang, which emits no output and looks to every observer like a silent kill
/// with no TOTAL) into a LOUD, NAMED, auto-bisectable FAIL, NOT to throttle a slow-but-passing suite.
/// Per-suite wall-clock cap. Most suites are quick and share the 6min default, but `compiler-ml` is the
/// dominant sweep — ~73 cases each compiling a Cadenza program through the ML compiler AND running it
/// under wasmtime, and its heaviest file (sread-eval: 40 run-src @tests, each building the WHOLE pipeline
/// into its own component) alone measured >300s UNDER LOAD. At the flat 6min cap a load-spike pushes the
/// suite past 360s NONDETERMINISTICALLY → a false HANG-fail on a known-good base (the standing
/// throughput-drag red gate). Until v-compiler-ml's file SPLIT lands (their in-lane fix — shrinks the
/// per-file build so no file nears the cap), give compiler-ml a larger cap so a load-spike can't
/// false-fail it, WITHOUT blinding the suite (a true infinite hang still fails loud+named at the higher
/// cap — this raises the false-hang threshold, it does not quarantine). `CDZ_SUITE_TIMEOUT_SECS`
/// overrides ALL suites when set (an operator escape hatch). Value chosen to comfortably clear the
/// measured ~25-30min worst-case full compiler-ml sweep with margin.
fn suite_timeout_for(suite: &str) -> std::time::Duration {
    if let Ok(secs) = std::env::var("CDZ_SUITE_TIMEOUT_SECS")
        && let Ok(secs) = secs.parse::<u64>()
        && secs > 0
    {
        return std::time::Duration::from_secs(secs);
    }
    let secs = if suite.contains("compiler-ml") {
        45 * 60 // the heavy sweep — clear its ~25-30min worst case + load margin (not a throttle)
    } else {
        6 * 60
    };
    std::time::Duration::from_secs(secs)
}

/// Per-FILE wall-clock cap for the compiler-ml sweep when it is run one file at a time (see
/// `step_cached_per_file`). Distinct from `suite_timeout_for` (a whole-suite ceiling): a single
/// pathological compile — one def that blows up the compiler and NEVER exits (the runaway-compile
/// gate-block, pid 456190 @ 18min+@100% never exiting, 2026-07-20) — otherwise burns the ENTIRE
/// generous 45min suite budget before the step can fail, and pr-sync re-runs `check` several times a
/// batch, so ONE bad file freezes the whole backlog for ~1h. Bounding each file individually kills
/// only the offending compile, at a tight cap, NAMED and auto-bisectable, while every innocent file
/// still runs. TWO legitimate files now sit near the cap under load (pr-sync report): sread-eval
/// (40 run-src @tests, each building the whole pipeline into its own component) at ~480s, and
/// conformance-db-cx at ~580-700s — both slow-but-PASSING (the compile progresses, doesn't loop),
/// racing the old 720s cap NONDETERMINISTICALLY when fleet load is high (they finish at ~750-900s under
/// contention). So the default is 1200s — the value pr-sync had to set by hand
/// (`CDZ_ML_PER_FILE_TIMEOUT_SECS=1200`) on EVERY re-gate, which was doubling the ~3hr suite per batch;
/// making it the default lets the FIRST gate pass and ends that wasted double-gate. Enough headroom that
/// a genuinely-slow file finishes under load, while still FAR below the 45-min whole-suite ceiling (so
/// one true runaway is still killed fast+named). This
/// is a HANG bound, not a throughput throttle (concurrency — JOBS=2 — is the throughput lever).
/// `CDZ_ML_PER_FILE_TIMEOUT_SECS` overrides (operator escape hatch); `=0`/garbage → default.
fn ml_per_file_timeout() -> std::time::Duration {
    if let Ok(secs) = std::env::var("CDZ_ML_PER_FILE_TIMEOUT_SECS")
        && let Ok(secs) = secs.parse::<u64>()
        && secs > 0
    {
        return std::time::Duration::from_secs(secs);
    }
    std::time::Duration::from_secs(1200) // covers the ~750-900s-under-load worst case (sread-eval-sum/conformance-db-cx); pr-sync-validated. Hang bound.
}

/// One worker's captured outcome for a single `cdz test <file>` in the parallel per-file sweep. The
/// worker records this into its slot; the main thread replays them in file order for an order-stable log.
struct PerFileResult {
    verdict: PerFileVerdict,
}

/// The four ways a per-file `cdz test` can end — mirrors the arms of the old serial `wait_with_timeout`
/// match, but as owned data so the verdict can cross the worker→main-thread boundary before it's acted on.
enum PerFileVerdict {
    /// The child exited within the cap; `ok` is `status.success()`. Output is captured for the log.
    Ran {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        ok: bool,
    },
    /// Killed for exceeding the per-file cap (runaway compile) after `elapsed` seconds.
    TimedOut { elapsed: u64 },
    /// The child could not be spawned at all.
    LaunchErr(String),
    /// The wait itself errored (an io failure, not a child verdict).
    WaitErr(String),
}

/// How many compiler-ml `cdz test` files to run CONCURRENTLY in the per-file gate sweep. Each job is a
/// full-pipeline WASM compile (memory-heavy AND CPU-heavy — every run-src `@test` builds the whole
/// pipeline into its own component), and the gate host is SHARED with the rest of the fleet, so the
/// default cap depends on whether the shared-closure providers were WARMED first (see below): `min(cores,4)`
/// when warm succeeded, else the conservative `min(cores,2)`.
///
/// WHY the cap is COUPLED to the warm outcome (reviewer FYI on the P0 pair): the 4-cap is only safe
/// BECAUSE `step_cached_per_file` warms the providers once before the pool, so each per-file run is a
/// cheap consumer-only cache HIT rather than the heavy cold re-emit. But warm-once is best-effort — if it
/// TIMES OUT under shared-fleet CPU contention (the exact condition the original 4→2 downgrade cited,
/// pr-sync systemic-timeout report batch #124+), the per-file pool runs against a COLD cache, and at 4-way
/// that re-exposes the cold-emit-races-the-cap scenario the 2-cap guarded. So key the default cap to the
/// warm outcome: 4 when warm succeeded (cheap HITs, collapse the sum toward the slowest-file floor —
/// operator P0 gate <10min), 2 when it did NOT (cold sweep gets ~2× CPU/file to finish within the cap).
/// `CDZ_ML_JOBS` still overrides either default (an operator throughput lever — raise on a DEDICATED/idle
/// host); `=0`/garbage falls back to the outcome-based default; clamped to `[1, file_count]`.
fn ml_test_jobs(file_count: usize, warm_succeeded: bool) -> usize {
    // Split env-reading (impure) from the clamping (pure) so the logic is testable without mutating
    // process-global env — `#[test]`s run in parallel, so touching env in a test races sibling tests.
    let override_opt = std::env::var("CDZ_ML_JOBS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0);
    ml_test_jobs_from(override_opt, file_count, warm_succeeded)
}

/// Pure core of [`ml_test_jobs`]: given an already-parsed `CDZ_ML_JOBS` override (`None` = unset /
/// zero / garbage → use the outcome-based default), the file count, and whether warm-once SUCCEEDED,
/// compute the worker-pool size. Kept env-free so it is exercised by a race-free unit test.
fn ml_test_jobs_from(
    override_opt: Option<usize>,
    file_count: usize,
    warm_succeeded: bool,
) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // Cap COUPLED to the warm outcome (reviewer FYI): 4 only when the providers were warmed (each per-file
    // run is then a cheap cache HIT, so 4-way doesn't race the per-file cap — operator P0 gate <10min); 2
    // when warm did NOT succeed (a COLD sweep re-does the heavy per-file re-emit, so keep the conservative
    // cap that gives each file ~2× CPU to finish within the cap — the original 4→2 downgrade's premise
    // still holds on the cold path). This closes the latent regression where a warm-once TIMEOUT under
    // fleet contention would otherwise run a cold sweep at 4-way = the exact race the 2-cap guarded.
    let default = if warm_succeeded {
        cores.clamp(1, 4)
    } else {
        cores.clamp(1, 2)
    };
    let jobs = override_opt.unwrap_or(default);
    jobs.clamp(1, file_count.max(1))
}

/// Like `Child::wait_with_output`, but with a hard wall-clock `timeout`. Returns `Ok(Some(output))` if
/// the child exited within the deadline, `Ok(None)` if it was KILLED for exceeding it (a hang), or the
/// underlying io error. Drains stdout+stderr on reader threads so a child that fills a pipe buffer
/// can't deadlock the wait (the reason we can't just poll `try_wait` on a piped child). On timeout the
/// child is killed and reaped so no zombie/orphan survives — the exact leak that piled up 100+ spinning
/// `cdz-run` procs and forced a manual host recovery.
///
/// `pub(crate)` so the `fleet` submodule can bound its batch pre-filter nix build with the same
/// kill-on-hang guarantee (a hung `local-gate` build must never freeze pr-sync's single-threaded tick).
pub(crate) fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    // Take the pipes and drain each on its own thread — a hung child may still have emitted partial
    // output, and an undrained full pipe would block the child (and us) even after we decide to kill.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    // Poll for exit until the deadline. A short sleep between polls keeps this cheap (a normal case
    // exits in <1s → at most a couple polls) without a busy-loop.
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None => {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };

    match status {
        Some(status) => {
            // Exited in time — join the drain threads for the full captured output.
            let stdout = out_thread.join().unwrap_or_default();
            let stderr = err_thread.join().unwrap_or_default();
            Ok(Some(std::process::Output {
                status,
                stdout,
                stderr,
            }))
        }
        None => {
            // Timed out — kill + reap so no orphan survives, then join the readers (they finish once
            // the killed child's pipes close). The partial output is discarded (a hang has no verdict).
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_thread.join();
            let _ = err_thread.join();
            Ok(None)
        }
    }
}

/// Where the paths to the built pipeline binaries live, resolved once.
struct Tools {
    syntax: PathBuf,
    corpus: PathBuf,
    rcdzc: PathBuf,
    run: PathBuf,
    /// The directory holding the built `cdz-rt` rlib + its deps (`target/<subdir>`), passed to `rustc`
    /// as `-L dependency=<dir> --extern cdz_rt=<dir>/libcdz_rt.rlib` when compiling an emitted async
    /// module (which `use`s the shared `cdz_rt::CdzEnv`). `None` if the rlib build failed / wasn't run.
    cdz_rt_dir: Option<PathBuf>,
    /// The directory holding the built `cdz-num` rlib (`libcdz_num.rlib`) — passed to `rustc` as `-L
    /// dependency=<dir> --extern cdz_num=…` when an emitted (sync or async) program uses `cdz_num::Big`.
    /// `None` if the rlib wasn't built.
    cdz_num_dir: Option<PathBuf>,
    /// The directory holding the built `cadenza-ast` rlib (`libcadenza_ast.rlib`) + its deps — passed to
    /// `rustc` as `-L dependency=<dir> --extern cadenza_ast=…` when an emitted program uses the native R2
    /// value codec (`Core::ValueEncode`/`ValueDecode` emit `cadenza_ast::codec::encode`/`decode`). `None`
    /// if the rlib wasn't built.
    cadenza_ast_dir: Option<PathBuf>,
    /// The committed `DEBUG_RUNTIME_HASH` (parsed from `runtime_abi.rs`'s `None => "…"` default arm) — the
    /// content address of the CANONICAL debug-counters runtime a `(live-objects N)` case must run on. A
    /// stale debug runtime in the store reclaims differently (→ a false live-objects pass/fail), so the
    /// gate verifies the resolved debug runtime against this before asserting balance. `None` if unparsed
    /// (then the check is skipped — can't verify). The release side gets this for free via the
    /// self-describing stamped heap import; the debug side must check explicitly.
    debug_runtime_hash: Option<String>,
}

/// Build the three pipeline tools once (under `profile`) and return their binary paths — shared by
/// `run`/`gate`/`roundtrip`/`fmt`/`emit` so none pays a per-invocation `cargo run` build. The
/// interactive commands use `dev` (fast build); the corpus gate uses `release-debug` (optimized), so
/// that the ~900-case run is not dominated by unoptimized tools.
fn build_tools(paths: &Paths, profile: &str) -> Tools {
    // CDZ_SEED_BIN_DIR override (v-xtask-decompose, operator all-nix mandate 2026-08-28): a directory of
    // PREBUILT tool binaries (cdz / cdz-corpus / cdz-run) supplied by a nix app, so we SKIP the internal
    // `cargo build` of the toolchain below — that self-build is exactly the "rebuild the world" per-worktree
    // cold compile the decomposition is eliminating (a nix app injects the warm-cached seedCompiler+cdzCorpus
    // instead). The rust-gate rlibs (cdz-rt / cdz-num / cadenza-ast) are NOT in this dir, so their dirs are
    // recorded only if present (binary-only consumers — roundtrip / emit / run — never touch them; the
    // `--target rust` gate does NOT set this env, so it still cargo-builds the rlibs below). Unset (the plain
    // `cargo xtask …` path) → the cargo-build below, byte-for-byte unchanged.
    if let Some(dir) = std::env::var_os("CDZ_SEED_BIN_DIR") {
        let bin = PathBuf::from(dir);
        let cdz = bin.join("cdz");
        return Tools {
            syntax: cdz.clone(),
            corpus: bin.join("cdz-corpus"),
            rcdzc: cdz,
            run: bin.join("cdz-run"),
            cdz_rt_dir: bin.join("libcdz_rt.rlib").exists().then(|| bin.clone()),
            cdz_num_dir: bin.join("libcdz_num.rlib").exists().then(|| bin.clone()),
            cadenza_ast_dir: bin
                .join("libcadenza_ast.rlib")
                .exists()
                .then(|| bin.clone()),
            debug_runtime_hash: parse_committed_debug_runtime_hash(&paths.repo),
        };
    }
    let sh = Shell::new().expect("open a shell");
    sh.change_dir(&paths.repo);
    // The front-end + compiler CLIs are now ONE binary, `cdz` (`cdz convert …` / `cdz compile …`);
    // `cdz-corpus` and `cdz-run` stay separate (corpus normalization; wasmtime runner). Build `cdz`
    // in place of the retired `cdz-syntax`/`rcdzc` bins.
    // Also build `cdz-rt` (the shared native runtime interface an emitted ASYNC module links against for
    // `CdzEnv`). It is a plain rlib; the Rust-async gate passes it to `rustc` via `--extern`. Built here
    // once (cheap, tiny crate) so the ~900-case async gate does not rebuild it per case.
    // Also build `cdz-num` (the bignum the rust backend emits `cdz_num::Big` against — it source-shares
    // the runtime's `bigint.rs`). A plain rlib the SYNC rust gate passes to `rustc` via `--extern` when
    // an emitted program uses BigInt. Built here once alongside `cdz-rt`.
    if let Err(e) = cmd!(
        sh,
        "cargo build --quiet --profile {profile} -p cdz -p cdz-corpus -p cdz-run -p cdz-rt -p cdz-num -p cadenza-ast"
    )
    .quiet()
    .run()
    {
        eprintln!("xtask: building the tools failed: {e}");
        std::process::exit(1);
    }
    // Cargo puts the `dev` profile's artifacts under `target/debug`; every other profile lands under
    // `target/<profile>`.
    let subdir = if profile == "dev" { "debug" } else { profile };
    let bin = paths.repo.join("target").join(subdir);
    // `syntax` and `rcdzc` now BOTH point at `cdz`; the call sites supply the subcommand (`convert`
    // is already the first arg at every `syntax` site; a `compile` is prepended at every `rcdzc` site).
    let cdz = bin.join("cdz");
    // The `cdz-rt` rlib lands at `target/<subdir>/libcdz_rt.rlib`; the directory itself is the `-L`
    // search path for `rustc`. Only present when the rlib actually built.
    let cdz_rt_dir = bin.join("libcdz_rt.rlib").exists().then(|| bin.clone());
    // The `cdz-num` rlib (`libcdz_num.rlib`) — the SYNC rust gate links it via `--extern` for a
    // BigInt-using program. Same directory as `cdz-rt`; recorded only when the rlib actually built.
    let cdz_num_dir = bin.join("libcdz_num.rlib").exists().then(|| bin.clone());
    // The `cadenza-ast` rlib (`libcadenza_ast.rlib`) — the rust gate links it via `--extern` when an
    // emitted program uses the native R2 value codec (`cadenza_ast::codec::encode`/`decode`). Same dir as
    // the others; recorded only when the rlib actually built.
    let cadenza_ast_dir = bin
        .join("libcadenza_ast.rlib")
        .exists()
        .then(|| bin.clone());
    // The committed DEBUG_RUNTIME_HASH — the canonical debug-counters runtime a `(live-objects N)` case
    // must run on. xtask is compiler-free (no rcdzc dep), so read the value from the codegen'd source
    // (like the flake's parity check), never via rcdzc; parsed, not hashed — no IFD / no rebuild.
    let debug_runtime_hash = parse_committed_debug_runtime_hash(&paths.repo);
    Tools {
        syntax: cdz.clone(),
        corpus: bin.join("cdz-corpus"),
        rcdzc: cdz,
        run: bin.join("cdz-run"),
        cdz_rt_dir,
        cdz_num_dir,
        cadenza_ast_dir,
        debug_runtime_hash,
    }
}

/// Parse a committed runtime-hash const's `None => "<hash>"` default arm from the codegen'd source.
/// xtask must NOT depend on the compiler crate (lean-xtask mandate; and reading the const *value* at
/// xtask's own compile would pick up whatever `CDZ_*` env was injected into xtask's build, not the
/// committed canonical), so it reads the committed value from SOURCE — like the flake's parity check.
/// The three consts (`REQUIRED_RUNTIME_HASH` / `DEBUG_RUNTIME_HASH` / `REQUIRED_NFC_HASH`) live in
/// `cadenza-compile-abi/src/runtime_hash.rs` (rcdzc's `runtime_abi.rs` only `pub use`s them now — the
/// consts MOVED there, which is why the earlier `runtime_abi.rs`-targeted parse returned `None`). Scope
/// to `pub const <const_name>`, then take the first quoted string after its `None =>` arm. Parsed, not
/// hashed — no IFD / no rebuild. Returns `None` if the file / const / default arm is absent.
fn parse_committed_hash(repo: &Path, const_name: &str) -> Option<String> {
    let src = std::fs::read_to_string(
        repo.join("implementation/seed/crates/cadenza-compile-abi/src/runtime_hash.rs"),
    )
    .ok()?;
    let after_const = src.split(&format!("pub const {const_name}")).nth(1)?;
    let after_none = after_const.split("None =>").nth(1)?;
    let open = after_none.find('"')? + 1;
    let rest = &after_none[open..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

/// The committed canonical RELEASE runtime hash (`REQUIRED_RUNTIME_HASH`) — the hash a nix-built compiler
/// pins and a compiled program imports its runtime by. Used to WARN when a native `cargo xtask build`
/// produces a divergent (native-unfaithful) release runtime a nix-`cdz`-compiled program can't resolve.
fn parse_committed_runtime_hash(repo: &Path) -> Option<String> {
    parse_committed_hash(repo, "REQUIRED_RUNTIME_HASH")
}

/// The committed `DEBUG_RUNTIME_HASH` — the canonical debug-counters runtime a `(live-objects N)` case
/// must run on (located by content address). See `parse_committed_hash`.
fn parse_committed_debug_runtime_hash(repo: &Path) -> Option<String> {
    parse_committed_hash(repo, "DEBUG_RUNTIME_HASH")
}

/// The outcome of driving one program (sexpr text) through the pipeline.
enum Ran {
    /// Ran to a value, rendered to canonical text, plus the OBSERVED HOST CALLS (each a dotted `E.op`, in
    /// call order — from cdz-run's `host-call` stderr lines). The observed sequence is verified against a
    /// case's `(host-calls …)`; empty for a program that makes no host call (the common shape). The third
    /// field is the SET of compile WARNINGS (`(code, message)`) captured from the clean-compile stderr —
    /// a `(warns CODE (message …))` clause grades a PRESENCE check over it (operator seq353 inc2); empty
    /// on the non-emitting/differential paths that don't capture compile stderr.
    Value(String, Vec<String>, Vec<(String, String)>),
    /// The compiler rejected/declined the program. `code` is the diagnostic CODE the compiler emitted
    /// (`Some("CDZ0210")`) — a TYPED rejection the corpus can match against `(error CODE)` — or `None`
    /// for a codeless DECLINE (an unimplemented construct: `Reject::decline`), which grades as `todo`
    /// (not-yet-built), never a disagreement. This is the "grade by what the compiler DOES" rule applied
    /// to rejections: a coded reject is a decision to check, a codeless decline is a gap to fill.
    /// `message` is the diagnostic PROSE recovered from `cdz compile` stderr (empty when none / not on
    /// a stderr-capturing path) — the corpus `(error CODE (message "…"))` / `(declines (message "…"))`
    /// clause grades a load-bearing SUBSTRING of it, case-sensitive, no normalization (v-diagnostics
    /// ruling: messages are single-source single-line, case is load-bearing). The portable-diagnostic-
    /// test capability (operator seq353); absent clause = code-only grading, unchanged.
    Declined {
        code: Option<String>,
        message: String,
    },
    /// The component ran but trapped.
    Trap(String),
    /// The backend produced an artifact that FAILED TO BUILD/LOAD — a broken artifact, distinct from a
    /// clean decline (the compiler refused up front) and from a trap (a well-formed artifact that ran
    /// and aborted). Only the Rust backend produces this today: emitted `.rs` that `rustc` rejects
    /// (e.g. the narrow-literal width mismatch that motivated Rust-backend gating). It grades as a FAIL
    /// on an `output`/`trap` case (the backend was asked for a value and emitted un-compilable source —
    /// the miscompile the gate must catch), the way a wrong `Value` does.
    BadArtifact(String),
}

/// Which backend the gate drives each corpus program through. `Wasm` is the default and historical
/// path (cdz-syntax → rcdzc → cdz-run); `Rust` emits `--target rust`, compiles the source with `rustc`,
/// and runs it — so the SAME corpus expectations grade the Rust backend, catching a non-compiling
/// artifact or a wrong answer against the one executable oracle (`backends-and-targets.md` §The meaning
/// against which every backend's output is judged MUST be the one executable semantics).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GateTarget {
    Wasm,
    Rust,
    /// The ASYNC/gas-metered Rust backend: emit `--target rust-async`, wrap it with a no-limit gas `Env`
    /// and a minimal executor, and drive the export under `block_on` — so the SAME corpus expectations
    /// grade the async form (its answers must match, gas threading and all).
    RustAsync,
    /// The CADENZA-BACKEND ROUND-TRIP: emit the OPTIMIZED program BACK to a Cadenza surface (`--target
    /// cadenza`), then RECOMPILE that surface through the normal wasm path and run it — grading the SAME
    /// corpus expectations against the round-tripped value. This is the CI witness for v-cadenza-backend's
    /// core invariant (the emitted surface must recompile + be value-equivalent); a case whose ORIGINAL
    /// does not compile to wasm is SHARED (a language/lowering limit, not a backend gap) → declines, and a
    /// case the cadenza backend cannot yet emit declines too — only an emitted surface that fails to
    /// recompile or gives a wrong value is a real break. Mirrors the standalone `hop2_validate` harness.
    Cadenza,
}

/// The `--target` value clap parses for the `gate` command (its own enum so clap validates the
/// spelling and `--help` lists the choices), mapped to [`GateTarget`] at dispatch.
#[derive(Clone, Copy, clap::ValueEnum)]
enum GateTargetArg {
    Wasm,
    Rust,
    RustAsync,
    Cadenza,
}

/// How the wasm gate checks a case's post-run HEAP BALANCE — the corpus opt-out heap-liveness model. The
/// live-cell count is read on the debug-counters value-heap runtime (`cdz-run --report-live-objects`) after
/// the run; the check is applied only to a HEAP-importing case (a scalar/const program has no heap to
/// balance and is always skipped, never a false fail).
#[derive(Clone, Copy)]
enum LiveObjectsCheck {
    /// Never check the balance — for the opt-sweep (a TIER divergence, not a heap regression) and the ML
    /// differential (a pure value comparison against the wasm oracle). `--report-live-objects` is not even
    /// passed, so host-call capture on the normal run path is preserved.
    Off,
    /// OPT-OUT DEFAULT (a case with no `(live-objects …)` clause): a heap-importing case must end at 0 live
    /// cells (no leak / no double-free); a no-heap case is skipped.
    Default,
    /// Assert the live-cell count == N — an explicit `(live-objects N)` CLEAN residual (the reachable-return
    /// count; N=0 = fully reclaimed). A `(live-objects known-leak)` case is NOT this — it maps to `Off`.
    Expect(u32),
}

impl LiveObjectsCheck {
    /// Map a corpus record to the gate check under seq-15 PURE-BINARY leak semantics: a KNOWN-LEAK case is
    /// NOT count-checked (`Off` — its leak magnitude does not matter, per the operator ruling); a CLEAN case
    /// asserts its EXACT residual (explicit count ⇒ `Expect(n)`, no clause ⇒ `Default` = heap ⇒ 0).
    fn from_record(count: Option<u32>, known_leak: bool) -> Self {
        if known_leak {
            LiveObjectsCheck::Off
        } else {
            match count {
                Some(n) => LiveObjectsCheck::Expect(n),
                None => LiveObjectsCheck::Default,
            }
        }
    }

    /// The expected live-cell count for a HEAP-importing case, or `None` to not check the balance at all.
    fn expected_for_heap(self) -> Option<u32> {
        match self {
            LiveObjectsCheck::Off => None,
            LiveObjectsCheck::Default => Some(0),
            LiveObjectsCheck::Expect(n) => Some(n),
        }
    }
}

/// Drive one program's s-expression `text` through cdz-syntax → rcdzc → cdz-run, returning the
/// outcome. Uses a real pipe with the program fed on cdz-syntax's stdin (no temp files). When `call`
/// is given, the export is invoked with those runtime arguments (`--call <export> --arg <v>…`) — how a
/// case exercises a parameterized entrypoint rather than a nullary one; `None` runs the sole export
/// with no arguments (the common case).
#[allow(clippy::too_many_arguments)] // host protocol + wit-world, threaded alongside the pipeline args
fn run_program(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    modules: &[(String, String)],
    peers: &[(String, String)],
    call: Option<&Call>,
    host_responses: &[(String, String)],
    host_calls: &[String],
    wit_world: Option<&str>,
    component_name: Option<&str>,
    live_objects: LiveObjectsCheck,
    target: GateTarget,
) -> Ran {
    // A case that imposes an explicit WIT world (general WIT-ABI shape) is driven only through the WASM
    // path (the authoritative WIT-ABI boundary check). The Rust/ML backends have no external-world ingest
    // on this path, so a wit-world case is not-yet-supported there → DECLINE (Todo, coverage-not-yet),
    // never a disagreement. (The wasm path handles it; the others stay byte-identical for non-world cases.)
    // The HEAP-BALANCE check is WASM-only (the debug-counters `live-objects` export has no Rust/ML analog),
    // but under the opt-out model it applies to essentially every heap case — so it does NOT gate whether a
    // case runs on the other backends: they simply ignore `live_objects` and grade the value/trap outcome.
    if wit_world.is_some() && !matches!(target, GateTarget::Wasm) {
        return Ran::Declined {
            code: None,
            message: String::new(),
        };
    }
    // A `(then …)` two-call, a `(drop)`, or a `(call-method …)` value-resource case is driven ONLY through
    // the WASM harness — the two-call drive (`--call-twice`), the explicit resource-drop (`--drop-handle`),
    // and the named value-resource member reach (`--call-member`) all live in the wasm closure/escape
    // driver. The Rust/ML backends have no such resource drive on this path, so those cases are
    // not-yet-supported there → DECLINE (Todo, coverage-not-yet). Without this, a `(then)` backend runs only
    // the FIRST call (scalar `15` vs the expected `(tuple 15 17)`), a spurious disagreement; the decline
    // makes such a case baseline pass-wasm / todo-elsewhere, the same wasm-only convention `wit-world` uses.
    if call
        .map(|c| c.second_call.is_some() || c.drop_handle || c.method.is_some())
        .unwrap_or(false)
        && !matches!(target, GateTarget::Wasm)
    {
        return Ran::Declined {
            code: None,
            message: String::new(),
        };
    }
    // A CROSS-COMPONENT `(peer …)` case is driven ONLY through the WASM harness — the peer is a separately-
    // compiled component composed at run via `cdz-run --peer`/`run_with_peers`. The Rust/ML backends have no
    // cross-component composition on this path, so a peer case is not-yet-supported there → DECLINE (Todo),
    // the same wasm-only convention `wit-world` uses.
    if !peers.is_empty() && !matches!(target, GateTarget::Wasm) {
        return Ran::Declined {
            code: None,
            message: String::new(),
        };
    }
    // A `(peer …)` case binds its interface with the imposed-world consumer form `(effect E …) (bind E
    // "iface") (host (E) …)`; an EXPLICIT `(wit-world …)` clause ALONGSIDE a peer is UNSUPPORTED — the
    // consumer's declared world and the peer wiring disagree, and `cdz-run` composes an UNPARSEABLE
    // component ("invalid component: failed to parse WebAssembly module") with no hint at the cause
    // (breaker's F2 differential burned a full bisection on exactly this). Reject the combination UP FRONT
    // with an actionable message that names the fix, instead of letting the opaque compose failure through.
    // Bind-only is THE peer form (see `spec/semantics/29-cross-component-peers.sexp`'s authoring rules); no
    // corpus case pairs a peer with a wit-world, so this only fires on a MISAUTHORED case, turning a
    // confusing invalid-component crash into a clear decline-with-a-route-to-a-fix.
    if !peers.is_empty() && wit_world.is_some() {
        return Ran::BadArtifact(
            "a (peer …) case must bind its interface with (bind E \"iface\") in the consumer, not declare \
             an explicit (wit-world …): the wit-world+peer combination composes an unparseable component. \
             Remove the (wit-world …) clause and use the bind-only consumer form."
                .to_string(),
        );
    }
    match target {
        GateTarget::Wasm => run_program_wasm(
            tools,
            store,
            program,
            modules,
            peers,
            call,
            host_responses,
            None,
            wit_world,
            component_name,
            live_objects,
        ),
        // Both Rust backends render a host call the SAME way: the emitted `mod prog` calls
        // `crate::__cdz_host_<key>()` and `run_program_rust` generates those shim fns from `host_responses`
        // (a non-host case passes empty slices → byte-identical to before). A UNIT-result effect op has no
        // response but IS in host_calls (H8), so thread host_calls through too. The host SHIMS are ordinary
        // sync fns; an emitted async fn calls them synchronously (a host op charges no gas — only the
        // enclosing async fn's entry `consume_boxed` does), so the async path needs NO async-shim variant —
        // the SAME `host_responses`/`host_calls` protocol threads through with `async_mode=true`. (Option A's
        // uniform-env emit already renders the async `Core::HostCall`; this just lets the harness DRIVE it,
        // instead of the prior blanket async-host decline that todo'd the whole host/@param async frontier.)
        GateTarget::Rust => run_program_rust(
            tools,
            program,
            modules,
            call,
            false,
            None,
            host_responses,
            host_calls,
        ),
        GateTarget::RustAsync => run_program_rust(
            tools,
            program,
            modules,
            call,
            true,
            None,
            host_responses,
            host_calls,
        ),
        GateTarget::Cadenza => run_program_cadenza(
            tools,
            store,
            program,
            modules,
            call,
            host_responses,
            live_objects,
        ),
    }
}

/// Drive one program through the CADENZA-BACKEND ROUND-TRIP (`GateTarget::Cadenza`): emit the optimized
/// program back to a Cadenza SURFACE (`cdz compile --target cadenza`), then RECOMPILE that surface through
/// the normal wasm path ([`run_program_wasm`]) and run it — so the case's recorded expectation grades the
/// round-tripped value, exactly as `hop2_validate` does out-of-band. The grading contract:
/// - a MULTI-FILE package case is not yet round-tripped → DECLINE (Todo, coverage-not-yet);
/// - a case whose ORIGINAL does not compile to wasm is SHARED (a language/lowering limit, NOT a backend
///   gap — the standard wasm gate already records it) → DECLINE, so it never counts as a cadenza break;
/// - a case the cadenza backend cannot yet EMIT (hop1 declines) → DECLINE (Todo);
/// - only an emitted surface that fails to RECOMPILE, traps, or yields a wrong value is a real break — that
///   flows through as the wasm run's `Ran`.
///
/// (`wit_world`/`peer`/two-call/drop/method cases already declined for every non-`Wasm` target upstream in
/// [`run_program`], so they never reach here.)
fn run_program_cadenza(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    modules: &[(String, String)],
    call: Option<&Call>,
    host_responses: &[(String, String)],
    live_objects: LiveObjectsCheck,
) -> Ran {
    // Multi-file package round-trip is a later increment — decline (Todo), never a spurious break.
    if !modules.is_empty() {
        return Ran::Declined {
            code: None,
            message: String::new(),
        };
    }
    // Precondition: the ORIGINAL program must compile to wasm. If it doesn't, this is a SHARED gap (the
    // standard wasm gate owns it), NOT a cadenza-backend round-trip break → decline so it stays uncounted.
    if emit_component_single_at(tools, program, None, None, None).is_err() {
        return Ran::Declined {
            code: None,
            message: String::new(),
        };
    }
    // hop1: emit the optimized program BACK to a Cadenza surface (sexpr text). A decline here = the cadenza
    // backend cannot yet re-emit this form → Todo (coverage-not-yet), not a disagreement.
    let surface = match emit_cadenza_surface(tools, program) {
        Some(s) => s,
        None => {
            return Ran::Declined {
                code: None,
                message: String::new(),
            };
        }
    };
    // hop2: recompile the emitted surface through the normal wasm path and run it. A recompile failure /
    // trap / wrong value here is a REAL cadenza round-trip break (the whole point of this target).
    run_program_wasm(
        tools,
        store,
        &surface,
        &[],
        &[],
        call,
        host_responses,
        None,
        None,
        None,
        live_objects,
    )
}

/// Emit the OPTIMIZED program back to a Cadenza SURFACE as sexpr text: sexpr → binary AST (`cdz-syntax
/// convert`) → `cdz compile --target cadenza` (the cadenza binary AST) → sexpr (`cdz convert --from binary
/// --to sexpr`). Returns the surface text, or `None` if any stage fails (hop1 decline / a hang). The sexpr
/// form is then re-parseable source for the wasm recompile leg — a true round-trip through the same pipeline.
fn emit_cadenza_surface(tools: &Tools, program: &str) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Stage 1: sexpr text (stdin) → binary AST (stdout).
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", "sexpr", "--to", "binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    syntax
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .ok();

    // Stage 2: binary AST → cadenza binary AST (`compile --target cadenza`); capture nothing but the bytes.
    let compile = Command::new(&tools.rcdzc)
        .args(["compile", "--target", "cadenza", "-", "-o", "-"])
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let compile_out =
        match wait_with_timeout(compile, run_timeout()).expect("wait rcdzc -t cadenza") {
            Some(out) => out,
            None => {
                let _ = syntax.wait();
                return None;
            }
        };
    let _ = syntax.wait();
    if !compile_out.status.success() {
        return None; // hop1 declined — the cadenza backend cannot re-emit this form yet.
    }

    // Stage 3: cadenza binary AST (stdin) → sexpr text (stdout) — the re-parseable surface.
    let mut convert = Command::new(&tools.syntax)
        .args(["convert", "--from", "binary", "--to", "sexpr", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    convert
        .stdin
        .take()
        .unwrap()
        .write_all(&compile_out.stdout)
        .ok();
    let convert_out = wait_with_timeout(convert, run_timeout()).expect("wait cdz convert")?;
    if !convert_out.status.success() {
        return None;
    }
    String::from_utf8(convert_out.stdout).ok()
}

/// Drive one program through cdz-syntax → rcdzc (wasm) → cdz-run — the historical path. A multi-file
/// PACKAGE case (`modules` non-empty) instead writes the entry + library files to a temp dir and runs
/// `cdz compile <files> --entry main` (the package path); either way the emitted component is run the
/// same way.
#[allow(clippy::too_many_arguments)]
fn run_program_wasm(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    modules: &[(String, String)],
    peers: &[(String, String)],
    call: Option<&Call>,
    host_responses: &[(String, String)],
    opt_level: Option<&str>,
    wit_world: Option<&str>,
    component_name: Option<&str>,
    live_objects: LiveObjectsCheck,
) -> Ran {
    use std::io::Write;
    use std::process::Stdio;

    // Emit the component bytes — either the single-file pipe or the multi-file package compile.
    // `opt_level` selects the pass tier (only the opt-sweep passes a level; `None` = compiler default).
    // A `(wit-world …)` case imposes an explicit export world (single-file only for now).
    let component = if modules.is_empty() {
        emit_component_single_at(tools, program, opt_level, wit_world, component_name)
    } else {
        emit_component_package(tools, program, modules, opt_level)
    };
    let (component, warnings) = match component {
        Ok(bytes_and_warnings) => bytes_and_warnings,
        Err(ran) => return ran,
    };

    // CROSS-COMPONENT PEERS: compile each peer program to its OWN standalone component (the same
    // single-file emit as the entry — NOT linked like a module), write it into a temp dir, and pass
    // `--peer <iface>=<path>` so cdz-run composes it with the entry via `run_with_peers`. The temp dir
    // must outlive the run (its `.wasm` files are read at instantiate), so it is removed only after the
    // run completes (below). Empty for a single-component case (no `--peer`, byte-identical to before).
    let mut peer_paths: Vec<(String, PathBuf)> = Vec::new();
    let peer_dir: Option<PathBuf> = if peers.is_empty() {
        None
    } else {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICK: AtomicU64 = AtomicU64::new(0);
        let tick = TICK.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cdz-peers-{}-{tick}", std::process::id()));
        if std::fs::create_dir_all(&dir).is_err() {
            return Ran::BadArtifact("could not create a temp peer dir".into());
        }
        for (i, (iface, pprog)) in peers.iter().enumerate() {
            // The peer EXPORTS `iface`, so compile it with `--component-name <iface>` (the mechanism a
            // source program uses to export through a named interface instance). A peer whose interface
            // shape isn't inferable from its exports alone may additionally need a wit-world — that surface
            // is TBD pending the source-provider shape (coordinating with v-wm); component-name is required
            // regardless.
            let (peer_bytes, _warnings) =
                match emit_component_single_at(tools, pprog, opt_level, None, Some(iface)) {
                    Ok(bw) => bw,
                    Err(ran) => {
                        let _ = std::fs::remove_dir_all(&dir);
                        return ran;
                    }
                };
            let path = dir.join(format!("peer-{i}.wasm"));
            if std::fs::write(&path, &peer_bytes).is_err() {
                let _ = std::fs::remove_dir_all(&dir);
                return Ran::BadArtifact("could not write a peer component".into());
            }
            peer_paths.push((iface.clone(), path));
        }
        Some(dir)
    };

    // Stage 3: run the component (its stdout is the value; a trap goes to stderr with exit 1).
    let mut run = std::process::Command::new(&tools.run);
    run.arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = store {
        run.arg("--store").arg(dir);
    }
    // Each cross-component peer: `--peer <iface>=<path>` (the compiled peer component). cdz-run composes
    // them with the entry (`run_with_peers`). Absent for a single-component case.
    for (iface, path) in &peer_paths {
        run.arg("--peer").arg(format!("{iface}={}", path.display()));
    }
    // HEAP-BALANCE (corpus opt-out heap-liveness): every HEAP-importing wasm case must end at its expected
    // live-cell count. cdz-run detects the heap import and emits a `live-objects\t<N>` line ONLY for a heap
    // case (a scalar/const program emits none → the balance check is skipped, never a false fail), so we
    // pass `--report-live-objects` for every case whose check is not `Off`. The count is meaningful only on
    // the DEBUG-COUNTERS runtime (its `live-objects` export reports the real count; the shipped runtime
    // always reports 0), so override the runtime with `--runtime <debug-path>`. If the debug runtime is not
    // in the store, or it is STALE (not the committed DEBUG_RUNTIME_HASH), the balance check is SKIPPED
    // (with a loud stderr note) rather than mass-declining every heap case — the value/trap grade still
    // runs. (A stale/missing debug runtime is separately caught by the DEBUG_RUNTIME_HASH parity check and
    // the freshness protocol; #3753's stale-runtime detection is kept here to drive the skip.)
    // GUARDED-ALL (`gate --guarded-all`, memory-safety escape verification): route EVERY case through the
    // debug-counters runtime — not just the live-objects-clause cases — so the runtime's generation guard
    // (`assert_node_live`, cdz-runtime/src/rc.rs) fires corpus-wide. That guard is the deterministic
    // under-retain / use-after-free WITNESS: on free the debug runtime bumps the node generation odd +
    // retains the freed shell, so a later dup/drop/access of a freed cell TRAPS (→ a fail against the
    // expected value/trap). The shipped/release runtime deallocates with NO guard, so a UAF there is
    // silent UB — which is why a global escape change must be verified on the debug runtime, not release
    // (and the release live-objects census reports 0 vacuously = a false-clean; see grade.rs). The gate
    // ENTRY has already fail-fast-verified the debug runtime is present + fresh under `--guarded-all`, so
    // here a stale/missing runtime only affects the (independent) census cases.
    let guarded_all = std::env::var_os("CDZ_GATE_GUARDED_ALL").is_some();
    let want_census = !matches!(live_objects, LiveObjectsCheck::Off);
    let mut report_live = false;
    if want_census || guarded_all {
        match resolve_debug_runtime(store) {
            Some(path) => {
                let stale = tools
                    .debug_runtime_hash
                    .as_deref()
                    .zip(path.file_stem().and_then(|s| s.to_str()))
                    .map(|(committed, store_hash)| store_hash != committed)
                    .unwrap_or(false);
                if stale {
                    // `--guarded-all` already exited at entry on a stale runtime; this note is for the
                    // census path only (a stale runtime skips the heap-balance count, not the value grade).
                    if want_census {
                        eprintln!(
                            "xtask gate: SKIPPING heap-balance check — STALE debug runtime in store \
                             (!= committed DEBUG_RUNTIME_HASH); run `cargo xtask build`"
                        );
                    }
                } else {
                    // NFC (the runtime's inline dependency) resolves from the store; if the gate passed no
                    // `--store`, point it at the debug runtime's own store dir so NFC + the debug runtime
                    // come from the same build.
                    if store.is_none()
                        && let Some(parent) = path.parent()
                    {
                        run.arg("--store").arg(parent);
                    }
                    // The debug runtime activates the generation guards (guarded-all's whole point);
                    // `--report-live-objects` + the count assertion stay gated on an actual census clause.
                    run.arg("--runtime").arg(&path);
                    if want_census {
                        run.arg("--report-live-objects");
                        report_live = true;
                    }
                }
            }
            None => {
                if want_census {
                    eprintln!(
                        "xtask gate: SKIPPING heap-balance check — debug-counters runtime not in store \
                         (run `cargo xtask build`)"
                    );
                }
            }
        }
    }
    // A `(call …)` case names the export and passes runtime arguments; cdz-run coerces each `--arg` to
    // the export's declared parameter type (its `--arg` allows a leading `-`, so a negative value is
    // taken as the argument, not a flag).
    if let Some(call) = call {
        if let Some(member) = &call.method {
            // A `(call-method <member>)` case has no export: the program's sole producer makes the value-
            // resource (cdz-run routes to the resource-escape path with no `--call`), and `--call-member`
            // names the member to reach on it (instead of the default `encode`). Args are the member's.
            run.arg("--call-member").arg(member);
            for arg in &call.args {
                run.arg("--arg").arg(arg);
            }
        } else {
            // A `(wit-world …)` case's guest exports THROUGH the named interface instance, so qualify the
            // export as `<iface>#<export>` (cdz-run resolves the interface-nested member); a synthesized-world
            // case (no component-name) calls the bare top-level export as before.
            let export = match component_name {
                Some(iface) => format!("{iface}#{}", call.export),
                None => call.export.clone(),
            };
            run.arg("--call").arg(&export);
            for arg in &call.args {
                run.arg("--arg").arg(arg);
            }
        }
        // A `(then …)` continuation drives a SECOND call on the same closure handle (borrow<t>
        // repeatability): `--call-twice` puts cdz-run in two-call mode (make ONCE, call twice, render the
        // pair as a tuple), and each `--then-arg` is a second-call argument. A nullary `(then)` passes
        // `--call-twice` alone. Absent for the ordinary one-call form.
        if let Some(second) = &call.second_call {
            run.arg("--call-twice");
            for arg in second {
                run.arg("--then-arg").arg(arg);
            }
        }
        // A `(drop)` clause: resource-drop the minted closure handle after the call(s), before the run
        // reads the result / heap balance — so a `(live-objects 0)` case pins release (default holds the
        // handle → the known leak of 1).
        if call.drop_handle {
            run.arg("--drop-handle");
        }
    }
    // HOST-CALL RESPONSES (E2h): a program that delegates an effect to the host consumes these in order.
    // Each `(op, value)` becomes `--host-response op=value`; `cdz-run` binds the imported ops to return
    // them. Empty for a non-host case (no flags added → byte-identical invocation to before).
    for (op, value) in host_responses {
        run.arg("--host-response").arg(format!("{op}={value}"));
    }
    let mut child = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));
    child.stdin.take().unwrap().write_all(&component).ok();
    let waited = wait_with_timeout(child, run_timeout()).expect("wait cdz-run");
    // The peers' `.wasm` files were read at instantiate; the child has now exited, so drop the temp peer
    // dir before grading (covers every return path below, including the timeout branch).
    if let Some(dir) = &peer_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
    let run_out = match waited {
        Some(out) => out,
        // A runaway program (an infinite loop, or a runtime that never returns) — killed at the
        // deadline. Grade it as a trap-with-reason "timeout": on an `(output …)` case it FAILs (a value
        // was expected), and a `(trap …)` case reason-matches only if it literally expects "timeout".
        None => return Ran::Trap("timeout (hang)".to_string()),
    };
    if run_out.status.success() {
        // cdz-run prints the OBSERVED host calls to stderr as `host-call\t<op>` lines, in call order;
        // parse them so the case's `(host-calls …)` can be verified. Empty for a non-host program.
        let observed = observed_host_calls(&run_out.stderr);
        let stdout = String::from_utf8_lossy(&run_out.stdout);
        // With `--report-live-objects`, cdz-run appends a `live-objects\t<M>` line after the value FOR A
        // HEAP-IMPORTING CASE (a no-heap program emits none). Split it off; the value is everything before.
        let mut lo: Option<u32> = None;
        let mut value_lines: Vec<&str> = Vec::new();
        for l in stdout.lines() {
            if let Some(n) = l.strip_prefix("live-objects\t") {
                lo = n.trim().parse::<u32>().ok();
            } else {
                value_lines.push(l);
            }
        }
        let value = value_lines.join("\n").trim().to_string();
        match lo {
            // A heap case reported its balance: compare to the expected count (opt-out default 0, or the
            // case's explicit / known-leak N). A mismatch is a synthesized trap that grade_ran surfaces
            // against the expected `(output …)` — the heap-balance assertion.
            Some(m) if report_live => match live_objects.expected_for_heap() {
                Some(expected) if m == expected => Ran::Value(value, observed, warnings),
                Some(expected) => Ran::Trap(format!(
                    "live-objects mismatch: expected {expected}, got {m}"
                )),
                // `report_live` is only set when the check is not Off, so `expected_for_heap` is Some here.
                None => Ran::Value(value, observed, warnings),
            },
            // No live-objects line (a no-heap case, or the balance check was skipped) → value-only grade.
            _ => Ran::Value(value, observed, warnings),
        }
    } else {
        // The trap reason is on cdz-run's `<prog>: trap: <reason>` stderr line. With `--report-live-objects`
        // an informational `live-objects run on …` diagnostic PRECEDES it, and host-call lines may too — so
        // scan for the trap line rather than blindly taking the first stderr line.
        let stderr = String::from_utf8_lossy(&run_out.stderr);
        if let Some(l) = stderr.lines().find(|l| l.contains(": trap: ")) {
            return Ran::Trap(l.to_string());
        }
        // No `: trap: ` line. Distinguish a SILENT-DEATH crash from a clean run-failure diagnostic (breaker's
        // B1-sibling). A CRASH (the run STARTED — the informational `live-objects run on value-heap runtime`
        // provenance banner is printed pre-invoke — then DIED) leaves stdout empty and NOTHING on stderr but
        // that banner (+ any `host-call`/`host-arg` trace lines) — no meaningful diagnostic at all. A clean
        // run failure (a compose/instantiate REJECTION — `cdz-run: peer … mismatch`/does-not-export — or an
        // exhausted host-response) emits a STRUCTURED diagnostic line that corpus cases legitimately pin. So:
        // strip the known informational banner + host-call traces; if a meaningful diagnostic REMAINS, keep the
        // old `Ran::Trap(first_line)` (byte-identical — the rejection cases stay reason-matched); if NOTHING
        // remains, the run died without output → a BadArtifact ICE-class failure with an honest label (the run
        // face of the artifact-ICE: the compiler said yes and the run produced garbage), never a misleading
        // `Ran::Trap(<stray banner>)`.
        if run_failure_has_diagnostic(&stderr) {
            Ran::Trap(first_line(&run_out.stderr))
        } else {
            Ran::BadArtifact(format!(
                "run died without output — a crash mid-run with no trap or diagnostic ({})",
                run_out
                    .status
                    .code()
                    .map(|c| format!("exit {c}"))
                    .unwrap_or_else(|| "killed by signal".into())
            ))
        }
    }
}

/// Resolve the DEBUG-COUNTERS value-heap runtime file in the content-addressed store — its `live-objects`
/// export reports the real live-cell count (the shipped runtime always returns 0). Reads the store's
/// `runtime.toml` `debug_runtime = "<hash>"` line and returns `<store>/<hash>.wasm` if it exists. `None`
/// when no store is configured, no `runtime.toml`, no `debug_runtime` line, or the file is absent — the
/// caller then declines the `(live-objects …)` case (Todo) rather than checking a vacuous balance.
fn resolve_debug_runtime(store: &Option<PathBuf>) -> Option<PathBuf> {
    // Candidate stores: the explicit `--store` if given; else `$CDZ_STORE` then the default
    // `target/cadenza-store` (where `cargo xtask build` writes the debug-counters runtime — the release
    // gate leaves `--store` unset and relies on cdz-run's own resolution for the SHIPPED runtime, but the
    // debug runtime only lives in the built target store, so probe it explicitly).
    let candidates: Vec<PathBuf> = match store {
        Some(dir) => vec![dir.clone()],
        None => {
            let mut v = Vec::new();
            if let Ok(s) = std::env::var("CDZ_STORE") {
                v.push(PathBuf::from(s));
            }
            v.push(PathBuf::from("target/cadenza-store"));
            v
        }
    };
    for dir in candidates {
        let Ok(toml) = std::fs::read_to_string(dir.join("runtime.toml")) else {
            continue;
        };
        let hash = toml.lines().find_map(|l| {
            l.trim()
                .strip_prefix("debug_runtime")?
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches('"')
                .to_string()
                .into()
        });
        let Some(hash) = hash else { continue };
        let path = dir.join(format!("{hash}.wasm"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// The single-file component-emit path: program text (stdin) → binary AST → component (stdout), via
/// the `cdz convert | cdz compile` pipe. `Err(Ran::Declined)` on a rejection/decline (its code
/// recovered from stderr).
/// A successful component emit: the component bytes + the SET of compile warnings (`(code, message)`)
/// captured from the clean-compile stderr — what the `(warns …)` clause grades against (operator
/// seq353 inc2). `Err(Ran)` on the failure side carries the decline/trap.
type EmittedComponent = (Vec<u8>, Vec<(String, String)>);

/// The single-file component-emit path at an optimization level — `opt_level` is the `cdz compile
/// --opt-level` value (`"O0"`..`"O3"`), or `None` for the compiler default (`O1`). Only the
/// opt-level-equivalence sweep ([`gate_opt_sweep`]) passes a level; the normal gate passes `None` so its
/// behavior is byte-identical to before.
fn emit_component_single_at(
    tools: &Tools,
    program: &str,
    opt_level: Option<&str>,
    wit_world: Option<&str>,
    component_name: Option<&str>,
) -> Result<EmittedComponent, Ran> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // A `(wit-world …)` case imposes an EXTERNAL declared world (+ optional component-name), so the guest
    // + world must be compiled as separate ARTIFACTS (a stdin pipe carries only one) — delegate to the
    // world-aware path. The common synthesized-world case (no world) stays the stdin pipe below.
    if let Some(world) = wit_world {
        return emit_component_with_world(tools, program, world, component_name, opt_level);
    }

    // Stage 1: program text (stdin) → binary AST (stdout).
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", "sexpr", "--to", "binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    syntax
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .ok();

    // Stage 2: AST → component; capture stderr so a decline carries its diagnostic. When a level is
    // requested, pass it through — `cdz compile --opt-level <L>` selects the pass tier (the default O1 is
    // what the bare `["compile", "-", "-o", "-"]` uses).
    let mut compile_args: Vec<&str> = vec!["compile", "-", "-o", "-"];
    if let Some(level) = opt_level {
        compile_args.push("--opt-level");
        compile_args.push(level);
    }
    // A component-name WITHOUT a wit-world (a bare peer/provider): the guest's exports cross THROUGH the
    // named interface instance, its shape synthesized from the exported fn signatures (the wit-world path
    // above handles the explicit-world case). `cdz compile --component-name <iface>` on the stdin pipe.
    if let Some(cn) = component_name {
        compile_args.push("--component-name");
        compile_args.push(cn);
    }
    let rcdzc = Command::new(&tools.rcdzc)
        .args(&compile_args)
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = match wait_with_timeout(rcdzc, run_timeout()).expect("wait rcdzc") {
        Some(out) => out,
        // A COMPILE-HANG (the known `and/or/not` / `Any`-parse-if bugs spin the compiler forever) —
        // killed at the deadline. This is a FAIL, not a codeless decline: a hang is a real compiler bug
        // the gate must surface, so map it to a trap-reason "compile timeout" (fails an `(output …)`
        // case) rather than `Declined` (which would grade the hang as harmless not-yet-built `todo`).
        None => {
            let _ = syntax.wait();
            return Err(Ran::Trap("compile timeout (hang)".to_string()));
        }
    };
    let _ = syntax.wait();
    if rcdzc_out.status.success() {
        // Success: the component bytes, plus EVERY compile warning (a clean compile still emits a set of
        // `warning [CODE]` lines to stderr) — threaded so a `(warns …)` clause can grade against them.
        Ok((rcdzc_out.stdout, collect_warnings(&rcdzc_out.stderr)))
    } else {
        // A rejection: recover the diagnostic CODE from the first `error [CODE]` line rcdzc printed to
        // stderr. A TYPED rejection carries a code; a codeless DECLINE (unimplemented construct) none.
        Err({
            let (code, message) = first_error_diag(&rcdzc_out.stderr);
            Ran::Declined { code, message }
        })
    }
}

/// The multi-file PACKAGE component-emit path (`DESIGN-package-linking.md`): write the ENTRY (`program`,
/// as `main.sexp`) and each library `(name, prog)` (as `<name>.sexp`) into a fresh temp dir, then run
/// `cdz compile <lib>.sexp… main.sexp --entry main -o -` — the `cdz` front-end parses each source in
/// process and `compile()` links them. `Err(Ran::Declined)` on a reject/decline (code from stderr).
/// The WIT-WORLD single-file emit path (`DESIGN-compiler-platform-separation.md` §3b): compile a guest
/// against an EXTERNAL declared world so its export crosses under a named interface, rather than the
/// synthesized-world boundary. Convert the guest + world s-expr to binary AST, write them to a temp dir,
/// and run `cdz compile ast:main=<guest> wit-world:wit-world=<world> [--component-name <iface>] -o -` (a
/// stdin pipe carries only one artifact, so both go through files). `Err(Ran::Declined)` on a rejection.
fn emit_component_with_world(
    tools: &Tools,
    program: &str,
    world: &str,
    component_name: Option<&str>,
    opt_level: Option<&str>,
) -> Result<EmittedComponent, Ran> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    // A unique temp dir per invocation (PID + monotonic tick) so concurrent gate workers never collide.
    static TICK: AtomicU64 = AtomicU64::new(0);
    let tick = TICK.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-world-{}-{tick}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return Err(Ran::BadArtifact("could not create a temp world dir".into()));
    }
    // `cdz-syntax convert --from sexpr --to binary -` on one s-expr text → its binary AST bytes.
    let convert = |sexpr: &str| -> Option<Vec<u8>> {
        let mut c = Command::new(&tools.syntax)
            .args(["convert", "--from", "sexpr", "--to", "binary", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
        c.stdin.take().unwrap().write_all(sexpr.as_bytes()).ok();
        let out = c.wait_with_output().expect("wait cdz-syntax convert");
        out.status.success().then_some(out.stdout)
    };
    let fail = |dir: &PathBuf, msg: &str| -> Result<EmittedComponent, Ran> {
        let _ = std::fs::remove_dir_all(dir);
        Err(Ran::BadArtifact(msg.into()))
    };
    let (Some(prog_bin), Some(world_bin)) = (convert(program), convert(world)) else {
        return fail(
            &dir,
            "wit-world case: guest/world s-expr failed to convert to binary AST",
        );
    };
    let prog_path = dir.join("main.bin");
    let world_path = dir.join("world.bin");
    if std::fs::write(&prog_path, &prog_bin).is_err()
        || std::fs::write(&world_path, &world_bin).is_err()
    {
        return fail(&dir, "wit-world case: could not write temp artifact files");
    }

    let mut cmd = Command::new(&tools.rcdzc);
    cmd.arg("compile")
        .arg(format!("ast:main={}", prog_path.display()))
        .arg(format!("wit-world:wit-world={}", world_path.display()));
    if let Some(iface) = component_name {
        cmd.arg("--component-name").arg(iface);
    }
    if let Some(level) = opt_level {
        cmd.arg("--opt-level").arg(level);
    }
    cmd.arg("-o")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().unwrap_or_else(|e| launch_fail("rcdzc", e));
    let out = match wait_with_timeout(child, run_timeout()).expect("wait rcdzc") {
        Some(o) => o,
        None => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Ran::Trap("compile timeout (hang)".to_string()));
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok((out.stdout, collect_warnings(&out.stderr)))
    } else {
        let (code, message) = first_error_diag(&out.stderr);
        Err(Ran::Declined { code, message })
    }
}

fn emit_component_package(
    tools: &Tools,
    program: &str,
    modules: &[(String, String)],
    opt_level: Option<&str>,
) -> Result<EmittedComponent, Ran> {
    use std::process::Command;

    // A unique temp dir per invocation (PID + a monotonic counter) so concurrent gate workers never
    // collide — `Date::now`/random are unavailable, so use the process id + an atomic tick.
    use std::sync::atomic::{AtomicU64, Ordering};
    static TICK: AtomicU64 = AtomicU64::new(0);
    let tick = TICK.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-pkg-{}-{tick}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return Err(Ran::BadArtifact(
            "could not create a temp package dir".into(),
        ));
    }

    // Write every file. The entry is `main.sexp` (its program is named `main`); each library is
    // `<name>.sexp` (matching the `(import "name" …)` target).
    let mut specs: Vec<PathBuf> = Vec::new();
    let write = |path: &PathBuf, text: &str| std::fs::write(path, text);
    let entry_path = dir.join("main.sexp");
    if write(&entry_path, program).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Ran::BadArtifact("could not write the entry file".into()));
    }
    for (name, prog) in modules {
        // Validate the module name is a single safe path component (Copilot PR#517) — the wasm twin of the
        // guard in `emit_rust_package`: a `/`/`\`/`..` name would escape the temp dir.
        if !is_safe_module_name(name) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Ran::BadArtifact(format!(
                "unsafe module name {name:?} (not a single path component) — cannot form a package file"
            )));
        }
        let p = dir.join(format!("{name}.sexp"));
        if write(&p, prog).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Ran::BadArtifact("could not write a library file".into()));
        }
        specs.push(p);
    }
    specs.push(entry_path); // the entry last (order is irrelevant to link, but keep it deterministic)

    // `cdz compile <files> --entry main -o -` → component bytes on stdout (link-map is skipped in
    // single-output mode). stderr carries a decline's diagnostic.
    let mut cmd = Command::new(&tools.rcdzc);
    cmd.arg("compile");
    for s in &specs {
        cmd.arg(s);
    }
    cmd.args(["--entry", "main", "-o", "-"]);
    // Thread the requested opt tier (only the opt-sweep passes one; None = compiler default) — so a
    // package case is compiled at the SAME level being swept, not silently at the default. Without this
    // the sweep could not cover multi-module programs (they'd all compile at O1 regardless of level).
    if let Some(level) = opt_level {
        cmd.args(["--opt-level", level]);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| launch_fail("cdz compile", e));
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok((out.stdout, collect_warnings(&out.stderr)))
    } else {
        Err({
            let (code, message) = first_error_diag(&out.stderr);
            Ran::Declined { code, message }
        })
    }
}

/// Emit Rust source for a SINGLE-file program via the `cdz convert | cdz compile - --target <rust>` pipe.
/// `Ok(source)` or `Err(Ran::Declined { code })` on a shared-front reject/decline (code from stderr).
/// `opt_level` selects the Core pass tier (`--opt-level <L>`); only the opt-sweep passes one — the normal
/// gate passes `None` so its behavior is byte-identical to before.
fn emit_rust_single(
    tools: &Tools,
    program: &str,
    rust_target: &str,
    opt_level: Option<&str>,
) -> Result<String, Ran> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", "sexpr", "--to", "binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    syntax
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .ok();
    let mut compile_args: Vec<&str> = vec!["compile", "-", "-o", "-", "--target", rust_target];
    if let Some(level) = opt_level {
        compile_args.push("--opt-level");
        compile_args.push(level);
    }
    let rcdzc = Command::new(&tools.rcdzc)
        .args(&compile_args)
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = match wait_with_timeout(rcdzc, run_timeout()).expect("wait rcdzc") {
        Some(out) => out,
        // A compile-hang on the RUST backend — killed at the deadline. A FAIL (real compiler bug), not a
        // codeless decline: map to a trap-reason so an `(output …)` case fails rather than grading todo.
        None => {
            let _ = syntax.wait();
            return Err(Ran::Trap("compile timeout (hang)".to_string()));
        }
    };
    let _ = syntax.wait();
    if !rcdzc_out.status.success() {
        return Err({
            let (code, message) = first_error_diag(&rcdzc_out.stderr);
            Ran::Declined { code, message }
        });
    }
    Ok(String::from_utf8_lossy(&rcdzc_out.stdout).to_string())
}

/// Whether `name` is a single SAFE path component usable as a `<name>.sexp` filename — used to reject a
/// module name that would make `dir.join(name)` escape the package temp dir (Copilot PR#517 + #520).
///
/// A PLATFORM-INDEPENDENT character denylist, deliberately NOT `Path::components()`: on Linux (where the
/// fleet runs) `Path::components()` treats `\`, `C:foo`, `\\srv\s` as a single `Normal` component (no
/// backslash-separator or drive-prefix semantics), so it would PASS a Windows-escaping name AND regress the
/// backslash rejection the earlier check had. Instead reject, cross-platform: empty; the `.`/`..` traversal
/// components; any name containing a separator (`/` or `\`); and a `:` (a Windows drive/ADS prefix like
/// `C:foo` — the #520 completeness gap — which `PathBuf::join` treats as prefixed/absolute on Windows).
/// Deliberately strict; a corpus module target is a plain identifier-like string, and a name that trips this
/// fails the trial cleanly.
fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
}

/// Emit Rust source for a PACKAGE (entry + imported libraries) — the rust-target twin of
/// [`emit_component_package`]. Writes the entry as `main.sexp` and each library `(name, prog)` as
/// `<name>.sexp` into a fresh unique temp dir, then runs `cdz compile <lib>.sexp… main.sexp --entry main
/// --target <rust> -o -`; the `cdz` front-end parses + LINKS the sources in process (same linker the wasm
/// package path uses) and the rust backend emits ONE combined module. `Ok(source)` or `Err(Ran::…)` on a
/// reject/decline (code from stderr) or a temp-file failure.
fn emit_rust_package(
    tools: &Tools,
    program: &str,
    modules: &[(String, String)],
    rust_target: &str,
    opt_level: Option<&str>,
) -> Result<String, Ran> {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    static TICK: AtomicU64 = AtomicU64::new(0);
    let tick = TICK.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-rustpkg-{}-{tick}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return Err(Ran::BadArtifact(
            "could not create a temp rust package dir".into(),
        ));
    }
    // The entry is `main.sexp`; each library is `<name>.sexp` (matching its `(import "name" …)` target).
    let mut specs: Vec<PathBuf> = Vec::new();
    let entry_path = dir.join("main.sexp");
    if std::fs::write(&entry_path, program).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Ran::BadArtifact("could not write the entry file".into()));
    }
    for (name, prog) in modules {
        // VALIDATE the module name is a single SAFE path component before using it as a filename (Copilot
        // PR#517): a name with a path separator (`/`, `\`) or a `..` component would make `dir.join(name)`
        // escape the temp dir (writing `<name>.sexp` OUTSIDE it) or imply subdirectories that fail to
        // create. The input is corpus-controlled (a `(module "name" …)` target — not external), so this is
        // a robustness guard, not a security boundary; fail cleanly rather than write astray.
        if !is_safe_module_name(name) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Ran::BadArtifact(format!(
                "unsafe module name {name:?} (not a single path component) — cannot form a package file"
            )));
        }
        let p = dir.join(format!("{name}.sexp"));
        if std::fs::write(&p, prog).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Ran::BadArtifact("could not write a library file".into()));
        }
        specs.push(p);
    }
    specs.push(entry_path); // entry last (link order is irrelevant, but keep it deterministic)
    let mut cmd = Command::new(&tools.rcdzc);
    cmd.arg("compile");
    for s in &specs {
        cmd.arg(s);
    }
    cmd.args(["--entry", "main", "-o", "-", "--target", rust_target]);
    // Thread the requested opt tier (only the opt-sweep passes one; None = compiler default) so a package
    // case is compiled at the SAME level being swept — matching the wasm package path (`emit_component_package`).
    if let Some(level) = opt_level {
        cmd.args(["--opt-level", level]);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| launch_fail("cdz compile", e));
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err({
            let (code, message) = first_error_diag(&out.stderr);
            Ran::Declined { code, message }
        })
    }
}

/// Drive one program through cdz-syntax → rcdzc `--target rust` → `rustc` → run — the Rust-backend
/// gate path. Returns the SAME [`Ran`] outcomes as the wasm path, so `grade_trial` judges the Rust
/// backend against the one corpus oracle:
///  - rcdzc REJECTS/declines → `Ran::Declined { code }` (unchanged from wasm — the front-end is shared);
///  - the emitted `.rs` fails to COMPILE under `rustc` → `Ran::BadArtifact` (the miscompile a broken
///    artifact is — e.g. the narrow-literal width mismatch this gate was built to catch);
///  - the compiled program PANICS at run time → `Ran::Trap` (a Cadenza trap is a Rust panic);
///  - it prints a value → `Ran::Value` (the export's result, in cdz-run's bare-scalar rendering, which
///    Rust's `Display` for an integer/bool reproduces exactly — `42`, `true`).
///
/// A driver `fn main` is generated that calls the sole export (or the `(call …)` export with the trial's
/// args written verbatim as Rust literals) and prints the result, so the value crosses on stdout exactly
/// as cdz-run emits it. Scalar-only today: a compound result declines in the Rust backend (→ `Declined`
/// → Todo), so no `(: value type)` rendering is needed here.
/// Kebab-normalize an effect name to its component-extern form — MUST match the rust backend's
/// `crate::backend::common::export_name::kebab_extern_name` (camelCase→kebab, `_`→`-`, lowercased): `Env`→
/// `env`, `Log`→`log`, `Param`→`param`, `E`→`e`. So the driver can normalize a recorded response key's
/// effect part the same way the backend does, yielding the same shim ident regardless of the corpus key's
/// casing (the corpus records `env.width` normalized but `Param.width` in source case — both normalize equal).
fn kebab_effect(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Derive the crate-root host-call shim fn ident from a recorded response key (`effect.op`) — kebab-normalize
/// the EFFECT part (matching the backend's `canonical_host_op_key`), keep the op verbatim, then map the
/// dotted key's non-ident chars → `_`. MUST equal the backend's emitted `host_shim_ident` for the same op.
fn host_shim_ident_from_key(op_key: &str) -> String {
    let (eff, op) = op_key.split_once('.').unwrap_or(("", op_key));
    let canonical = format!("{}.{}", kebab_effect(eff), op);
    let mut s = String::with_capacity(canonical.len() + 11);
    s.push_str("__cdz_host_");
    for c in canonical.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

/// Generate the crate-root host-call shim fns the emitted `mod prog` references (`crate::__cdz_host_<id>()`).
/// A shim is generated for EVERY distinct `__cdz_host_*` symbol the module names — including UNEXERCISED
/// defs (a `delegated` def beside the called `handled` one) — since every referenced symbol must be DEFINED
/// or rustc E0425s at link. A symbol matched to recorded responses (by the driver-derived ident, which
/// kebab-normalizes the response-key effect to agree with the backend) returns them in order + prints
/// `host-call\t<recorded-op>`; an unmatched symbol gets a `panic!` stub (never reached on a passing trial,
/// loud on a real mismatch). H1: fixed-width-INTEGER responses (`i64`; backend casts to width).
fn build_rust_host_shims(
    module: &str,
    host_responses: &[(String, String)],
    host_calls: &[String],
) -> String {
    // Map recorded op key → (its CANONICAL dotted key for the host-call print, values in order), by shim
    // ident. The printed `host-call\t<op>` is the CANONICAL key (kebab-normalized effect + verbatim op), NOT
    // the raw recorded key — a case's `(host-calls …)` records the canonical form, and the grader compares
    // observed vs expected by exact string, so a source-cased response key (`Param.width`) must be
    // normalized (`param.width`) before printing or the assertion never matches (the wasm oracle observes
    // the canonical name too, via cdz-run's two-sided normalization).
    let mut by_ident: std::collections::BTreeMap<String, (String, Vec<String>)> =
        std::collections::BTreeMap::new();
    for (op, value) in host_responses {
        let ident = host_shim_ident_from_key(op);
        by_ident
            .entry(ident)
            // Compute the canonical key LAZILY — only on first insert per op (repeated calls to the same op
            // reuse the entry), so a multi-call op doesn't re-format/re-allocate the key each iteration.
            .or_insert_with(|| {
                let (eff, opname) = op.split_once('.').unwrap_or(("", op.as_str()));
                (format!("{}.{}", kebab_effect(eff), opname), Vec::new())
            })
            .1
            .push(value.clone());
    }
    // UNIT-RESULT ops (H8): a `(host-calls …)` entry whose op has NO `(host-response …)` is a pure
    // effect op that returns the unit value (it crosses the boundary only to be OBSERVED — e.g. `log.emit`).
    // It records its call NAME (needed for the observed-sequence check) but no response VALUE. Map its shim
    // ident → canonical op so the referenced-shim loop below can emit a `()`-returning shim that prints the
    // op (rather than the panic stub a no-response ident would otherwise get). `host_calls` is already the
    // canonical dotted form (cdz-run/the corpus record it normalized), but membership is keyed by shim
    // IDENT — `by_ident`/`response_ops` derive their idents through the same `host_shim_ident_from_key`
    // mangling, so an op that IS in host_responses but under a source-cased key (`Param.width` vs the
    // canonical `param.width`) still matches and is NOT mis-treated as unit-result.
    let response_idents: std::collections::BTreeSet<String> = host_responses
        .iter()
        .map(|(op, _)| host_shim_ident_from_key(op))
        .collect();
    let mut unit_ops: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for op in host_calls {
        let ident = host_shim_ident_from_key(op);
        if response_idents.contains(&ident) {
            continue; // a VALUE-result op: handled via by_ident above.
        }
        unit_ops.insert(ident, op.clone());
    }
    // Every `crate::__cdz_host_<ident>(<args>)` the module references, with its ARG COUNT (the shim's fn
    // arity must match every call site or rustc E0061s). The backend emits args as simple `__ha0, __ha1, …`
    // idents (H3), so counting the `__ha` tokens in the call's paren group gives the arity reliably (no
    // nested-paren ambiguity). A no-arg call `X()` → 0. Dedup by ident (an op is called at one arity).
    let mut referenced: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut rest = module;
    while let Some(pos) = rest.find("crate::__cdz_host_") {
        let after = &rest[pos + "crate::".len()..];
        let end = after
            .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
            .unwrap_or(after.len());
        let ident = after[..end].to_string();
        // The arg list is the parenthesized group immediately after the ident: `(__ha0, __ha1)` or `()`.
        let arity = after[end..]
            .strip_prefix('(')
            .and_then(|s| s.find(')').map(|c| &s[..c]))
            .map(|argstr| argstr.matches("__ha").count())
            .unwrap_or(0);
        referenced.entry(ident).or_insert(arity);
        rest = &after[end..];
    }
    if referenced.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (fn_name, &arity) in &referenced {
        // The shim's params are GENERIC + ignored — the arg VALUES crossed the boundary but do not select
        // the response (host_responses is keyed per-op, arg-independent) and the corpus host-call sequence
        // compares the op NAME only. `<A0: …>(_a0: A0)` accepts ANY arg type (int/string/bytes) at the call
        // site so a String/Bytes arg (H7) type-checks without the driver knowing arg types.
        let generics = (0..arity)
            .map(|i| format!("A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let generics = if generics.is_empty() {
            String::new()
        } else {
            format!("<{generics}>")
        };
        let params = (0..arity)
            .map(|i| format!("_a{i}: A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        match by_ident.get(fn_name) {
            Some((op, values)) => {
                // RETURN TYPE keyed on the recorded response value text (matches the backend's per-result-
                // kind read): a quoted "…" → `String`; a `[..]`/byte-list-looking value → `Vec<u8>`; a
                // `.`-bearing non-bool → `f64`; else `i64` (bool true/false → 1/0). The `__V` response table
                // is that type; the shim hands out one per call in order.
                let all_quoted = values.iter().all(|v| {
                    let t = v.trim();
                    t.starts_with('"') && t.ends_with('"') && t.len() >= 2
                });
                let is_float = !all_quoted
                    && values
                        .iter()
                        .any(|v| v.trim().contains('.') && v.trim() != "true" && v.trim() != "false");
                let (ret_ty, arr, is_owned) = if all_quoted {
                    // String response: `"hi".to_string()` per value; the shim returns `String` (owned).
                    (
                        "String".to_string(),
                        values
                            .iter()
                            .map(|v| format!("{}.to_string()", v.trim()))
                            .collect::<Vec<_>>()
                            .join(", "),
                        true,
                    )
                } else if is_float {
                    (
                        "f64".to_string(),
                        values.iter().map(|v| v.trim().to_string()).collect::<Vec<_>>().join(", "),
                        false,
                    )
                } else {
                    (
                        "i64".to_string(),
                        values
                            .iter()
                            .map(|v| match v.trim() {
                                "true" => "1".to_string(),
                                "false" => "0".to_string(),
                                other => other.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        false,
                    )
                };
                let n = values.len();
                if is_owned {
                    // An owned (String/Vec) response can't live in a `static` array (non-const); build a
                    // fresh owned value per call, indexed by the call counter via a match.
                    let arms = values
                        .iter()
                        .enumerate()
                        .map(|(k, v)| format!("{k} => {}.to_string(),", v.trim()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         match __k {{ {arms} _ => unreachable!() }} }}\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         static __V: [{ret_ty}; {n}] = [{arr}]; \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         __V[__k] }}\n"
                    ));
                }
            }
            // A referenced shim with NO recorded response. Two sub-cases:
            //   (a) UNIT-RESULT op (H8): its op appears in the case's `(host-calls …)` but records no
            //       response value (a pure effect op — `log.emit` — that returns the unit value). Emit a
            //       `()`-returning shim that prints its canonical op so the observed-sequence check passes.
            //       No response table: it hands out `()` unconditionally, however many times it's called.
            //   (b) Otherwise an UNEXERCISED def (e.g. an unused `delegated` def beside the called one, whose
            //       op is neither responded-to nor called): a panic stub (never reached on a passing trial)
            //       so the artifact links. Generic + returns i64 (unreached, so the type is irrelevant).
            None => match unit_ops.get(fn_name) {
                Some(op) => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) {{ \
                     eprintln!(\"host-call\\t{op}\"); }}\n"
                )),
                None => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> i64 {{ panic!(\"unexercised host op {fn_name}\") }}\n"
                )),
            },
        }
    }
    out
}

#[allow(clippy::too_many_arguments)] // host protocol = responses + calls, threaded alongside the pipeline args
fn run_program_rust(
    tools: &Tools,
    program: &str,
    modules: &[(String, String)],
    call: Option<&Call>,
    async_mode: bool,
    opt_level: Option<&str>,
    // Recorded host responses (`effect.op` key → value text), consumed IN CALL ORDER. For a host-delegating
    // case the emitted `mod prog` calls `crate::__cdz_host_<id>()`; we generate those shim fns here. Empty
    // for a non-host case (no shims → byte-identical driver to before).
    host_responses: &[(String, String)],
    // The case's recorded `(host-calls …)` op sequence (canonical dotted form). Used to generate a
    // `()`-returning shim for a UNIT-RESULT effect op — one that is CALLED but records no response value
    // (H8). Empty for a non-host case (or one with only value-result ops).
    host_calls: &[String],
) -> Ran {
    use std::process::Command;

    // Stage 1+2: program text → Rust source. A SINGLE-file case pipes `cdz convert | cdz compile - --target
    // rust[-async]`; a PACKAGE case (imported libraries present) writes the entry + each library to a temp
    // dir and runs `cdz compile <lib>.sexp… main.sexp --entry main --target rust` — the front-end links the
    // sources exactly as the wasm package path does (`emit_component_package`), then the rust backend emits
    // one combined module. Either way `module` is the emitted Rust; the rest of this fn (rustc + run) is
    // shared. `Err(Ran::…)` short-circuits a reject/decline/write failure with the same outcome as wasm.
    let rust_target = if async_mode { "rust-async" } else { "rust" };
    let module = if modules.is_empty() {
        match emit_rust_single(tools, program, rust_target, opt_level) {
            Ok(m) => m,
            Err(ran) => return ran,
        }
    } else {
        match emit_rust_package(tools, program, modules, rust_target, opt_level) {
            Ok(m) => m,
            Err(ran) => return ran,
        }
    };

    // The export to invoke, and the call expression. The gate passes bare value text (`20`, `-1`,
    // `true`); written verbatim they are valid Rust literals whose type the fn signature fixes. A
    // negative arg is a valid Rust expression too. With no `(call …)`, invoke the sole export nullary.
    let (export, call_expr) = match call {
        Some(c) => {
            // Each arg is a canonical sexp VALUE; a scalar passes through, a compound (`(tuple …)`,
            // `(record …)`) is rebuilt as the Rust expression the backend's parameter type expects.
            // TYPE-AWARE marshal for a BIGINT param: the corpus arg is a BARE decimal (`5`) — its
            // `(: 5 BigInt)` type annotation is stripped by the corpus parser, and unlike a String value
            // (self-identifying `"…"`) a bare `5` gives `rust_call_arg` no way to know it must cross as a
            // `cdz_num::Big`. Emitted verbatim `5` is an i64 literal → rustc E0308 against `fn(a: cdz_num::
            // Big)` (breaker/corpus-bugfix: the BigInt-entry-param artifact-no-build, the BigInt twin of the
            // FIXED String-entry `.to_string()` marshal). So read the emitted fn's param TYPES off its
            // signature and, for a `cdz_num::Big` param, marshal the decimal arg via `big_arg_expr` (the
            // owned-BigInt construction — the BigInt analogue of `.to_string()`). A non-BigInt param, or an
            // arg not a bare decimal, keeps the ordinary `rust_call_arg`.
            let name = rust_ident(&c.export);
            let arg_param_tys: Vec<Option<String>> = parse_emitted_sig(&module, &name, async_mode)
                .map(|sig| {
                    sig.params
                        .iter()
                        .filter(|p| !is_env_param(p))
                        .map(|p| p.split_once(':').map(|(_, ty)| ty.trim().to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let args: Vec<String> = c
                .args
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let is_bigint_param = arg_param_tys
                        .get(i)
                        .and_then(|o| o.as_deref())
                        .is_some_and(|ty| ty == "cdz_num::Big");
                    if is_bigint_param && let Ok(n) = a.trim().parse::<i128>() {
                        cdz_rust_render::big_arg_expr(n)
                    } else {
                        rust_call_arg(a)
                    }
                })
                .collect();
            // HOST-CLOSURE FACTORY export: a def returning a closure (`(def (both (: a)(: b)) (fn (x) …))`)
            // emits `pub fn both(a, b) -> Rc<dyn Fn(x)->r>` — the captured params (a,b) are the factory's
            // OWN params, and the returned `Rc<dyn Fn>` is APPLIED to the remaining call args. The gate's
            // flat `(call both (:10)(:20)(:5))` therefore splits: the first K = factory-param-count args
            // make the handle (`both(10, 20)`), the rest apply it (`(5)`) — the native equivalent of the
            // wasm make/call resource-handle ABI. A NON-factory export (return type is not `Rc<dyn Fn`) is
            // the ordinary single call. Recover K by counting the factory signature's params.
            // CLOSURE-PARAMETER CONSUMER export: a def taking one or more `(-> a b)` params + applying
            // them (`(def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))`). The host supplies each
            // closure — the harness builds it from a companion PRODUCER export (a factory whose result is
            // that same `Rc<dyn Fn>` type) and threads it. The flat `(call apply-it 100 7)` maps its args
            // LEFT-TO-RIGHT onto the consumer's params: a closure param consumes K args (its producer's
            // capture count) to build the closure via the producer; a scalar param consumes one. This
            // mirrors the wasm make/call resource ABI (which cdz-run does itself; the rust driver has no
            // cdz-run, so it synthesizes the closure here). Checked BEFORE the factory branch: a consumer's
            // return type is NOT a closure, so `rust_factory_param_count` returns None for it.
            if let Some(consumer_call) =
                build_closure_consumer_call(&module, &name, &args, async_mode)
            {
                (name, consumer_call)
            } else {
                match rust_factory_param_count(&module, &name, async_mode) {
                    Some(k) if k <= args.len() => {
                        let (caps, applied) = args.split_at(k);
                        let call = format!("{name}({})({})", caps.join(", "), applied.join(", "));
                        (name, call)
                    }
                    _ => (name.clone(), format!("{name}({})", args.join(", "))),
                }
            }
        }
        None => {
            // The sole export's name is the fn to call with no args — recover it from the emitted
            // signature. Sync mode emits `pub fn <name>(`; async mode emits `pub async fn <name><E:
            // CdzEnv>(`, so split on whichever marker is present and stop the name at `(` OR `<` (the
            // async generic-parameter list). (A nullary export; the common no-`call` case.)
            let marker = if async_mode {
                "pub async fn "
            } else {
                "pub fn "
            };
            match module
                .split(marker)
                .nth(1)
                .map(|s| s.split(['(', '<']).next().unwrap_or("").trim())
            {
                Some(name) if !name.is_empty() => (name.to_string(), format!("{name}()")),
                _ => return Ran::BadArtifact("no exported fn in emitted Rust".to_string()),
            }
        }
    };

    // A per-INVOCATION temp dir, GLOBALLY UNIQUE so no two workers ever touch the same `prog.rs`/`prog`.
    // The gate grades trials IN PARALLEL. Keying the dir on a content hash of `program+call` (the prior
    // scheme) was NOT enough: two DISTINCT corpus cases can normalize to the SAME program+call, so two
    // workers land in one dir and race on BOTH files — one worker rewrites `prog.rs` while another's rustc
    // reads it (a truncated source → a spurious build error, its stderr often leading with an unrelated
    // warning), and one execs `prog` while another relinks it (a write-vs-exec race → "text file busy" /
    // "no such file" / permission-denied). All of those surfaced as non-deterministic `BadArtifact` fails
    // whose SET changed run to run. A process-unique dir (a monotonic counter — `Date`/rng are unavailable
    // and would break parallel determinism) gives every invocation its own paths, so there is nothing to
    // race. The small cost is losing compile REUSE across identical programs; compilation is already the
    // dominant cost and identical program+call across cases is rare, so correctness plainly wins.
    static COMPILE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let key = fnv1a(&format!("{program}\u{0}{call_expr}"));
    let seq = COMPILE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("rcdzc-gate-rust-{key:016x}-{pid}-{seq}"));
    let _ = std::fs::create_dir_all(&dir);
    // Each invocation's dir is now unique (no reuse across cases), so it must clean itself up or /tmp
    // grows unboundedly across a run. An RAII guard removes it on EVERY return path (the fn has many
    // early `BadArtifact`/`Declined` exits). Best-effort — a leftover dir on a crash is harmless.
    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            // `RCDZC_GATE_KEEP=1` leaves the per-trial temp dir (emitted `prog.rs` + binary) in place for
            // debugging a build failure — the gate's `BadArtifact` message shows only the first stderr
            // line (often a warning), so keeping the source lets a developer see the full rustc error.
            if std::env::var_os("RCDZC_GATE_KEEP").is_none() {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    let _guard = TmpDir(dir.clone());
    let src = dir.join("prog.rs");
    let bin = dir.join("prog");
    // The driver's entry MUST NOT collide with the export: the corpus overwhelmingly names its export
    // `main`, and the emitted module defines `pub fn main`, so a top-level driver `fn main` beside it is
    // a duplicate `main` (E0428). Wrap the whole emitted module in `mod prog { … }` — its `pub fn main`
    // becomes `prog::main`, and the driver's `fn main` is then the only crate `main`. The module's
    // `#![allow(…)]` inner attributes are valid at the top of the `mod` block, so no rewriting is needed.
    //
    // A UNIT-returning export (`-> ()`) prints the token `unit` (matching cdz-run) — `()` has no
    // `Display`, so the driver evaluates the call for its (absent) value and prints `unit` directly. A
    // scalar/bool export prints via `Display` (`42`, `true`), exactly as cdz-run renders it. The return
    // type is read off the emitted `pub fn …(-> <ty>)` signature, so the one driver stays type-agnostic.
    // The export's CADENZA result type, read off the `// cdz-return[<ident>]: <type>` note the backend
    // emits (its `render_name`) — drives how the result is rendered to cdz-run's text form. The Cadenza
    // type (not the Rust type) is used because it carries what a boundary render needs: field NAMES and
    // the `Tuple`-vs-`Record` distinction the Rust tuple `(T0,T1)` erases.
    let ret_ty = cdz_return_type(&module, &export);
    // HOST-CLOSURE FACTORY result peel: a factory export's `cdz-return` note is the CURRIED arrow of the
    // returned closure — `(-> arg (-> arg2 result))`. The gate applies the factory (`f(caps)(args)`), so the
    // value it renders is the closure's FINAL result, not the arrow. Peel the leading `(-> X …)` wrappers to
    // that final type so `cdz_render_expr` renders it structurally (a Tuple/List result → `(tuple …)`, not a
    // bare `{}` Display that E0277s on a Rust tuple). Only for a factory (its signature returns `Rc<dyn Fn`);
    // an ordinary export keeps its `ret_ty` unchanged.
    let is_factory =
        call.is_some() && rust_factory_param_count(&module, &export, async_mode).is_some();
    // (The H1 factory+host_responses scope-guard was REMOVED once the closure-capture double-emit was fixed:
    // a let-bound host-call value captured by a returned closure used to be RE-EMITTED at the closure build
    // site — a second host call → double-counted responses. The `Core::Closure` capture emit now references
    // the enclosing `let` binding instead of re-emitting, so a factory whose build-time host result a
    // returned closure captures fires the host op exactly once, and these cases pass.)
    // A CLOSURE-PARAMETER CONSUMER export (takes a closure param, supplied by a producer sibling). Its own
    // RESULT crosses the host boundary the SAME way a factory's does — a String/Bytes result is serialized
    // as `list<u8>` and the corpus records the bare byte-int list `(104 105)`, NOT the quoted `"hi"` form.
    // Detected the same way the call-synthesis path does (a consumer builds via `build_closure_consumer_call`);
    // used only to route a String/Bytes consumer result through `cdz_render_bytes_list` (below), mirroring the
    // factory branch. (A consumer's return type is not a closure, so it never overlaps `is_factory`.)
    let is_consumer = call.is_some()
        && !is_factory
        && parse_emitted_sig(&module, &export, async_mode)
            .is_some_and(|sig| sig.params.iter().any(|p| is_closure_param(p)));
    let ret_ty = if is_factory {
        ret_ty.map(|t| peel_arrow_result(&t))
    } else {
        ret_ty
    };
    // The driver's `fn main` calls the export and prints the result the way cdz-run renders it. In ASYNC
    // mode the export is an `async fn` taking `&mut impl CdzEnv` first, so the driver supplies a no-limit
    // gas `Env` + a minimal `block_on` executor and drives `prog::export(&mut env, args)` — the answer
    // must MATCH the sync/wasm oracle (gas metering is invisible to the result), so it grades identically.
    let call_or_await = if async_mode {
        // A CLOSURE-PARAMETER CONSUMER call is already a fully-driven block in async mode — `build_closure_
        // consumer_call` returns `{ let __g0 = block_on(prog::mk(&mut env, …)); block_on(prog::app(&mut env,
        // __g0, …)) }` (it must `let`-bind each closure so the producer + consumer `&mut env` borrows don't
        // overlap, E0499). It is already `prog::`-qualified + `block_on`-wrapped, so pass it VERBATIM — the
        // arg-threading rewrites below would double-wrap it. Discriminated by the leading `{` (no other call
        // shape starts with a block).
        if call_expr.starts_with('{') {
            call_expr.clone()
        } else
        // Rewrite the call to thread the gas/yield `env` as the export's FIRST arg and drive its future to
        // completion. Three call shapes:
        //  - non-factory nullary `export()`         → `block_on(prog::export(&mut env))`
        //  - non-factory with args `export(a, b)`   → `block_on(prog::export(&mut env, a, b))`
        //  - FACTORY `export(caps…)(applied…)`      → `block_on(prog::export(&mut env, caps…))(applied…)`
        //    A factory's `call_expr` is TWO call groups: the factory call (its OWN params = the captures)
        //    and the application of the RETURNED closure. `env` threads into the FACTORY call only; the
        //    returned closure is a plain sync `Rc<dyn Fn>` (an async lifted-closure body declines), so the
        //    `(applied…)` application stays OUTSIDE `block_on` and is a synchronous call. Splitting on the
        //    first top-level `)(` separates the two groups; a non-factory call has no such split.
        if let Some((factory_call, application)) = is_factory
            .then(|| split_factory_application(&call_expr))
            .flatten()
        {
            // `factory_call` = `export(caps…)` (caps may be empty); `application` = `(applied…)`.
            let caps = factory_call
                .strip_prefix(&format!("{export}("))
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            let factory = if caps.is_empty() {
                format!("prog::{export}(&mut env)")
            } else {
                format!("prog::{export}(&mut env, {caps})")
            };
            // OPTION A: the factory returns `Rc<dyn EnvClosure<A,R>>` (NOT a sync `Rc<dyn Fn>`), so its
            // returned closure is applied via `handle.call(&mut env, arg).await`, NOT the sync `(applied)`.
            // `block_on(factory)` yields the handle; bind it to `__h` FIRST so the `.call(&mut env, …)` env
            // borrow doesn't overlap the factory's (E0499). `application` is `(applied…)` — the flat applied
            // args; `EnvClosure::call` takes ONE `A` (a multi-arg closure tuples them, matching the lifted
            // convention + the emit's CallClosure). Strip the parens and tuple ≥2 args into one `A`.
            let applied = application
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("")
                .trim();
            let arg = env_closure_call_arg(applied);
            format!("{{ let __h = block_on({factory}); block_on(__h.call(&mut env, {arg})) }}")
        } else if call_expr.ends_with("()") {
            format!("block_on(prog::{export}(&mut env))")
        } else {
            // `export(a, b)` → `block_on(prog::export(&mut env, a, b))`.
            let arglist = call_expr
                .strip_prefix(&format!("{export}("))
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            format!("block_on(prog::{export}(&mut env, {arglist}))")
        }
    } else {
        format!("prog::{call_expr}")
    };
    // Render the result to cdz-run's text form via a TYPE-DIRECTED expression built from the return type.
    // A scalar prints via `{}`; `()` prints `unit`; a tuple `(T0,T1)` prints `(tuple <r.0> <r.1>)`,
    // recursively — matching the `(tuple …)` a compound value crosses as (the gate accepts either the
    // bare value or the full `(: value type)` form, so the bare `(tuple …)` suffices).
    // The user-sum descriptors (`// cdz-sum[Ident]: (Variant payload) …`) the backend emitted — the
    // variant structure the render needs to `match` a user-enum value into its canonical bare form.
    let sums = cdz_sum_descriptors(&module);
    // …and the erased-newtype descriptors (`// cdz-newtype[Pt]: <inner>`) — a newtype-typed boundary value
    // renders by its inner type (the tag erased), not `Display` of the erased Rust tuple.
    let newtypes = cdz_newtype_descriptors(&module);
    // …and the per-generic-sum parameter COUNT (`// cdz-sum-params[Box]: 1`) — the driver substitutes a
    // generic sum's `T{k}` payload placeholders with the result type's concrete args when it renders one.
    let sum_params = cdz_sum_params(&module);
    // …and the set of sums whose variant heads render QUALIFIED (`// cdz-sum-qualified-heads[Ast]`) — the
    // backend's per-sum `sum_needs_qualified_heads` decision, so the render qualifies a ctor exactly as the
    // wasm backend does (the built-in `Ast` and any user sum with a prelude-colliding variant name).
    let qualified_heads = cdz_sum_qualified_heads(&module);
    // …and the QUANTITY result's unit VALUE-FORM (`// cdz-unit[<ident>]: <value-form>`) — the dotted
    // `((. Unit base) …)` / `Unit./`-quotient surface cdz-run prints a quantity value with, which the type
    // note's `render_name` (the bare `(Unit.base …)` / `Unit.*` TYPE surface) does NOT carry. The backend
    // emits it via `Unit::render_value_form` for every Qty result, so the top-level Qty render splices it
    // verbatim rather than reconstructing it from the type string. Keyed by the export's ident.
    let unit_form = cdz_unit_form(&module, &export);
    // …and the NON-scale-1 quantity's `num/den` scale (`// cdz-scale[<ident>]`) — the harness multiplies the
    // boundary magnitude by it so `5 kilometer` displays `5000 meter`. `None` for a scale-1/non-Qty result.
    let unit_scale = cdz_scale(&module, &export);
    // …and the PER-ELEMENT quantity scale map (`// cdz-qty-at[<ident>]: <path> <num>/<den>`) — for a
    // COMPOUND result carrying non-scale-1 Qty leaves (a tuple/record of quantities at different units), each
    // leaf display-scales independently, keyed by its positional descent path. `cdz_render_expr` consults it
    // in the Qty arm for a nested Qty (the top-level bare Qty keeps using `unit_scale`). Empty for a result
    // with no compound non-scale-1 Qty leaf.
    let qty_at = cdz_qty_at(&module, &export);
    // A DIVERGING export — its Cadenza result type is `Never`, which the `cdz-return` note renders as the
    // fresh var/`Any` a `(trap …)` / never-returning body carries: either `Any` (a grounded hole) or a bare
    // type variable `?N` (an unconstrained result var — e.g. `Option.expect` on a statically-None value,
    // whose result never materializes). The backend emits `-> !` for such a body; the driver must just CALL
    // the export (letting it trap) with NO `let __r`/`println!` — binding a `!` value and printing it is an
    // `unreachable statement` + `()`-isn't-`Display` build error. The gate observes the panic as the
    // recorded `(trap …)` outcome. (A `?N` return means the body diverges; a genuinely-polymorphic non-
    // diverging export is not producible here — the backend would have declined its unrepresentable result.)
    let diverging = ret_ty.as_deref().is_some_and(|t| {
        t == "Any" || t == "!" || (t.starts_with('?') && t[1..].chars().all(|c| c.is_ascii_digit()))
    });
    let body = if diverging {
        format!("fn main() {{ {call_or_await}; }}\n")
    } else {
        match ret_ty.as_deref().map(|ty| {
            // HOST-CLOSURE FACTORY String/Bytes RESULT: a String/Bytes crossing the host boundary AS A
            // CLOSURE RESULT is serialized as `list<u8>` — the corpus renders it as the bare byte-int list
            // `(104 105)` (`()` when empty), NOT the quoted `"hi"` / `b"…"` form a PLAIN String/Bytes export
            // uses. (The wasm `call` method copies the String/Bytes handle into linear memory + returns it as
            // list<u8>; the rust target mirrors the observable form.) So for a FACTORY result of String/Bytes,
            // render the byte list directly; every other type (and a plain export) keeps `cdz_render_expr`.
            if (is_factory || is_consumer) && (ty == "String" || ty == "Bytes") {
                cdz_render_bytes_list(ty)
            } else if is_factory && factory_result_is_value_form_sum(ty, &sums) {
                // HOST-CLOSURE FACTORY SUM RESULT (S4a + user-sum): a sum crossing the host boundary AS A
                // CLOSURE RESULT is value-ENCODED — the corpus records it as the TYPE-ANNOTATED value form
                // `(: (Some 5) (Option Int64))` / `(: (N unit) Dir)` (the shape the wasm `call` method's
                // value-encode produces), NESTED inside the case's own `output (: <that> <type>)`. A PLAIN
                // sum export renders the bare `(Some 5)` (the grader's `expected_value` unwraps one annotation
                // level), but a factory sum result needs the INNER annotation too — so wrap `cdz_render_expr`'s
                // bare value in `(: <value> <type>)`, mirroring the byte-list special-case above.
                let inner = cdz_render_expr(
                    ty,
                    &sums,
                    &newtypes,
                    &sum_params,
                    unit_form.as_deref(),
                    unit_scale,
                    &qty_at,
                    &qualified_heads,
                );
                // The Cadenza type surface for the annotation (`(Option Int64)`) — parenthesize a
                // multi-token type (`Option Int64` → `(Option Int64)`); a bare single token stays as-is.
                let ty_surface = if ty.contains(' ') && !ty.starts_with('(') {
                    format!("({ty})")
                } else {
                    ty.to_string()
                };
                format!("format!(\"(: {{}} {ty_surface})\", {inner})")
            } else {
                cdz_render_expr(
                    ty,
                    &sums,
                    &newtypes,
                    &sum_params,
                    unit_form.as_deref(),
                    unit_scale,
                    &qty_at,
                    &qualified_heads,
                )
            }
        }) {
            Some(render) => {
                format!(
                    "fn main() {{ let __r = {call_or_await}; println!(\"{{}}\", {render}); }}\n"
                )
            }
            // Unknown return type (no emitted signature parsed) — fall back to `{}` (a scalar).
            None => format!("fn main() {{ println!(\"{{}}\", {call_or_await}); }}\n"),
        }
    };
    // HOST-CALL SHIMS (H1): generate the crate-root `__cdz_host_*` fns the emitted `mod prog` references,
    // from the recorded responses (empty for a non-host case → no shims).
    let host_shims = build_rust_host_shims(&module, host_responses, host_calls);
    // In async mode the driver needs an `Env` impl (a no-limit gas meter — the gate checks ANSWERS, not
    // fuel bounds) and a tiny `block_on` executor, plus `let mut env = …` before the call.
    let full = if async_mode {
        format!("mod prog {{\n{module}\n}}\n{host_shims}{ASYNC_GATE_HARNESS}\n{body}")
    } else {
        format!("mod prog {{\n{module}\n}}\n{host_shims}{body}")
    };
    // `body` above referenced `env` in async mode; the harness defines it via a `let` inside `main`, so
    // splice that in. (Kept simple: the harness provides a `gate_env()` and `block_on`, and `main` binds
    // `env` first.)
    let full = if async_mode {
        full.replace("fn main() {", "fn main() { let mut env = GateEnv;")
    } else {
        full
    };
    if std::fs::write(&src, &full).is_err() {
        return Ran::BadArtifact("could not write emitted Rust to a temp file".to_string());
    }
    // Compile with the ambient rustc. A compile failure is a BAD ARTIFACT (the backend emitted source
    // that does not build) — the exact miscompile class this gate catches. In ASYNC mode the emitted
    // module `use`s the shared `cdz_rt::CdzEnv`, so link the pre-built `cdz-rt` rlib: `-L dependency=<dir>
    // --extern cdz_rt=<dir>/libcdz_rt.rlib`.
    // Compile at opt-level 0 (no `-O`). The gate grades the emitted program's OUTPUT (a compile failure or a
    // wrong answer), which is identical at -O0 and -O2 for a correct program — so -O0 is verdict-equivalent
    // (verified: 09-functions shard 298 pass / 2 todo / 0 fail at both). LLVM optimization is rustc's peak-
    // memory phase, and it drove a per-case OOM (exit-143) on the memory-heaviest cases (04-capabilities /
    // 09-functions / 25-verification) on the free ~16GB arm nightly runner — those cases compile fine at -O
    // locally, so it was a runner-RAM ceiling, not a bug. -O0 cuts peak memory to fit the runner (and is
    // faster). The gate tests the BACKEND's emit, not rustc's optimizer, so no coverage is lost.
    let mut cmd = Command::new("rustc");
    cmd.args(["--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&bin);
    if async_mode && let Some(dir) = tools.cdz_rt_dir.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("--extern")
            .arg(format!("cdz_rt={}", dir.join("libcdz_rt.rlib").display()));
    }
    // A program that uses BigInt emits `cdz_num::Big`, so link the `cdz-num` rlib. Provided for BOTH sync
    // and async (BigInt appears in either); harmless when the program doesn't reference it (`--extern`
    // only makes the crate available, it isn't force-linked). `-L dependency` lets rustc find its deps.
    if let Some(dir) = tools.cdz_num_dir.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("--extern")
            .arg(format!("cdz_num={}", dir.join("libcdz_num.rlib").display()));
    }
    // A program that uses the native R2 value codec emits `cadenza_ast::codec::encode`/`decode`, so link the
    // `cadenza-ast` rlib. Provided for BOTH sync and async; harmless when unreferenced (`--extern` only makes
    // it available). Its transitive deps (`num_bigint`, `unicode_normalization`) land in `<dir>/deps`, so add
    // that search path too (the top-level rlib is in `<dir>` itself).
    if let Some(dir) = tools.cadenza_ast_dir.as_deref() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg("-L")
            .arg(format!("dependency={}", dir.join("deps").display()))
            .arg("--extern")
            .arg(format!(
                "cadenza_ast={}",
                dir.join("libcadenza_ast.rlib").display()
            ));
        // The native value-encode emit constructs `num_bigint::BigInt` for Int leaves, so `num_bigint` must
        // be a NAMABLE extern (not just findable for cadenza_ast's transitive link). Its rlib is hash-named
        // in `<dir>/deps` (`libnum_bigint-<hash>.rlib`); glob for it and `--extern num_bigint=` it. Harmless
        // when unreferenced. (num_bigint is a shared dep — cdz-num uses it too — so it is always built.)
        if let Ok(entries) = std::fs::read_dir(dir.join("deps"))
            && let Some(rlib) = entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libnum_bigint-") && n.ends_with(".rlib"))
            })
        {
            cmd.arg("--extern")
                .arg(format!("num_bigint={}", rlib.display()));
        }
    }
    let compiled = cmd.output();
    let compiled = match compiled {
        Ok(o) => o,
        Err(e) => return Ran::BadArtifact(format!("rustc failed to launch: {e}")),
    };
    if !compiled.status.success() {
        return Ran::BadArtifact(first_line(&compiled.stderr));
    }
    // Run it. A panic (Cadenza trap) exits non-zero → `Ran::Trap`; a clean run prints the value. Retry a
    // few times on a LAUNCH error: a freshly-compiled binary can transiently report "text file busy" /
    // permission-denied on Linux while the writer handle (rustc/its linker) is still closing — a race,
    // not a defect in the artifact. Under the gate's heavy parallelism this window can outlast a single
    // retry, so back off briefly and retry a handful of times before giving up. (A GENUINELY unrunnable
    // binary fails every attempt, so this never hides a real problem.)
    let mut last_err = None;
    let mut got = None;
    for attempt in 0..8 {
        // Spawn with piped stdout/stderr so the run is bounded by a wall-clock timeout — an
        // infinite-loop program (the rust-backend analogue of a runaway wasm case) would otherwise hang
        // the gate forever, the exact host-overload the timeouts bound.
        match Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => match wait_with_timeout(child, run_timeout()) {
                Ok(Some(o)) => {
                    got = Some(o);
                    break;
                }
                // Runaway emitted binary — killed at the deadline. FAIL(hang), same as the wasm run path.
                Ok(None) => return Ran::Trap("timeout (hang)".to_string()),
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(2 * (attempt + 1)));
                }
            },
            Err(e) => {
                last_err = Some(e);
                // A short escalating backoff to let the writer handle close.
                std::thread::sleep(std::time::Duration::from_millis(2 * (attempt + 1)));
            }
        }
    }
    let run = match got {
        Some(o) => o,
        None => {
            return Ran::BadArtifact(format!(
                "compiled prog failed to launch: {}",
                last_err.expect("a launch error after all retries failed")
            ));
        }
    };
    if run.status.success() {
        // A host-delegating case's shim fns print `host-call\t<op>` to stderr in call order (H1); parse them
        // for the `(host-calls …)` check, exactly as the wasm path does. Empty for a non-host program.
        let observed = observed_host_calls(&run.stderr);
        // The rust-backend path does not capture rcdzc compile warnings today (its emit path differs);
        // `(warns …)` cases are graded on the wasm path. Empty warnings here (a later increment can wire
        // the rust emit's compile stderr if warning-parity across backends is wanted).
        Ran::Value(
            String::from_utf8_lossy(&run.stdout).trim().to_string(),
            observed,
            Vec::new(),
        )
    } else {
        Ran::Trap(rust_panic_message(&run.stderr))
    }
}

/// The trap REASON from a Rust process's panic stderr. Rust formats a panic as
/// `thread '<name>' panicked at <file>:<line>:<col>:` followed by the panic MESSAGE on the NEXT line
/// (`panic!("unreachable")` → the message line is `unreachable`). The gate's `classify` classifies by
/// that reason, so the message line — not the `panicked at <loc>` header (which has no reason) — is what
/// must be returned. Take the line AFTER the `panicked at` header; fall back to the first line if the
/// format is unexpected (a non-panic non-zero exit).
fn rust_panic_message(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut lines = s.lines();
    while let Some(line) = lines.next() {
        if line.contains("panicked at") {
            // The message is the next non-empty line (Rust prints it immediately after the header).
            if let Some(msg) = lines.next() {
                return msg.trim().to_string();
            }
        }
    }
    first_line(bytes)
}

/// If `name`'s emitted signature is a CLOSURE FACTORY — `pub fn <name>(<params>) -> …Rc<dyn Fn(…)…` — return
/// the factory's PARAMETER COUNT (the split point between make-captures and the applied closure args);
/// `None` for an ordinary (non-factory) export. A closure-factory def emits its captured params as the
/// factory's own params and returns the closure as an `Rc<dyn Fn>` VALUE, so the host calls `f(caps…)` then
/// applies `(args…)`. Counting params is a simple top-level comma count inside the signature's `(...)` (the
/// only nesting a scalar/compound param type introduces is `<…>`/`(…)` in the type, which we balance). Sync
/// mode marker `pub fn `; async `pub async fn ` (its `<E: CdzEnv>` generic list precedes the `(`).
/// Whether a peeled factory-result type is a SUM whose value crosses the host boundary as the TYPE-ANNOTATED
/// value form (`(: (Some 5) (Option Int64))` / `(: (N unit) Dir)`) — the S4a render special-case. A factory
/// sum result is value-encoded, so its rendered value needs the inner `(: value type)` wrapper (a plain sum
/// export renders bare and the grader unwraps one level). Matches (a) a built-in Option/Result head, bare
/// (`Option Int64`) or parenthesized (`(Option Int64)`); or (b) a USER sum — a bare type NAME (or applied
/// head `(Box …)`) that has an emitted `// cdz-sum[…]` descriptor (a key in `sums`). A non-sum type
/// (scalar/Tuple/List) renders bare (no wrapper) as before.
fn factory_result_is_value_form_sum(
    ty: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
) -> bool {
    let head = ty.trim().trim_start_matches('(');
    if head.starts_with("Option ")
        || head.starts_with("Result ")
        || head == "Option"
        || head == "Result"
    {
        return true;
    }
    // A USER sum: the head token (a bare `Dir`, or the applied head of `(Box Int64)`) is a descriptor key.
    let head_token = head.split_whitespace().next().unwrap_or(head);
    sums.contains_key(head_token)
}

/// Peel a CURRIED arrow type down to its final (non-arrow) RESULT — `(-> Int64 (-> Int64 (Tuple Int64
/// Int64)))` → `(Tuple Int64 Int64)`. A host-closure factory's `cdz-return` note is the returned closure's
/// arrow; the gate applies the factory to full arity so the rendered value is this final result. A
/// non-arrow type is returned unchanged. Balanced-paren aware: the arrow is `(-> <arg> <rest>)` where
/// `<arg>` may itself be a parenthesized compound, so skip the first top-level sub-term (`<arg>`) and take
/// `<rest>`, recursing while `<rest>` is itself a `(-> …)`.
fn peel_arrow_result(ty: &str) -> String {
    let mut cur = ty.trim();
    loop {
        let inner = match cur.strip_prefix("(-> ") {
            Some(i) => i.trim_end().strip_suffix(')').map(str::trim),
            None => None,
        };
        let Some(inner) = inner else {
            return cur.to_string();
        };
        // `inner` = `<arg> <rest>`. Skip the first top-level term (`<arg>`), balancing parens.
        let bytes = inner.as_bytes();
        let mut depth = 0usize;
        let mut split = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b' ' if depth == 0 => {
                    split = Some(i);
                    break;
                }
                _ => {}
            }
        }
        match split {
            Some(i) => cur = inner[i + 1..].trim(),
            // No top-level space → a single-token arrow body with no result (malformed); return as-is.
            None => return cur.to_string(),
        }
    }
}

/// The render EXPRESSION for a host-closure factory's String/Bytes RESULT — the `list<u8>` byte form the
/// corpus expects (`(104 105)` for "hi", `()` empty), NOT the quoted `"hi"`/`b"…"` a plain export uses. The
/// String path iterates its UTF-8 bytes (`.bytes()`); the Bytes path iterates the `Vec<u8>` (`.iter()`).
/// Emits `(b0 b1 …)` — space-separated byte ints in one paren group, no trailing space (a leading space per
/// byte, so the empty value yields `()`).
fn cdz_render_bytes_list(ty: &str) -> String {
    // `__r` is the closure's applied result: a `String` (String result) or `Vec<u8>` (Bytes result).
    let iter = if ty == "String" {
        "(__r).bytes()"
    } else {
        "(__r).iter().copied()"
    };
    format!(
        "{{ let mut __s = String::from(\"(\"); let mut __first = true; for __b in {iter} {{ \
         if !__first {{ __s.push(' '); }} __first = false; __s.push_str(&__b.to_string()); }} \
         __s.push(')'); __s }}"
    )
}

/// The single `EnvClosure::call` `A` argument from a flat applied-args string (`"5"`, `"3, 4"`, `""`). A
/// 0-arg closure takes `()`; a 1-arg closure the bare arg; a ≥2-arg closure a TUPLE `(a, b)` — matching the
/// lifted-lambda calling convention the backend's `Core::CallClosure`/`EnvClosure` impl uses (a multi-arg
/// closure tuples its flat args into one `A`, destructured inside `call`). Splits on TOP-LEVEL commas only
/// (a compound arg `(tuple 1 2)` / `Rc::new(..)` keeps its inner commas), so `(a, (x, y))` stays two args.
///
/// Nesting is balanced over `()`, `[]`, `{}` (block/struct-literal args), AND `<>` — EXCEPT a `>` that is
/// the tail of a `->` return arrow (a closure-typed arg `Rc<dyn Fn(i64) -> i64>` contains a `->` whose `>`
/// must NOT close a `<` group, or the `<>` depth would decrement one step too EARLY — dropping back to 0
/// before the type's real closing `>`, so a comma AFTER the arrow leaks as top-level and the arg mis-tuples;
/// the depth uses `saturating_sub`, so it never underflows, it just closes the group prematurely). Today the
/// gate's `applied` args are corpus call VALUES (scalars, `(tuple …)`, bignum exprs) — no block/struct-
/// literal/closure-sig arg reaches here — so the `{}`/`->` handling is DEFENSIVE (github-liaison #2391 c1):
/// it keeps the splitter correct if the emit surface ever grows such an arg, rather than silently mis-tupling.
fn env_closure_call_arg(applied: &str) -> String {
    let applied = applied.trim();
    if applied.is_empty() {
        return "()".to_string();
    }
    let bytes = applied.as_bytes();
    let mut depth = 0usize;
    let mut n_top_commas = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            // A `>` preceded by `-` is the arrow of a `->` return type, NOT a `<` group close — skip it
            // (matching the emitted `Rc<dyn Fn(A) -> R>` / `EnvClosure` closure-typed arg spelling).
            b'>' if i > 0 && bytes[i - 1] == b'-' => {}
            b')' | b'>' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => n_top_commas += 1,
            _ => {}
        }
    }
    if n_top_commas == 0 {
        applied.to_string() // 1 arg → the bare arg
    } else {
        format!("({applied})") // ≥2 args → a tuple of them
    }
}

/// Split a FACTORY call expression `export(caps…)(applied…)` into `("export(caps…)", "(applied…)")` at the
/// boundary between the factory's own arg group and the returned-closure application. Returns `None` when
/// there is no top-level application group (a non-factory call `export(args…)`, or a factory whose result
/// closure is not applied). The split is the FIRST `)` at paren-depth 0 that is immediately followed by a
/// `(` — a nested `)(` inside a compound argument (`both((tuple 1 2), (record …))`) sits at depth > 0 and is
/// skipped, so only the real factory/application seam matches.
fn split_factory_application(call_expr: &str) -> Option<(String, String)> {
    let bytes = call_expr.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                // A depth-0 close directly followed by `(` is the factory→application seam.
                if depth == 0 && call_expr[i + 1..].starts_with('(') {
                    return Some((call_expr[..=i].to_string(), call_expr[i + 1..].to_string()));
                }
            }
            _ => {}
        }
    }
    None
}

/// The parsed shape of an emitted `pub fn <name>(…) -> <ret>` signature, used by BOTH the factory
/// (producer) analysis and the closure-parameter (consumer) analysis so they share ONE arrow-aware
/// param-list walk (v-fleet-tooling review ask #1: don't naive-substring the closure-type text).
struct EmittedSig<'a> {
    /// Each top-level parameter's verbatim `<name>: <type>` text, in source order. Empty for a nullary fn.
    /// The env param (`__cdz_env: &mut __CdzE`) is INCLUDED here (callers that care filter it).
    params: Vec<&'a str>,
    /// The return-type text (up to the fn body `{`).
    ret_head: String,
}

/// Parse the emitted signature of `name` (the SOURCE-level export ident) out of `module`. Returns the
/// param-list, the per-parameter slices (arrow-aware split, so a closure-typed param `g: Rc<dyn Fn(i64)
/// -> i64>` is ONE param, not split at its inner `->`/`,`), and the return-type head. `None` if no such
/// exported fn header is found or its param list is malformed.
fn parse_emitted_sig<'a>(module: &'a str, name: &str, async_mode: bool) -> Option<EmittedSig<'a>> {
    let marker = if async_mode {
        "pub async fn "
    } else {
        "pub fn "
    };
    // Find the exact `<marker><name>` header. The name boundary matters: a bare `split` on `pub fn both`
    // also matches `pub fn both2(` (prefix), so a MULTI-export module grabs the wrong occurrence (Copilot
    // PR#548). Only an occurrence whose next char starts the param list `(` (sync) or the generic list
    // `<` (async) — never an identifier-continuation char — is the real header.
    let needle = format!("{marker}{name}");
    let after = module
        .match_indices(&needle)
        .map(|(i, _)| &module[i + needle.len()..])
        .find(|rest| matches!(rest.chars().next(), Some('(') | Some('<')))?;
    // Skip an async generic-parameter list `<…>` if present, to reach the param-list `(`.
    let after = after.trim_start();
    let after = if after.starts_with('<') {
        &after[after.find('>').map(|i| i + 1)?..]
    } else {
        after
    };
    let after = after.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    // Walk the param list, tracking nesting depth so a `(…)`/`<…>` inside a param TYPE isn't miscounted,
    // and recording each top-level comma so params can be split. A `>` closes an angle group EXCEPT the
    // `>` of a `->` return arrow (which appears INSIDE the list when a param type is itself a closure,
    // `g: Rc<dyn Fn(i64) -> i64>`); counting it as a close underflows depth so the list's own `)` never
    // returns to depth 0 → the slice below would panic `begin > end` (v-rust-backend hit this). Guard: a
    // `>` immediately preceded by `-` is an arrow, not a bracket close.
    let bytes = after.as_bytes();
    let mut depth = 0usize;
    let mut end = 0usize;
    let mut comma_positions = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            b'>' if i == 0 || bytes[i - 1] != b'-' => depth = depth.saturating_sub(1),
            b',' if depth == 1 => comma_positions.push(i),
            _ => {}
        }
    }
    // If the walk never found the param-list close (`end` still 0 — a malformed/unexpected shape), this is
    // not a signature we can analyze: return None rather than slicing `&after[1..0]` (panics `begin > end`).
    if end == 0 {
        return None;
    }
    // Split the param list into top-level params at the recorded commas (indices into `after`; the list
    // runs `1..end` past the leading `(`). An empty list (nullary fn) → no params.
    let params: Vec<&str> = if after[1..end].trim().is_empty() {
        Vec::new()
    } else {
        let mut parts = Vec::new();
        let mut start = 1usize; // just past the `(`
        for &c in &comma_positions {
            parts.push(after[start..c].trim());
            start = c + 1;
        }
        parts.push(after[start..end].trim());
        parts
    };
    let ret_head: String = after[end + 1..].chars().take_while(|&c| c != '{').collect();
    Some(EmittedSig { params, ret_head })
}

/// Whether a parameter slice (`<name>: <type>`) is the async gas/yield env param — backend plumbing, not
/// a source param. Its emitted names mirror the rcdzc rust backend's `ENV_PARAM`/`ENV_TYPE_PARAM`
/// (`backend/rust/mod.rs`): value `__cdz_env`, type `__CdzE`.
fn is_env_param(param: &str) -> bool {
    param.trim_start().starts_with("__cdz_env") || param.contains("&mut __CdzE")
}

/// Whether a type string names a runtime closure VALUE — either the SYNC `Rc<dyn Fn(…)>` or the ASYNC
/// (Option A) `Rc<dyn cdz_rt::EnvClosure<A, R>>` (a lifted async closure crosses as an `EnvClosure` trait
/// object, not a `dyn Fn`, since its `call` future borrows the `&mut env` — see `cdz_rt::EnvClosure`). Both
/// closure-detection sites (a PARAM type, a factory RESULT type) key off this so the async host-closure
/// cases are recognized as factories/consumers/producers exactly like the sync ones.
fn names_closure_value(ty: &str) -> bool {
    ty.contains("Rc<dyn Fn(")
        || ty.contains("Rc<dyn cdz_rt::EnvClosure<")
        || ty.contains("Rc<dyn EnvClosure<")
}

/// Whether a parameter slice (`<name>: <type>`) is a closure — sync `Rc<dyn Fn(…)>` or async `Rc<dyn
/// EnvClosure<…>>`.
fn is_closure_param(param: &str) -> bool {
    names_closure_value(param)
}

/// The closure TYPE of a parameter slice (`g: std::rc::Rc<dyn Fn(i64) -> i64>`) → the `Rc<dyn Fn…>` text,
/// or `None` if the param is not a closure. Used to match a consumer's closure param to the PRODUCER whose
/// result type is that same closure type. Extracts the BALANCED `Rc<…>` (stops at the angle bracket that
/// matches the opening `<` of `Rc<`), so a param that is NOT last in the list (`g: Rc<dyn Fn(i64)->i64>,
/// x: i64`) yields ONLY the closure type — NOT the trailing `, x: i64`. This matters for a HIGHER-ORDER
/// producer: a substring-tolerant match would false-pair a first-order consumer param `Rc<dyn Fn(i64)->i64>`
/// to a higher-order producer `Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->i64>` (the former is a substring of the
/// latter), so the pairing must compare EXACT balanced closure types (`ty_matches` uses `==` — see there).
fn closure_param_type(param: &str) -> Option<&str> {
    // Locate the `Rc<…>` closure type — sync `Rc<dyn Fn(` or async `Rc<dyn [cdz_rt::]EnvClosure<`. The
    // balanced `<`/`>` walk below then extracts the whole `Rc<…>` (the `EnvClosure<A, R>`'s inner `<>` and
    // the `Fn(A) -> R`'s `->` are both handled by the depth counter + the `->` guard), so an async closure
    // param `g: std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>` yields exactly its `Rc<…>` type.
    let start = param
        .find("std::rc::Rc<dyn Fn(")
        .or_else(|| param.find("std::rc::Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| param.find("Rc<dyn Fn("))
        .or_else(|| param.find("Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| param.find("Rc<dyn EnvClosure<"))?;
    let rest = &param[start..];
    // Find the `<` that opens `Rc<` and walk to its MATCHING `>` (depth-balanced over `<`/`>`), so a nested
    // `Rc<dyn Fn(Rc<…>)…>` returns its whole self and a trailing `, x: i64` is excluded. CRITICAL: the
    // return arrow `->` contains a `>` that must NOT be counted as a closing angle bracket — skip a `>`
    // immediately preceded by `-` (matching Rust's `->` in the emitted `Rc<dyn Fn(A) -> R>`).
    let open = rest.find('<')?;
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    for (i, c) in rest.char_indices().skip(open) {
        match c {
            '<' => depth += 1,
            '>' if i > 0 && bytes[i - 1] == b'-' => {} // the `>` of a `->` return arrow — not a bracket
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=i].trim());
                }
            }
            _ => {}
        }
    }
    // Unbalanced (shouldn't happen for emitted Rust) — fall back to the whole tail, trimmed.
    Some(rest.trim())
}

/// The TYPE of a parameter slice `<name>: <type>` → the `<type>` text (everything after the first `:`).
/// A param is always `name: type` in emitted Rust (no un-annotated params), so the split is unambiguous.
fn param_type_of(param: &str) -> String {
    match param.split_once(':') {
        Some((_, ty)) => ty.trim().to_string(),
        None => param.trim().to_string(),
    }
}

/// The closure type out of a return-head — sync `-> std::rc::Rc<dyn Fn(i64) -> i64>` or async `-> std::rc::
/// Rc<dyn cdz_rt::EnvClosure<i64, i64>>` — trimming the leading `->`. `None` if the return is not a closure.
fn closure_ret_type(ret_head: &str) -> Option<String> {
    let start = ret_head
        .find("std::rc::Rc<dyn Fn(")
        .or_else(|| ret_head.find("std::rc::Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| ret_head.find("Rc<dyn Fn("))
        .or_else(|| ret_head.find("Rc<dyn cdz_rt::EnvClosure<"))
        .or_else(|| ret_head.find("Rc<dyn EnvClosure<"))?;
    Some(ret_head[start..].trim().to_string())
}

/// Build the driver call for a CLOSURE-PARAMETER consumer export, or `None` if `name` has no closure param
/// (then the caller falls through to the factory/ordinary path). The consumer's params are mapped
/// LEFT-TO-RIGHT onto the flat call `args`: a closure param consumes K args (its producer's capture count)
/// and becomes `prog::<producer>(<those K args>)`; a non-closure param consumes one arg verbatim. Each
/// closure param is paired to its producer DETERMINISTICALLY: the FIRST not-yet-used export whose result
/// type equals the param's closure type, scanning producers in module order and consuming each once
/// (v-fleet-tooling review ask #2 — a stable pairing so a future change cannot silently reorder them).
/// `prog::` prefixes match the driver's `mod prog { … }` wrapping. Async producers/consumers are driven by
/// the caller's `block_on` (the closure value itself is a sync `Rc<dyn Fn>` per the option-C rule).
fn build_closure_consumer_call(
    module: &str,
    name: &str,
    args: &[String],
    async_mode: bool,
) -> Option<String> {
    let sig = parse_emitted_sig(module, name, async_mode)?;
    // Only a CONSUMER (has a closure param); if none, let the factory/ordinary path handle it.
    let source_params: Vec<&&str> = sig.params.iter().filter(|p| !is_env_param(p)).collect();
    if !source_params.iter().any(|p| is_closure_param(p)) {
        return None;
    }
    // Enumerate the PRODUCER exports, in module order. A producer that supplies a closure comes in TWO
    // emitted shapes, both handled:
    //  - FACTORY: a def WITH captures returns the closure as a VALUE — `fn make_adder(k) -> Rc<dyn Fn…>`
    //    (result type IS a closure). Its cap-count args build the closure: `make_adder(<caps>)`.
    //  - PEELED: a NULLARY def whose closure body is eta-peeled to a direct fn — `fn mk(x) -> i64` (a
    //    `(fn (x) …)` with no capture; the closure is applied at one site so the emitter inlines it). Its
    //    closure type is `Rc<dyn Fn(<its params>) -> <its ret>>`; no cap args — the closure IS the fn,
    //    wrapped `Rc::new(prog::mk as fn(<p>)-><r>) as Rc<dyn Fn…>`.
    // Each producer is consumed at most once; a closure param pairs to the FIRST unused producer whose
    // closure type matches (deterministic left-to-right — review ask #2).
    enum Producer {
        /// A real factory: its RESULT closure type + capture-arg count. `shape` is the factory's `cdz-return`
        /// arrow render-name (the pre-erasure Cadenza closure type, e.g. `(-> (Tuple Int64 Int64) Int64)` vs
        /// `(-> (Record (a Int64) (b Int64)) Int64)`) — used to disambiguate a Tuple-arg vs Record-arg
        /// producer whose ERASED `Rc<dyn Fn>` type collides, matched against the consumer's `cdz-param-shapes`.
        Factory {
            ident: String,
            closure_ty: String,
            cap: usize,
            shape: Option<String>,
        },
        /// An eta-peeled nullary producer: `fn ident(<params>) -> <ret>` == the closure `Rc<dyn Fn(<params
        /// types>) -> <ret>>`. We store the equivalent closure type + the raw `fn` type for the coercion.
        /// `shape` is its `cdz-produces-closure` arrow (the pre-erasure Cadenza shape) — used, like the
        /// Factory's, to disambiguate a Tuple-arg vs Record-arg peeled producer of colliding erasure.
        Peeled {
            ident: String,
            closure_ty: String,
            fn_ty: String,
            shape: Option<String>,
        },
    }
    let mut producers: Vec<Producer> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, _) in module
        .match_indices("pub fn ")
        .chain(module.match_indices("pub async fn "))
    {
        let rest = &module[i..];
        let after_kw = match rest
            .strip_prefix("pub async fn ")
            .or_else(|| rest.strip_prefix("pub fn "))
        {
            Some(s) => s,
            None => continue,
        };
        let ident: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.is_empty() || ident == name || !seen.insert(ident.clone()) {
            continue;
        }
        let Some(psig) = parse_emitted_sig(module, &ident, async_mode) else {
            continue;
        };
        let src_params: Vec<&&str> = psig.params.iter().filter(|p| !is_env_param(p)).collect();
        if names_closure_value(&psig.ret_head) {
            // FACTORY: result is a closure (sync `Rc<dyn Fn(` or async `Rc<dyn EnvClosure<`). A NULLARY
            // factory (cap = 0) returning a closure value is the common host-closure producer `(def (mk) (fn
            // (n) …))`; classifying it here (not as a Peeled fn-item, which an async closure can't be) is what
            // lets the async consumer path drive it via `block_on(prog::mk(&mut env))` → the `Rc<dyn
            // EnvClosure>` handle. Its `cdz-return[ident]` note is the arrow render-name (pre-erasure closure
            // shape) — captured for the collision-disambiguation pairing below.
            let cty = closure_ret_type(&psig.ret_head)?;
            let shape = cdz_return_type(module, &ident);
            producers.push(Producer::Factory {
                ident,
                closure_ty: cty,
                cap: src_params.len(),
                shape,
            });
        } else {
            // PEELED candidate: a fn whose EQUIVALENT closure value is `Rc::new(fn-item)`. Its closure type
            // is `Rc<dyn Fn(<param types>) -> <ret>>` and the raw `fn(<param types>) -> <ret>` is the
            // coercion target. `param_type_of` yields each param's full Rust type — so a HIGHER-ORDER
            // producer (S4: a fn that itself takes a closure param, `fn mk(f: Rc<dyn Fn(i64)->i64>) -> i64`)
            // is a valid producer whose closure type is `Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->i64>`. It is
            // supplied to the consumer as `Rc::new(prog::mk as fn(Rc<dyn Fn…>)->i64)`. (Previously a fn WITH
            // a closure param was skipped as "a consumer, not a producer" — but the higher-order round-trip
            // has mk be BOTH: a producer for app's `g` AND a consumer of an in-guest `f`. Pairing is by
            // erased closure-type via `ty_matches` below, so a genuine consumer that matches no sibling
            // closure param is simply never paired — accepting it as a producer candidate is harmless.)
            let param_types: Vec<String> = src_params.iter().map(|p| param_type_of(p)).collect();
            let ret = psig
                .ret_head
                .trim()
                .trim_start_matches("->")
                .trim()
                .to_string();
            let closure_ty = format!("std::rc::Rc<dyn Fn({}) -> {}>", param_types.join(", "), ret);
            let fn_ty = format!("fn({}) -> {}", param_types.join(", "), ret);
            let shape = cdz_produces_closure(module, &ident);
            producers.push(Producer::Peeled {
                ident,
                closure_ty,
                fn_ty,
                shape,
            });
        }
    }
    let mut used_producer = vec![false; producers.len()];
    let mut arg_i = 0usize;
    let mut call_args: Vec<String> = Vec::with_capacity(source_params.len());
    // In ASYNC mode a closure argument built from a FACTORY producer must be driven through `block_on` (the
    // producer is an `async fn`), and threading `&mut env` into BOTH the producer call AND the consumer call
    // in one expression would be two simultaneous `&mut env` borrows (E0499). So bind each async-built
    // closure to a `let __gN` FIRST (sequential borrows), then call the consumer with the bound names. The
    // collected `let` statements are returned as a prelude the caller splices before the consumer call.
    let mut async_lets: Vec<String> = Vec::new();
    // The CONSUMER's per-closure-param Cadenza arrow shapes (`// cdz-param-shapes[name]: <arrow> | <arrow>`),
    // in closure-param order — the pre-erasure types the driver matches against each producer's `shape` to
    // disambiguate a Tuple-arg vs Record-arg closure whose ERASED `Rc<dyn Fn>` collides. Empty when the note
    // is absent (a scalar/tuple-only consumer, no ambiguity), in which case pairing falls back to erased-type.
    let consumer_shapes = cdz_param_shapes(module, name);
    let mut closure_param_idx = 0usize;
    // A producer matches a closure param when their ERASED closure types are compatible AND — when BOTH the
    // producer's `shape` and the consumer's param-shape are known — their pre-erasure shapes agree. The
    // shape guard only ever NARROWS a match (it never admits an erased-type mismatch), so a consumer/producer
    // without shape notes behaves exactly as before (erased-type-only pairing).
    let ty_matches = |prod: &Producer, cty: &str, want_shape: Option<&str>| {
        let (closure_ty, prod_shape) = match prod {
            Producer::Factory {
                closure_ty, shape, ..
            }
            | Producer::Peeled {
                closure_ty, shape, ..
            } => (closure_ty, shape.as_deref()),
        };
        // EXACT structural equality (not substring containment): both `closure_ty` (the producer's, built by
        // `format!("std::rc::Rc<dyn Fn({}) -> {}>", …)`) and `cty` (the consumer param's, extracted BALANCED
        // by `closure_param_type`) are spelled by the SAME `rust_type` formatter, so an identical closure
        // type compares equal. Substring containment (the prior check) FALSE-MATCHED a first-order consumer
        // param to a HIGHER-ORDER producer whose erased type CONTAINS it (`Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->
        // i64>` contains `Rc<dyn Fn(i64)->i64>`) → nondeterministic mis-pairing → ill-typed harness codegen
        // (github-liaison #1654 review). Exact `==` removes that class entirely.
        let erased_ok = closure_ty.as_str() == cty;
        if !erased_ok {
            return false;
        }
        match (want_shape, prod_shape) {
            (Some(w), Some(p)) => w == p,
            _ => true, // shape unknown on either side → erased-type match suffices (prior behavior)
        }
    };
    for p in &source_params {
        if let Some(cty) = closure_param_type(p) {
            let want_shape = consumer_shapes.get(closure_param_idx).map(|s| s.as_str());
            closure_param_idx += 1;
            // Pair this closure param to a producer: PREFER the first not-yet-used matching producer
            // (deterministic left-to-right — review ask #2), but FALL BACK to REUSING a matching producer
            // when every match is already used. Reuse is correct: the host mints a FRESH closure handle per
            // param (`app2(f, g, x)` with one nullary `mk` builds `mk`-equivalent closures for both f and
            // g), so one producer legitimately supplies several closure params.
            let pi = producers
                .iter()
                .enumerate()
                .position(|(pi, prod)| !used_producer[pi] && ty_matches(prod, cty, want_shape))
                .or_else(|| {
                    producers
                        .iter()
                        .position(|prod| ty_matches(prod, cty, want_shape))
                })?;
            used_producer[pi] = true;
            match &producers[pi] {
                Producer::Factory { ident, cap, .. } => {
                    if arg_i + cap > args.len() {
                        return None; // not enough args for this producer's captures
                    }
                    let caps = &args[arg_i..arg_i + cap];
                    arg_i += cap;
                    if async_mode {
                        // `block_on(prog::mk(&mut env, caps))` yields the (sync) `Rc<dyn Fn>` closure; bind it
                        // to a fresh `__gN` so the consumer call's `&mut env` borrow doesn't overlap it.
                        let g = format!("__g{}", async_lets.len());
                        let envcaps = if caps.is_empty() {
                            "&mut env".to_string()
                        } else {
                            format!("&mut env, {}", caps.join(", "))
                        };
                        async_lets.push(format!("let {g} = block_on(prog::{ident}({envcaps}));"));
                        call_args.push(g);
                    } else {
                        call_args.push(format!("prog::{ident}({})", caps.join(", ")));
                    }
                }
                Producer::Peeled {
                    ident,
                    fn_ty,
                    closure_ty,
                    ..
                } => {
                    // No cap args — wrap the peeled fn as the closure value (coerce fn-item → fn-ptr →
                    // the `dyn Fn` trait object). In ASYNC mode the peeled producer is an `async fn` (its
                    // signature is `fn(&mut E, …) -> impl Future`, NOT the sync `fn(…) -> ret`), so this
                    // fn-ptr coercion does not type — DECLINE the async peeled-producer consumer for now (a
                    // follow-up sub-slice; the async FACTORY-producer consumer is the bulk and lands here).
                    if async_mode {
                        return None;
                    }
                    call_args.push(format!(
                        "(std::rc::Rc::new(prog::{ident} as {fn_ty}) as {closure_ty})"
                    ));
                }
            }
        } else {
            // A non-closure param consumes exactly one verbatim call arg.
            if arg_i >= args.len() {
                return None;
            }
            call_args.push(args[arg_i].clone());
            arg_i += 1;
        }
    }
    // The consumer is called with the synthesized closure(s) + scalar args in param order.
    // SYNC: `prog::` is prepended by the caller's `call_or_await`/render, so return the bare `<name>(<args>)`.
    // ASYNC: the caller's async branch does NOT know how to thread `env`/`block_on` into a synthesized
    // consumer call (its shapes are nullary/args/factory), so this returns the FULLY-DRIVEN block itself — a
    // `{ let __g0 = …; block_on(prog::<name>(&mut env, <args>)) }` expression — and the caller uses it
    // verbatim (it is already `prog::`-qualified + `block_on`-wrapped, so `call_or_await` must NOT re-wrap it).
    if async_mode {
        let lets = async_lets.join(" ");
        Some(format!(
            "{{ {lets} block_on(prog::{name}(&mut env, {})) }}",
            call_args.join(", ")
        ))
    } else {
        Some(format!("{name}({})", call_args.join(", ")))
    }
}

fn rust_factory_param_count(module: &str, name: &str, async_mode: bool) -> Option<usize> {
    let sig = parse_emitted_sig(module, name, async_mode)?;
    // A FACTORY's return type is a closure (`-> …Rc<dyn Fn(` or async `-> …Rc<dyn EnvClosure<`) — only
    // then is this a producer.
    if !names_closure_value(&sig.ret_head) {
        return None;
    }
    // The factory's CAPTURE params = its source params minus the async env param (which is backend
    // plumbing, NOT a capture — counting it would misalign the flat `(call …)` args → captures|application
    // split, an E0061 arg-count mismatch + a lost application). Detect the env param by NAME rather than a
    // blind `-1`, so a hand-authored env-less async fixture stays correct.
    Some(sig.params.iter().filter(|p| !is_env_param(p)).count())
}

/// The async gate driver's harness: a no-limit `GateEnv` implementing the emitted `CdzEnv` (the gate
/// checks ANSWERS, not fuel bounds, so `consume` never blocks/panics) + a minimal `block_on` executor
/// (a real Waker is unneeded — the emitted futures never register one; they only `.await` `consume`,
/// which is `Ready` immediately, so a busy-poll loop drives them to completion). Spliced into the async
/// driver before `fn main`.
const ASYNC_GATE_HARNESS: &str = r#"
struct GateEnv;
impl cdz_rt::CdzEnv for GateEnv {
    async fn consume(&mut self, _gas: u64) {}
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker { raw() }
    fn raw() -> RawWaker { RawWaker::new(core::ptr::null(), &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let w = unsafe { Waker::from_raw(raw()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
"#;

/// A tiny FNV-1a hash of a string → a stable per-program key for the temp compile dir (no dependency;
/// `Date.now`/rng are unavailable and would break parallel determinism anyway).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Whether a NON-SUCCESS wasm run's stderr (with no `: trap:` line) carries a MEANINGFUL diagnostic — vs a
/// SILENT-DEATH crash that left nothing but informational noise (breaker's B1-sibling discriminator). A crash
/// (the run started then died) leaves only the `live-objects run on value-heap runtime` provenance banner
/// (printed pre-invoke) and possibly `host-call`/`host-arg` trace lines — all informational. A clean run
/// failure (a compose/instantiate REJECTION — `cdz-run: peer … mismatch` / does-not-export — or an exhausted
/// host-response) emits a STRUCTURED diagnostic line that corpus cases legitimately pin. Returns `true` iff a
/// line survives stripping those informational/trace lines: `true` → keep it as the trap reason (a real
/// failure); `false` → the run died without output (→ BadArtifact ICE-class). Extracted for unit tests.
fn run_failure_has_diagnostic(stderr: &str) -> bool {
    stderr.lines().map(str::trim).any(|l| {
        !l.is_empty()
            && !l.contains("live-objects run on value-heap runtime")
            && !l.starts_with("host-call\t")
            && !l.starts_with("host-arg\t")
    })
}

/// The OBSERVED host calls in cdz-run's stderr — each `host-call\t<op>` line's op, in emitted (call)
/// order (E2h). Empty when the run made no host call. The gate compares this sequence against a case's
/// recorded `(host-calls …)`.
fn observed_host_calls(stderr: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|l| l.strip_prefix("host-call\t").map(str::to_string))
        .collect()
}

/// The FIRST diagnostic CODE in rcdzc's stderr — the `CDZ####` inside the first `error [CODE]` line
/// (`rcdzc: error [CDZ0210] (node 3): …`). `None` if no line carries a bracketed code (a codeless
/// decline, or a warning-only run). Scans line-by-line for the first `error [` so a later warning's
/// code cannot shadow the error the corpus expects.
/// The FIRST error diagnostic's CODE **and** MESSAGE, recovered from `cdz compile` stderr — the
/// message half of the portable-diagnostic-test capability (operator seq353), so a corpus reject case
/// can pin a load-bearing phrase of the diagnostic prose, not just its `(error CODE)`. `cdz compile`
/// (the gate's emit path) prints one of two shapes on a rejection (verified on trunk): a CODED reject
/// `cdz: error [CDZ0101] (node 4): unbound name ...`, or an UNCODED decline `cdz: error: <full
/// message>` (a codeless decline's prose redirect).
/// Returns `(code, message)`: `code` is the `[CODE]` (`None` for an uncoded decline), and `message` is
/// the prose after the code + optional ` (node N):` locator (empty if no error line is found).
/// TOTAL over any stderr (never panics). Structured fix fields (kind/verified/edits) are NOT on this
/// path — `cdz check --json` emits those, a SEPARATE later increment's data source.
fn first_error_diag(stderr: &[u8]) -> (Option<String>, String) {
    for line in String::from_utf8_lossy(stderr).lines() {
        // Coded: `… error [CODE]…: message`. Code between `[` and `]`; message after the first `: `
        // that follows the `]` (skipping the optional ` (node N)` locator the compiler inserts).
        if let Some((_, after_err)) = line.split_once("error [")
            && let Some((code, rest)) = after_err.split_once(']')
            && !code.trim().is_empty()
        {
            let message = rest.split_once(": ").map(|(_, m)| m).unwrap_or("").trim();
            return (Some(code.trim().to_string()), message.to_string());
        }
        // Uncoded decline: `… error: <message>` (no `[CODE]`). Only fires when there is no `error [`.
        if !line.contains("error [")
            && let Some((_, msg)) = line.split_once("error: ")
        {
            return (None, msg.trim().to_string());
        }
    }
    (None, String::new())
}

/// Whether a RUNTIME failure reason is an ARTIFACT-ICE: a compile-success-but-unloadable component (wasmtime
/// `Component::new`/instantiate rejects it) — the "compiler said yes and produced garbage" ICE (breaker's B1).
/// Never a legitimate runtime trap, so it FAILs regardless of expectation kind (the trap-expectation channel
/// otherwise swallows it as Todo, since it classifies to no `TrapCode`). MIRROR of `cdz_corpus_grade`.
fn is_artifact_ice(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    r.contains("invalid component")
        || r.contains("failed to parse webassembly")
        || r.contains("failed to instantiate")
        || r.contains("instantiate component")
}

/// EVERY warning diagnostic on a SUCCESSFUL compile, recovered from `cdz compile` stderr — the
/// `(warns CODE (message "…"))` clause of the portable-diagnostic-test capability (operator seq353,
/// inc2). Unlike an error (first-wins), a clean compile can emit a SET of warnings (e.g. two unused
/// bindings → two lines), so this scans ALL lines matching `warning [CODE] (node N): message` and
/// collects every `(code, message)` pair — a case then asserts PRESENCE (some captured warning matches
/// a `(warns CODE (message …))` clause). Format is exactly parallel to the error line (v-diagnostics:
/// stable across CDZ0305/0306/0213/0308). TOTAL over any stderr (never panics); empty when none.
fn collect_warnings(stderr: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(stderr).lines() {
        if let Some((_, after_warn)) = line.split_once("warning [")
            && let Some((code, rest)) = after_warn.split_once(']')
            && !code.trim().is_empty()
        {
            let message = rest.split_once(": ").map(|(_, m)| m).unwrap_or("").trim();
            out.push((code.trim().to_string(), message.to_string()));
        }
    }
    out
}

/// Options for `gate` (grows without re-threading a widening arg list).
struct GateOpts {
    files: Vec<PathBuf>,
    store: Option<PathBuf>,
    case: Option<String>,
    save: bool,
    check: bool,
    target: GateTarget,
    /// `Some((i, n))` (1-based `i`) to run only shard `i` of `n` — a deterministic round-robin partition
    /// of the flat CASE list (every `n`-th case), so shards balance regardless of per-file case counts.
    /// `None` runs the whole (default or given) set.
    shard: Option<(usize, usize)>,
    /// GUARDED-ALL: run every case on the debug-counters runtime (generation guard fires corpus-wide) for
    /// deterministic under-retain/UAF verification of a global escape/RC change. Forces the in-process
    /// path and fail-fasts if the store's debug runtime is missing/stale.
    guarded_all: bool,
}

/// Run one or more corpus files through the pipeline and grade each case against its recorded
/// outcome. Delegates case parsing + normalization to `cdz-syntax corpus`, then drives each program.
/// The nix flake system string (`aarch64-linux`), for the `.#checks.<sys>.…` attr path. Read once from
/// `builtins.currentSystem` (cheap eval); falls back to `<arch>-linux` if nix is unavailable.
fn nix_current_system() -> String {
    std::process::Command::new("nix")
        .args([
            "eval",
            "--raw",
            "--impure",
            "--expr",
            "builtins.currentSystem",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}-linux", std::env::consts::ARCH))
}

/// The cached-corpus check attr for a corpus `.sexp` file at `target`, or `None` if the file is not a
/// recognized `NN-feature` corpus file (only those have a `corpus[-rust][-async]-<stem>` nix check).
/// Whole-corpus (no file) uses the top-level `corpus`/`corpus-rust`/`corpus-rust-async` aggregate.
fn corpus_check_attr(target: GateTarget, stem: Option<&str>) -> Option<String> {
    let prefix = match target {
        GateTarget::Wasm => "corpus",
        GateTarget::Rust => "corpus-rust",
        // rust-async gained a cached per-case nix check (#4728: `corpus-rust-async[-<stem>]`, graded vs
        // `.gate-baseline-rust-async`), so it delegates like wasm/rust instead of running in-process.
        GateTarget::RustAsync => "corpus-rust-async",
        // No nix per-case check wired yet — the cadenza round-trip runs IN-PROCESS (excluded from the
        // `gate_via_nix_cache` short-circuit), so this prefix is reserved for the future `corpus-cadenza`
        // check but not yet consumed.
        GateTarget::Cadenza => "corpus-cadenza",
    };
    match stem {
        None => Some(prefix.to_string()),
        // A per-file check exists only for the `NN-feature` corpus files (numeric prefix), matching the
        // flake's `corpusFileNames`.
        Some(s) if s.starts_with(|c: char| c.is_ascii_digit()) => Some(format!("{prefix}-{s}")),
        Some(_) => None,
    }
}

/// Why an interactive `gate` fell to the UNCACHED in-process path (for the fleet-load advisory). Pure so
/// the arm selection is unit-testable; the caller has already excluded the `--save`/`--check`/`--shard`
/// pipeline flows. Order matters: `--case` is structural (no cached check exists), the explicit
/// `CDZ_GATE_INPROCESS` opt-out is next, and everything else is an unavailable/unmapped cache.
fn gate_inprocess_reason(has_case: bool, inprocess_env: bool) -> &'static str {
    if has_case {
        "--case runs in-process (single-case debug)"
    } else if inprocess_env {
        "CDZ_GATE_INPROCESS=1 forces the in-process gate"
    } else {
        "the cached nix corpus was unavailable (nix missing/failed, or a file isn't an NN-feature corpus file)"
    }
}

/// Total corpus CASE count across the requested files (all of `spec/semantics/*.sexp` when empty), counted
/// cheaply by matching top-level `(case "` lines (no shred/nix eval). Used to scale the nix-cache build's
/// wall-clock cap to the real work: a cold / compiler-changed build rebuilds ONE derivation per case, so a
/// flat cap starves the fleet's mega-chapters (615/928 cases) while a case-count-proportional cap stays a
/// meaningful hang bound for every chapter size.
fn nix_gate_case_count(paths: &Paths, files: &[PathBuf]) -> u64 {
    let list: Vec<PathBuf> = if files.is_empty() {
        default_corpus_files(&paths.repo)
    } else {
        files.to_vec()
    };
    list.iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.lines().filter(|l| l.starts_with("(case \"")).count() as u64)
        .sum()
}

/// Run the corpus gate through the CACHED per-case nix corpus (`.#checks.<sys>.corpus[-rust][-<stem>]`)
/// instead of recompiling every case in-process — the operator's "don't rebuild the world on every gate"
/// (2026-08-26; ships the #3363 per-case caching as the gate agents run). A corpus-only edit re-runs ONLY
/// the changed case (content-addressed build); a compiler edit re-runs builds but the exec cache-hits on
/// identical emit. The nix exec reproduces `xtask gate --check` (baseline regression + the exec_exit rule),
/// so this is regression-GATED by construction. Returns `Some(exit_code)` when it delegated, or `None` to
/// fall through to the in-process path (an unrecognized `--files` entry, or nix unavailable). wasm/rust/
/// rust-async all have a cached check now; `--save`/`--shard`/`--case` stay in-process (caller).
fn gate_via_nix_cache(paths: &Paths, files: &[PathBuf], target: GateTarget) -> Option<i32> {
    let sys = nix_current_system();
    // Map the requested files → check attrs. Empty ⇒ the whole-corpus aggregate.
    let attrs: Vec<String> = if files.is_empty() {
        vec![corpus_check_attr(target, None)?]
    } else {
        let mut v = Vec::with_capacity(files.len());
        for f in files {
            let stem = f.file_stem().and_then(|s| s.to_str());
            // Any file we can't map to a cached check ⇒ fall through to in-process (don't silently skip it).
            v.push(corpus_check_attr(target, stem)?);
        }
        v
    };
    let installables: Vec<String> = attrs
        .iter()
        .map(|a| format!(".#checks.{sys}.{a}"))
        .collect();
    println!(
        "gate: CACHED corpus build via nix ({}) — reuses per-case results, no rebuild-the-world \
         (set CDZ_GATE_INPROCESS=1 to force the in-process gate)",
        installables.join(" ")
    );
    let mut cmd = std::process::Command::new("nix");
    cmd.current_dir(&paths.repo)
        .arg("build")
        .args(&installables)
        .args(["-L", "--keep-going"]);
    // Operator directive (2026-08-28, via concierge; v-nix substituter RCA): these `.#checks.…corpus-*`
    // outputs are `__contentAddressed` + LOCAL-ONLY, so querying remote substituters (cache.nixos.org /
    // install.determinate.systems) for their realisations is a guaranteed MISS — pure network-query waste
    // that stalled a whole-corpus sweep ~1.5h. Skip substitution for THESE builds only (scoped here at the
    // corpus-build chokepoint — NOT a global nix.conf `substitute = false`, so deliberate dep-fetch paths
    // like cache-warm keep the default and still fetch toolchain deps from cache). Escape: set
    // `CDZ_GATE_SUBSTITUTE=1` to re-enable — e.g. the first corpus build right after a flake.lock bump,
    // when a new rustc/binaryen must come from cache instead of building from source.
    if std::env::var_os("CDZ_GATE_SUBSTITUTE").is_none() {
        cmd.args(["--option", "substitute", "false"]);
    }
    let child = match cmd
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gate: could not launch nix ({e}); falling back to the in-process gate");
            return None;
        }
    };
    // Bound the build so a hung nix builder can't freeze an agent's gate forever — but GENEROUSLY, and
    // PROPORTIONAL TO WORK: a COLD build (first run / after a compiler change re-emits every case) rebuilds
    // one derivation per case, so the honest wall-clock scales with the CASE COUNT. A flat cap starved the
    // fleet's mega-chapters (14=615, 14b=502, 14c=928 cases): under load a full re-gate of one legitimately
    // exceeds a flat 45min and gets killed mid-build (v-effects blocked, concierge 2026-08-31), so no
    // run-value change in a 500+-case chapter could be full-chapter-gated. Scale the cap by the requested
    // files' total case count (≈10s/case, a generous per-case rebuild+exec budget under contention) with a
    // 45min FLOOR for small runs — the cap stays a MEANINGFUL hang bound (a true hang still fails, just at a
    // work-proportional deadline), it is not a throttle. `CDZ_GATE_NIX_TIMEOUT_SECS` still overrides
    // explicitly (wins over the scaled value). NOTE: this raises the LOCAL agent hang-bound; PARALLEL
    // case-sharding into multiple cap-fitting jobs (splitting one chapter's case derivations across builds)
    // is a heavier flake+xtask follow-up (co-owned with v-nix) if the proportional bound proves insufficient.
    let cap = match std::env::var("CDZ_GATE_NIX_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s > 0)
    {
        Some(explicit) => std::time::Duration::from_secs(explicit),
        None => {
            let cases = nix_gate_case_count(paths, files);
            let secs = (cases.saturating_mul(10)).max(45 * 60);
            std::time::Duration::from_secs(secs)
        }
    };
    match wait_with_timeout(child, cap) {
        Ok(Some(output)) => Some(output.status.code().unwrap_or(1)),
        Ok(None) => {
            eprintln!(
                "gate: nix cached corpus build exceeded its wall-clock cap ({}s, scaled by case count; \
                 killed). Override with CDZ_GATE_NIX_TIMEOUT_SECS if this is a legitimately-heavy build \
                 under load, not a hang.",
                cap.as_secs()
            );
            Some(1)
        }
        Err(e) => {
            eprintln!("gate: waiting on nix failed ({e}); falling back to the in-process gate");
            None
        }
    }
}

fn gate(paths: &Paths, profile: &str, opts: GateOpts) {
    // FIDELITY GUARD (v-rcdzc-ts-1 + concierge nix-arbitration, 2026-08-31): the native IN-PROCESS gate grade
    // DIVERGES from the authoritative nix corpus-exec grade in BOTH directions — it false-FAILs cases nix
    // PASSES (corpus-05 "runtime Qty returned WITH its unit") AND MISSES fails nix CATCHES (corpus-18 "Qty
    // with a bad inner type", CDZ0101 message check). A FRESH nix store did NOT fix it → the divergence is the
    // native PATH, not store staleness. So a baseline SAVED from this native grade is UNFAITHFUL (it masks
    // real regressions + bakes false-fails). The ONLY faithful baseline mechanism is the NIX harvest. REFUSE
    // `--save` here and direct to it, so no one re-produces an unfaithful baseline (as an aborted attempt
    // already did). This completes the v-xtask-decompose seq-202 intent (`xtask-save-baseline` / the nix app
    // is the `gate --save` replacement). Escape `CDZ_GATE_SAVE_NATIVE_ANYWAY=1` (DISCOURAGED — unfaithful).
    if opts.save && std::env::var_os("CDZ_GATE_SAVE_NATIVE_ANYWAY").is_none() {
        eprintln!(
            "xtask gate --save: REFUSED — the native in-process grade DIVERGES from the authoritative nix \
             corpus-exec grade (both ways: false-fails + missed fails, proven corpus-05/18), so a baseline \
             saved from it is UNFAITHFUL. Regenerate via the NIX harvest instead:\n\
             \x20 nix run .#save-baseline   (builds .#corpus-verdicts = the faithful nix grade, writes the \
             .gate-baseline; run per backend — wasm/rust/rust-async)\n\
             (override, discouraged, produces an unfaithful baseline: CDZ_GATE_SAVE_NATIVE_ANYWAY=1)"
        );
        std::process::exit(2);
    }
    // OPERATOR (2026-08-26 "cut over to the faster/cached wasm builds"): the whole-corpus + `--files`
    // verify path runs through the CACHED per-case nix corpus so it does NOT recompile every case each run
    // (#3363). `--case` (single-case debug), `--save` (baseline regen — needs in-process verdicts),
    // and `--shard` (case-sharded nightly) stay in-process below.
    // Escape hatch: `CDZ_GATE_INPROCESS=1`. Regression-gated by construction (the nix exec runs `--baseline`).
    // `--check` stays IN-PROCESS: it also does VANISHED detection (a baseline case with no run), which the
    // per-case `corpus` build doesn't — so pr-sync's authoritative `gate --check` keeps its full
    // regression+vanished semantics untouched. (The cached nix `corpus`/`corpus-vanished` checks cover both
    // for CI separately.) The delegated path is regression-gated by construction anyway (the nix exec runs
    // `--baseline`), so an agent's plain `gate <files>` still catches a pass→not-pass regression.
    if opts.case.is_none()
        && !opts.save
        && !opts.check
        && opts.shard.is_none()
        && matches!(
            opts.target,
            GateTarget::Wasm | GateTarget::Rust | GateTarget::RustAsync
        )
        && !opts.guarded_all
        && std::env::var_os("CDZ_GATE_INPROCESS").is_none()
        && let Some(code) = gate_via_nix_cache(paths, &opts.files, opts.target)
    {
        std::process::exit(code);
    }

    // FLEET-LOAD ADVISORY (concierge/operator 2026-08-27): control reached the IN-PROCESS path, so the
    // grade below builds the compiler (uncached, shared with no peer) and grades the cases across all
    // cores — this native gate was the ~55 host-load-spike source v-nix traced. Warn on the INTERACTIVE
    // fallback (an agent's spot-check) so the uncached run is VISIBLE and point at the cached nix check;
    // stay quiet for the sanctioned pipeline flows (`--save` baseline regen, `--check` pr-sync bar,
    // `--shard` nightly), which legitimately need in-process verdicts. Non-blocking — it still runs.
    if !opts.save && !opts.check && opts.shard.is_none() {
        let reason = gate_inprocess_reason(
            opts.case.is_some(),
            std::env::var_os("CDZ_GATE_INPROCESS").is_some(),
        );
        eprintln!(
            "⚠ xtask gate: UNCACHED in-process pipeline — builds the compiler + grades cases across all \
             cores, shared with no peer (this native gate was flagged as a fleet host-load-spike source). \
             Reason: {reason}."
        );
        // The cached, fleet-shared equivalent for any files that DO have a per-case nix check (wasm/rust
        // NN-feature corpus files). Only shell out for the system string when there's a hint to print.
        let cached: Vec<String> = opts
            .files
            .iter()
            .filter_map(|f| corpus_check_attr(opts.target, f.file_stem().and_then(|s| s.to_str())))
            .collect();
        if !cached.is_empty() {
            let sys = nix_current_system();
            let cmd = cached
                .iter()
                .map(|a| format!(".#checks.{sys}.{a}"))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("  Prefer the CACHED, fleet-shared check:  nix build {cmd}");
        } else {
            eprintln!(
                "  (no cached nix check exists for this invocation — a routine corpus spot-check on an \
                 `NN-feature` file with `--target wasm` IS cached; use that path when you can.)"
            );
        }
        eprintln!("  Proceeding in-process…");
    }

    let tools = build_tools(paths, profile);
    // `--guarded-all`: verify the debug-counters runtime is present + fresh BEFORE running, then export the
    // mode so `run_program_wasm` routes every case through it. A missing/stale debug runtime would silently
    // degrade to the release runtime (no generation guard → a false-clean UAF verification), so FAIL FAST
    // rather than mislead — this flag exists precisely to avoid that false-clean trap.
    if opts.guarded_all {
        match resolve_debug_runtime(&opts.store) {
            Some(path) => {
                let stale = tools
                    .debug_runtime_hash
                    .as_deref()
                    .zip(path.file_stem().and_then(|s| s.to_str()))
                    .map(|(committed, store_hash)| store_hash != committed)
                    .unwrap_or(false);
                if stale {
                    eprintln!(
                        "xtask gate --guarded-all: ABORT — STALE debug runtime in store \
                         (!= committed DEBUG_RUNTIME_HASH). A stale runtime would false-clean the UAF \
                         guard; run `cargo xtask build` first."
                    );
                    std::process::exit(2);
                }
            }
            None => {
                eprintln!(
                    "xtask gate --guarded-all: ABORT — debug-counters runtime not in store. The guarded \
                     verification needs it; run `cargo xtask build` first."
                );
                std::process::exit(2);
            }
        }
        // SAFETY: set once at gate entry, before the parallel grade fan-out spawns; the worker threads only
        // READ it (via `run_program_wasm`) and never write, so there is no concurrent-writer data race.
        unsafe { std::env::set_var("CDZ_GATE_GUARDED_ALL", "1") };
        eprintln!(
            "xtask gate --guarded-all: every case runs on the debug-counters runtime — its generation \
             guard (assert_node_live) is the corpus-wide under-retain/UAF witness."
        );
    }
    let files = if opts.files.is_empty() {
        default_corpus_files(&paths.repo)
    } else {
        opts.files.clone()
    };

    // `--case`: run only matching cases and print each one's program / expected / actual — the
    // single-case debug loop, not a pass/fail tally.
    if let Some(needle) = &opts.case {
        gate_one_case(&tools, &opts.store, &files, needle, opts.target);
        return;
    }

    // Gather every case (file then case order) into one flat list, then grade them in PARALLEL. Each
    // case is independent — `grade` only READS `tools`/`store` and spawns its own subprocess pipeline,
    // with no shared mutable state — so grading is embarrassingly parallel. The serial loop spent ~all
    // its wall time waiting on ~3 spawned processes per case (cdz-syntax → rcdzc → cdz-run) over 1000+
    // cases; fanning the cases across cores collapses that wait. Order is PRESERVED (each result is
    // written to its own index) because the baseline compare (`check_baseline`/`save_baseline`) is
    // positional — a race in verdict order would spuriously flag regressions.
    let mut records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools.corpus, file))
        .collect();
    // `--shard I/N`: keep only every `n`-th case (offset `i-1`) of the flat, order-stable case list. Case-
    // level (not file-level) so shards balance even though per-file case counts vary wildly (e.g. one file
    // holds ~1774 cases). Parsing all files is cheap; the cost is the per-case compile, which is what we
    // split. `--check` is scoped to the run cases (see `check_baseline`'s `subset`), so the other shards'
    // cases are not treated as regressions/vanished.
    if let Some((i, n)) = opts.shard {
        let before = records.len();
        records = records
            .into_iter()
            .enumerate()
            .filter(|(k, _)| k % n == i - 1)
            .map(|(_, r)| r)
            .collect();
        println!("gate: shard {i}/{n} — {} of {before} cases", records.len());
    }
    let graded = grade_all_parallel(&tools, &opts.store, records, opts.target);

    // Reassemble the tally + the ordered verdict list from the in-order graded results.
    let (mut pass, mut todo, mut fail) = (0u32, 0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();
    let mut verdicts: Vec<(String, Verdict)> = Vec::new();
    for (description, grade) in graded {
        let v = match grade {
            Grade::Pass => {
                pass += 1;
                Verdict::Pass
            }
            Grade::Todo => {
                todo += 1;
                Verdict::Todo
            }
            Grade::Fail(why) => {
                fail += 1;
                failures.push(format!("{description}: {why}"));
                Verdict::Fail
            }
        };
        verdicts.push((description, v));
    }

    println!("\ngate: {pass} pass, {todo} todo, {fail} fail");
    if !failures.is_empty() {
        println!("\nfailures:");
        for f in &failures {
            println!("  FAIL  {f}");
        }
    }

    if opts.save {
        save_baseline(paths, &verdicts, opts.target);
        println!(
            "\nbaseline saved: {} cases → {}",
            verdicts.len(),
            baseline_path(paths, opts.target).display()
        );
        return;
    }
    if opts.check {
        // A regression (a case that used to pass and now doesn't) fails the check even if the
        // totals look fine — the trap the raw counts hide. Newly-passing cases are reported, not failed.
        std::process::exit(check_baseline(
            paths,
            &verdicts,
            opts.target,
            opts.shard.is_some(),
        ));
    }
    if fail > 0 {
        std::process::exit(1);
    }
}

/// The observable-outcome KEY the opt-sweep compares across optimization levels — the projection every
/// level must preserve. Two runs are level-EQUIVALENT iff their keys are equal; a divergence in this key
/// across O0..O3 is a candidate miscompile. A value carries its host-call trace; a trap keys on its
/// classified KIND ([`classify`]) or — for an UNCLASSIFIED reason — the RAW first line (NOT a single
/// "other" bucket, else two genuinely-different unclassified traps would compare EQUAL and a real
/// cross-level trap divergence among them would be missed). Extracted as a free fn so the comparison the
/// blocking gate rests on is unit-tested (see `opt_sweep_outcome_key_*` tests).
fn sweep_outcome_key(ran: &Ran) -> String {
    match ran {
        Ran::Value(v, calls, _) if calls.is_empty() => format!("value {v}"),
        Ran::Value(v, calls, _) => format!("value {v} [host: {}]", calls.join(",")),
        Ran::Trap(msg) => match classify(msg) {
            Some(code) => format!("trap {}", code.code()),
            None => format!("trap raw:{}", first_line(msg.as_bytes())),
        },
        Ran::Declined { code, .. } => format!("declined {}", code.as_deref().unwrap_or("-")),
        Ran::BadArtifact(msg) => format!("bad-artifact {msg}"),
    }
}

/// The OPTIMIZATION-LEVEL-EQUIVALENCE gate (`xtask gate --opt-sweep`). The tiered `OptLevel` framework's
/// correctness invariant is that EVERY level produces OBSERVABLY-IDENTICAL behavior — only compile time /
/// output size differs, never the result (`core-semantics.md` §Observable Behavior Is A Defined
/// Projection Of A Run; DESIGN-tiered-optimization-levels-rcdzc.md §5). This mode compiles AND RUNS each
/// corpus program at `O0`/`O1`/`O2`/`O3` and asserts the OBSERVABLE OUTCOME (value / trap / decline) is
/// the SAME across all four. A level that changes the outcome is a candidate miscompile: a hard fail.
///
/// It compares the RUN OUTCOME, not the emitted bytes: the wasm emit is not byte-deterministic
/// run-to-run (map-iteration order in selection), so a byte diff has false positives — the observable
/// projection (the value a run yields, or the trap/decline) is the real invariant the levels must
/// preserve. Each case is driven per its trials (its `(call …)`s); a case that DECLINES at the default
/// tier is skipped (a decline is level-independent; the normal gate grades it Todo). Multi-file PACKAGE
/// cases ARE covered (`emit_component_package` threads the opt level, so each level recompiles the whole
/// package) — extending the guard to multi-module programs where a cross-module/inlining pass could
/// mis-optimize. Wired into `cargo xtask check` (operator directive 2026-07-17) → a HARD BLOCKING
/// merge gate: because pr-sync re-gates every MR via `check`, a cross-level divergence rejects the MR. It
/// guards nothing while the `PassManager` pipeline is empty (every level runs identically), but stands
/// ready to catch fleet-wide any future Core pass that mis-optimizes at `O2`/`O3`.
fn gate_opt_sweep(paths: &Paths, profile: &str, opts: &GateOpts) {
    // The sweep honors `--target wasm` (default), `--target rust`, and `--target rust-async` — each drives
    // its own compile+run path with the opt level threaded through (`run_program_wasm` /
    // `run_program_rust`), so the level-equivalence guard covers BOTH backends (the v-core-opt charter's
    // both-backend correctness bar).
    let tools = build_tools(paths, profile);
    let files = if opts.files.is_empty() {
        default_corpus_files(&paths.repo)
    } else {
        opts.files.clone()
    };
    const LEVELS: [&str; 4] = ["O0", "O1", "O2", "O3"];

    // The observable outcome of a run, as a comparable string — the projection the levels must preserve.
    // A host-delegating case (non-empty host_responses) is level-independent in its host protocol, so its
    // observed calls are folded into the value string like the normal gate renders them.
    // Gather every case, then sweep them IN PARALLEL — like `grade_all_parallel`, the work is
    // process-bound (each level is a full compile+run subprocess pipeline), so a worker pool pulling from
    // a shared cursor keeps many pipelines in flight. Order is irrelevant here (we only tally + collect
    // divergences, no positional baseline), so a simple lock-collected result set suffices.
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools.corpus, file))
        .collect();
    let (checked, skipped, divergences) = {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
        let checked = AtomicU32::new(0);
        let skipped = AtomicU32::new(0);
        let divergences: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let cursor = AtomicUsize::new(0);
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (cursor, checked, skipped, divergences, records, tools, store, target) = (
                    &cursor,
                    &checked,
                    &skipped,
                    &divergences,
                    &records,
                    &tools,
                    &opts.store,
                    opts.target,
                );
                scope.spawn(move || {
                    loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= records.len() {
                            break;
                        }
                        match sweep_one_case(tools, store, &records[i], &LEVELS, target) {
                            SweepOutcome::Skipped => {
                                skipped.fetch_add(1, Ordering::Relaxed);
                            }
                            SweepOutcome::Checked(diffs) => {
                                checked.fetch_add(1, Ordering::Relaxed);
                                if !diffs.is_empty() {
                                    divergences.lock().unwrap().extend(diffs);
                                }
                            }
                        }
                    }
                });
            }
        });
        (
            checked.into_inner(),
            skipped.into_inner(),
            divergences.into_inner().unwrap(),
        )
    };

    println!(
        "\ngate --opt-sweep: {checked} checked ({skipped} skipped: declines-at-default), {} divergence(s) across O0..O3",
        divergences.len()
    );
    if !divergences.is_empty() {
        println!("\ndivergences:");
        for d in &divergences {
            println!("  DIVERGE  {d}");
        }
        std::process::exit(1);
    }
    println!("all checked cases run to the SAME outcome at every optimization level ✓");
}

/// The per-case result of the opt-sweep: either the case was skipped (multi-file package, or it declines
/// at the default tier — a decline is level-independent), or it was checked and yielded zero-or-more
/// divergence messages (a level whose observable outcome differs from O1's).
enum SweepOutcome {
    Skipped,
    Checked(Vec<String>),
}

/// Sweep ONE corpus case across all optimization `levels`: for each trial (`(call …)`, or one implicit
/// no-call run), run the program at every level and compare the observable `outcome` to the O1 baseline.
/// Returns the divergences found (empty = the case is level-equivalent). Pure per-case work (own
/// subprocess pipelines, no shared mutable state), so the sweep runs these in parallel.
fn sweep_one_case(
    tools: &Tools,
    store: &Option<PathBuf>,
    rec: &CorpusRecord,
    levels: &[&str],
    target: GateTarget,
) -> SweepOutcome {
    // Multi-file PACKAGE cases are covered too: both backends thread the opt level through their package
    // emit (`emit_component_package` / `emit_rust_package`) so each level recompiles the whole package,
    // extending the level-equivalence guard to multi-module programs — exactly where a future
    // cross-module/inlining Core pass could mis-optimize. (A case that DECLINES at the default tier is
    // still skipped below — level-independent.) Each trial (a `(call …)`, or the single no-call trial) is
    // run at every level and compared. The run path follows `--target`: wasm (default) drives the wasm
    // pipeline, rust/rust-async the rustc pipeline (a host-delegating case declines under rust, so it is
    // skipped as level-independent just like a default decline).
    let calls: Vec<Option<&Call>> = if rec.trials.is_empty() {
        vec![None]
    } else {
        rec.trials.iter().map(|t| t.call.as_ref()).collect()
    };
    let run_at = |lvl: &str, call: Option<&Call>| -> Ran {
        match target {
            GateTarget::Wasm => run_program_wasm(
                tools,
                store,
                &rec.program,
                &rec.modules,
                &rec.peers,
                call,
                &rec.host_responses,
                Some(lvl),
                rec.wit_world.as_deref(),
                rec.component_name.as_deref(),
                LiveObjectsCheck::Off, // the opt sweep looks for a tier divergence, not a heap-balance regression
            ),
            // A host-delegating case is level-independent in its host protocol (the opt sweep looks for a
            // TIER divergence, not a host-boundary regression), so it declines here and is skipped below —
            // exactly as a default decline is. A case is host-delegating if it records responses OR calls
            // (a unit-result effect op records a call but no response — H8). Mirror the normal-gate dispatch.
            GateTarget::Rust | GateTarget::RustAsync
                if !rec.host_responses.is_empty() || !rec.host_calls.is_empty() =>
            {
                Ran::Declined {
                    code: None,
                    message: String::new(),
                }
            }
            GateTarget::Rust => {
                // Host cases declined above (level-independent) → no responses/calls reach here → `&[]`.
                run_program_rust(
                    tools,
                    &rec.program,
                    &rec.modules,
                    call,
                    false,
                    Some(lvl),
                    &[],
                    &[],
                )
            }
            GateTarget::RustAsync => run_program_rust(
                tools,
                &rec.program,
                &rec.modules,
                call,
                true,
                Some(lvl),
                &[],
                &[],
            ),
            // The cadenza round-trip is not part of the optimization-level-equivalence sweep (it is its own
            // `--target cadenza` gate, run without `--opt-sweep`) → decline here so it is skipped.
            GateTarget::Cadenza => Ran::Declined {
                code: None,
                message: String::new(),
            },
        }
    };
    let mut diffs = Vec::new();
    let mut checked_any = false;
    for call in calls {
        let runs: Vec<Ran> = levels.iter().map(|lvl| run_at(lvl, call)).collect();
        // A decline at the default tier means the case doesn't compile — skip it (level-independent).
        let default_idx = levels.iter().position(|l| *l == "O1").unwrap_or(0);
        if matches!(&runs[default_idx], Ran::Declined { .. }) {
            return SweepOutcome::Skipped;
        }
        checked_any = true;
        let base = sweep_outcome_key(&runs[default_idx]);
        for (lvl, ran) in levels.iter().zip(&runs) {
            let got = sweep_outcome_key(ran);
            if got != base {
                let label = call.map(|c| c.export.as_str()).unwrap_or("(no call)");
                diffs.push(format!(
                    "{} [{label}]: {lvl} → `{got}`, O1 → `{base}` — LEVELS DIVERGE (candidate miscompile)",
                    rec.description
                ));
            }
        }
    }
    if checked_any {
        SweepOutcome::Checked(diffs)
    } else {
        SweepOutcome::Skipped
    }
}

/// Grade every record in PARALLEL, returning `(description, grade)` in the SAME order as `records`.
///
/// The work is process-bound (each `grade` spawns cdz-syntax → rcdzc → cdz-run and waits), so a pool
/// of worker threads pulling from a shared cursor keeps many pipelines in flight at once. Uses
/// `std::thread::scope` — the workers borrow `tools`/`store`/`records` for the scope's lifetime, so no
/// `Arc`/clone and no extra dependency. Order is preserved by writing each result into its own slot
/// (indexed by the case's position), never by push order — the baseline compare is positional, so a
/// reordering would read as a spurious regression. The worker count is bounded by the machine's
/// parallelism but not below 1; the cases-per-core ratio is high, so a simple shared atomic cursor
/// (no work-stealing) balances well enough.
fn grade_all_parallel(
    tools: &Tools,
    store: &Option<PathBuf>,
    records: Vec<CorpusRecord>,
    target: GateTarget,
) -> Vec<(String, Grade)> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let n = records.len();
    // One result slot per case, filled by index (order-preserving). `Mutex<Option<_>>` is the simplest
    // sound cell here; contention is nil (each slot is written once, by one worker).
    let slots: Vec<Mutex<Option<(String, Grade)>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let cursor = AtomicUsize::new(0);

    // Cap workers at the machine's parallelism (min 1). More threads than cores buys nothing here —
    // the pipeline stages are separate processes the OS already schedules across cores. `CDZ_GATE_JOBS`
    // (a positive integer) caps it lower: each worker holds a live rustc/cdz-run subprocess, so on a
    // memory-tight runner a lower cap bounds peak RSS (the sharded nightly's OOM lever) at the cost of
    // parallelism. An unset/zero/unparseable value falls back to the core count.
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);
    let workers = std::env::var("CDZ_GATE_JOBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&j| j >= 1)
        .map_or(cores, |j| j.min(cores));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let cursor = &cursor;
            let slots = &slots;
            let records = &records;
            scope.spawn(move || {
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= records.len() {
                        break;
                    }
                    let rec = &records[i];
                    let grade = grade(tools, store, rec, target);
                    *slots[i].lock().unwrap() = Some((rec.description.clone(), grade));
                }
            });
        }
    });

    // All workers joined at scope end; every slot is filled exactly once.
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().expect("every case graded"))
        .collect()
}

/// Run only the case(s) whose description contains `needle`, printing each one's normalized program,
/// expected result, and actual outcome. A focused debug view, not a tally.
fn gate_one_case(
    tools: &Tools,
    store: &Option<PathBuf>,
    files: &[PathBuf],
    needle: &str,
    target: GateTarget,
) {
    let mut found = 0;
    for file in files {
        for rec in read_corpus(&tools.corpus, file) {
            if !rec.description.contains(needle) {
                continue;
            }
            found += 1;
            // Run each trial (re-driving the program per its `(call …)`) so the debug view shows every
            // call/expect/actual line — a multi-trial case lists them in order.
            let rans: Vec<Ran> = rec
                .trials
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    // Heap balance is checked on the FIRST trial only (see `grade`).
                    let live_objects = if i == 0 {
                        LiveObjectsCheck::from_record(rec.live_objects, rec.live_objects_known_leak)
                    } else {
                        LiveObjectsCheck::Off
                    };
                    run_program(
                        tools,
                        store,
                        &rec.program,
                        &rec.modules,
                        &rec.peers,
                        t.call.as_ref(),
                        &rec.host_responses,
                        &rec.host_calls,
                        rec.wit_world.as_deref(),
                        rec.component_name.as_deref(),
                        live_objects,
                        target,
                    )
                })
                .collect();
            let verdict = match grade_ran(&rec, &rans, target) {
                Grade::Pass => "PASS",
                Grade::Todo => "todo",
                Grade::Fail(_) => "FAIL",
            };
            println!("case:     {}", rec.description);
            println!("program:  {}", rec.program);
            for (trial, ran) in rec.trials.iter().zip(&rans) {
                if let Some(call) = &trial.call {
                    println!("call:     {} {}", call.export, call.args.join(" "));
                }
                let actual = match ran {
                    Ran::Value(v, calls, _) if calls.is_empty() => format!("value {v}"),
                    Ran::Value(v, calls, _) => {
                        format!("value {v} [host-calls: {}]", calls.join(", "))
                    }
                    Ran::Declined { code: Some(c), .. } => format!("rejected [{c}]"),
                    // A code-less decline whose message is an ICE signature is graded FAIL — the `actual:`
                    // label must FOLLOW that classification (not the generic "compiler can't compile it yet",
                    // which reads as an honest capability gap). breaker's cosmetic catch on #4523.
                    Ran::Declined {
                        code: None,
                        message,
                    } if is_ice_signature(message) => {
                        format!("ICE — compiler bug, declined with no diagnostic code: {message}")
                    }
                    // A code-less HONEST decline: SHOW its message so a NEW ICE-flavored signature not yet in
                    // `is_ice_signature` is DISCOVERABLE in the output (breaker's B2 — the label used to DROP
                    // the message, hiding candidate signatures). A truly silent decline (empty message) keeps
                    // the generic phrasing.
                    Ran::Declined {
                        code: None,
                        message,
                    } if !message.is_empty() => {
                        format!(
                            "declined, no diagnostic code (compiler can't compile it yet): {message}"
                        )
                    }
                    Ran::Declined { code: None, .. } => {
                        "declined (compiler can't compile it yet)".to_string()
                    }
                    Ran::Trap(t) => format!("trap: {t}"),
                    Ran::BadArtifact(e) => format!("artifact did not build: {e}"),
                };
                println!("expect:   {}", trial.expect);
                println!("actual:   {actual}");
            }
            println!("verdict:  {verdict}");
            // BLIND-SPOT NOTE (breaker 2026-08-31): `--case` grades the diagnostic CODE + message + the
            // value/trap/host-calls/warns/live-objects outcome, but NOT the DIAGNOSTIC-QUALITY asserts
            // (`(fix …)`/`(no-fix)`/`(count …)`) — those need the KIND_DIAGNOSTICS sidecar wire, which this
            // in-process spot-check path does not capture (only the nix corpus-exec / `cdz-run --grade` does).
            // So a `--case` PASS on an error/warning case does NOT confirm its fix asserts — a real
            // fix-proposal regression can hide behind a green `--case` (breaker mis-triaged one this way).
            // The record stream drops the fix asserts too, so we cannot detect them precisely here; flag the
            // scope on any error/warning case (the only kinds that can carry diagnostic-quality asserts).
            let diag_quality_eligible = rec.trials.iter().any(|t| {
                let kind = t
                    .expect
                    .split_once(' ')
                    .map_or(t.expect.as_str(), |(k, _)| k);
                kind == "error" || kind == "warning"
            });
            if diag_quality_eligible {
                println!(
                    "note:     --case does NOT grade (fix …)/(no-fix)/(count …) diagnostic-quality asserts \
                     (no sidecar capture) — a PASS here does not confirm them; run the nix corpus-exec \
                     (`cargo xtask gate --check` / the per-case drv) to grade those."
                );
            }
            println!();
        }
    }
    if found == 0 {
        eprintln!("xtask gate --case: no case matched {needle:?}");
        std::process::exit(1);
    }
}

/// One graded case's verdict.
enum Grade {
    /// Ran and matched the recorded outcome.
    Pass,
    /// The compiler can't yet handle it (declined), or the expectation needs machinery not wired
    /// yet (error-code matching, traps) — not a disagreement, just not-yet.
    Todo,
    /// Ran to an outcome that disagrees with the record — the actionable frontier.
    Fail(String),
}

/// Grade one case: run EACH trial (the program is re-driven per trial's `(call …)`) and COMBINE. A
/// case's verdict is the combination of its trials' verdicts: `Fail` if ANY trial fails (the actionable
/// disagreement wins, tagged with which trial), else `Todo` if any trial is todo (the whole case is
/// only as "done" as its least-done trial — a partially-declining case is not a live guard), else
/// `Pass`. The common single-trial case grades exactly as before.
fn grade(tools: &Tools, store: &Option<PathBuf>, rec: &CorpusRecord, target: GateTarget) -> Grade {
    let rans: Vec<Ran> = rec
        .trials
        .iter()
        .enumerate()
        .map(|(i, t)| {
            // The heap-balance check is applied to the FIRST trial only — a single `(live-objects N)`
            // clause can't express per-trial counts (a multi-trial case re-drives the SAME program with
            // different args, which leak different amounts), and the authoritative nix grade
            // (`cdz-run --grade`) already keys the balance off the first runnable trial. Later trials still
            // grade their value/trap outcome; they just skip the balance (`Off`).
            let live_objects = if i == 0 {
                LiveObjectsCheck::from_record(rec.live_objects, rec.live_objects_known_leak)
            } else {
                LiveObjectsCheck::Off
            };
            run_program(
                tools,
                store,
                &rec.program,
                &rec.modules,
                &rec.peers,
                t.call.as_ref(),
                &rec.host_responses,
                &rec.host_calls,
                rec.wit_world.as_deref(),
                rec.component_name.as_deref(),
                live_objects,
                target,
            )
        })
        .collect();
    grade_ran(rec, &rans, target)
}

/// Combine per-trial outcomes into the case's verdict. `rans[i]` is the outcome of `rec.trials[i]`.
/// Shared by the tally path and the single-case debug view. `target` is threaded so a `(warns …)`
/// pin can be graded only on the backend that can OBSERVE compile warnings (see the warns arm below).
fn grade_ran(rec: &CorpusRecord, rans: &[Ran], target: GateTarget) -> Grade {
    let mut todo = false;
    for (trial, ran) in rec.trials.iter().zip(rans) {
        match grade_trial(&trial.expect, ran) {
            Grade::Pass => {}
            Grade::Todo => todo = true,
            // Tag a failing trial with its call so a multi-trial case points at the offending run.
            Grade::Fail(why) => {
                return Grade::Fail(match &trial.call {
                    Some(c) if !c.args.is_empty() => {
                        format!("[{} {}] {why}", c.export, c.args.join(" "))
                    }
                    _ => why,
                });
            }
        }
    }
    // HOST-CALL verification (E2h): if the case recorded a `(host-calls …)` sequence, the run's OBSERVED
    // host calls must match it EXACTLY (order included) — a dropped, extra, or reordered call is a Fail,
    // closing the false-pass hole where a unit-returning host op (`log.emit`) matches the return value
    // while its side-effecting call was silently dropped. Verified only when the case RAN to a value
    // (a decline/trap is graded above); the observed calls come from the run that produced the value.
    // Applied once per case (host cases are single-trial), against the FIRST value-producing trial.
    if !rec.host_calls.is_empty()
        && let Some(Ran::Value(_, observed, _)) = rans.iter().find(|r| matches!(r, Ran::Value(..)))
        && *observed != rec.host_calls
    {
        return Grade::Fail(format!(
            "host-call mismatch: expected [{}], observed [{}]",
            rec.host_calls.join(", "),
            observed.join(", ")
        ));
    }
    // WARNING pins (operator seq353 inc2): each `(warns CODE (message …)?)` is a PRESENCE check over the
    // compile warnings the run captured — some emitted warning must share the code AND (if pinned) contain
    // the message phrase (case-sensitive). ORTHOGONAL to the outcome; checked against the value-producing
    // trial (a warning is on a program that compiled). A decline/trap case carries no warnings to match.
    //
    // NON-WASM SKIP (target-aware): a `(warns …)` pin is graded ONLY on the wasm target. Every warning
    // code is emitted in `compile.rs` — the shared front-end/compile stage (CDZ0306 UnusedBinding,
    // CDZ0305 DeadTrap, CDZ0213 RedundantArm, CDZ0308 UnreachableBranch), NONE in a backend — so a
    // warning is TARGET-INDEPENDENT: it provably fires during `compile()` regardless of the emit target.
    // The wasm gate is therefore a SUFFICIENT WITNESS. The rust / rust-async run paths surface only a
    // `value <n>` verdict line and swallow the (non-fatal) compile stderr, so their `Ran::Value` carries
    // no warnings — an OBSERVABILITY gap in the harness, NOT a behavior difference. Skipping (not failing)
    // the warns check there asserts nothing false and misses nothing real; failing it would red the gate
    // on a warning that genuinely fired. (Contrast `(error CODE)`: a reject IS observable on all three
    // because it fails the compile.) Making warns observable on rust would need the rust run path to
    // surface compile stderr — a separate, larger increment, not needed here.
    if !rec.warns.is_empty()
        && target == GateTarget::Wasm
        && let Some(Ran::Value(_, _, emitted)) = rans.iter().find(|r| matches!(r, Ran::Value(..)))
    {
        for (code, message) in &rec.warns {
            let present = emitted.iter().any(|(c, m)| {
                c == code && message.as_ref().is_none_or(|phrase| m.contains(phrase))
            });
            if !present {
                return Grade::Fail(match message {
                    Some(p) => {
                        format!("expected a warning {code} containing {p:?}, emitted: {emitted:?}")
                    }
                    None => format!("expected a warning {code}, emitted: {emitted:?}"),
                });
            }
        }
    }
    if todo { Grade::Todo } else { Grade::Pass }
}

/// Compare ONE trial's run outcome against its recorded `expect` payload — the pure per-trial grading
/// logic. (A case combines these across its trials in `grade_ran`.)
fn grade_trial(expect: &str, ran: &Ran) -> Grade {
    // NO capability gate: a case is graded by what the compiler ACTUALLY does with it, never skipped
    // because it carries a `(needs …)` tag. The compiler DECLINES a construct it cannot yet compile
    // (decline-don't-miscompile — `reference-compiler.md` §Outcomes Are Ordered By Safety), and a
    // decline grades `Todo` below — so an unimplemented feature is already out of scope WITHOUT a gate.
    // Gating a whole feature set on `(needs)` hid a case that RUNS TO A WRONG VALUE (a miscompile) as a
    // benign skip; running every case surfaces that as a `Fail`, the honest signal. (`(needs …)` stays
    // in the corpus as documentation of what a case exercises; it no longer suppresses grading.)
    let (kind, payload) = expect.split_once(' ').unwrap_or((expect, ""));
    match kind {
        // `output (: <value> <Type>)`: the run must produce that value. A SCALAR crosses as a bare value
        // (`cdz-run` renders `42`), so it matches the value-form's value alone; a COMPOUND crosses via
        // the resource escape as the WHOLE `(: value type)` form (the host decodes the canonical bytes
        // and prints value AND type). Accept EITHER — the bare value (scalar) or the full form (compound)
        // — so both ABIs grade against the one recorded `(: value type)` outcome.
        "output" => {
            let expected_full = payload.trim().to_string();
            match ran {
                // STRUCTURAL value compare via the SINGLE-SOURCE shared canonical reader/printer
                // (`cdz_corpus_grade::canonical_output_value`, operator SLICE-1) — replaces the local
                // `expected_value` string-scan that diverged and caused fleet red #7273. Canon BOTH the
                // expected payload AND the run value, compare the canonical renders (so bare-vs-`(: v T)`
                // and rendering variance normalize away, subsuming the old bare/full dual-check). A parse
                // failure is a LOUD Fail — never a silent pass: a corpus authoring error on the expected
                // side, a compiler emit bug (decode-validity break) on the actual side. Mirrors the merged
                // `cdz_corpus_grade::grade_trial` Output arm exactly (verdict-identical).
                Ran::Value(v, _, _) => {
                    match (canonical_output_value(payload), canonical_output_value(v)) {
                        (Ok(want), Ok(got)) if want == got => Grade::Pass,
                        (Ok(want), Ok(got)) => Grade::Fail(format!(
                            "expected {expected_full} (canonical {want}), ran → {v} (canonical {got})"
                        )),
                        (Err(e), _) => Grade::Fail(format!(
                            "corpus expected-output {expected_full} did not parse as a canonical value: {e}"
                        )),
                        (_, Err(e)) => Grade::Fail(format!(
                            "ran value {v} did not parse as a canonical value (compiler emit bug): {e}"
                        )),
                    }
                }
                // A CODE-LESS decline whose message is an ICE signature (`is_ice_signature`) on a case that
                // should produce a VALUE is a compiler BUG → FAIL, never a hidden todo (operator ruling
                // 2026-08-27, refined with breaker). A coded decline, or a code-less HONEST capability decline
                // ("no machine representation"…), stays Todo — the ~60-false-positive guard.
                Ran::Declined {
                    code: None,
                    message,
                } if is_ice_signature(message) => Grade::Fail(format!(
                    "ICE (compiler bug) on a case expecting {expected_full}: {message}"
                )),
                Ran::Declined { .. } => Grade::Todo, // coded/honest decline — compiler can't compile it yet
                Ran::Trap(t) => Grade::Fail(format!("expected {expected_full}, trapped: {t}")),
                // A broken artifact for a case the corpus says yields a VALUE is the miscompile the
                // Rust-backend gate exists to catch — the backend emitted un-compilable source.
                Ran::BadArtifact(e) => Grade::Fail(format!(
                    "expected {expected_full}, artifact did not build: {e}"
                )),
            }
        }
        // `error CODE`: the corpus says this program is REJECTED with diagnostic `CODE`. Grade by what
        // the compiler DID (the same rule as `output`, applied to rejections):
        //  - rejected with the MATCHING code  → Pass (the check fired, correctly coded);
        //  - ran to a VALUE (accepted an ill-formed program) → Fail (the miscompile the check must catch —
        //    the honest signal, the analogue of a wrong `output` value);
        //  - rejected with a DIFFERENT code / a CODELESS decline / a trap → Todo.
        // A different code is NOT a Fail: the ill-formed program was still correctly REFUSED (no
        // miscompile — nothing ran), the code TAXONOMY merely differs (the check isn't built yet and a
        // name goes unbound → CDZ0101, or the compiler picks a defensibly-different code — CDZ0203 "type
        // mismatch" where the corpus reference says CDZ0201 "malformed"). Aligning those codes (compiler
        // or corpus) turns each Todo into a Pass; treating them as Fail would swamp the honest
        // accepted-ill-formed signal with taxonomy noise. Only running an ill-formed program is a Fail.
        "error" => {
            // `error CODE` or `error CODE (message "phrase")` — CODE exact-matches; the optional message
            // clause additionally requires the emitted diagnostic to CONTAIN the phrase (case-sensitive).
            let (want, msg_phrase) = split_message_clause(payload);
            match ran {
                Ran::Value(v, _, _) => Grade::Fail(format!("expected rejection {want}, ran → {v}")),
                Ran::Declined {
                    code: Some(got),
                    message,
                } if got == want => match msg_phrase {
                    // No message clause → CODE alone decides (unchanged behavior).
                    None => Grade::Pass,
                    // Message clause present → the emitted diagnostic must contain the pinned phrase.
                    Some(phrase) if message.contains(phrase) => Grade::Pass,
                    Some(phrase) => Grade::Fail(format!(
                        "rejected [{got}] but message did not contain {phrase:?} (got: {message:?})"
                    )),
                },
                // Rejected (a different code), a codeless decline, or a trap — refused, not miscompiled.
                Ran::Declined { .. } | Ran::Trap(_) => Grade::Todo,
                // A broken artifact cannot validate a rejection CODE (the front-end never rejected — the
                // BACK end failed to build a value it accepted), so it is not a clean signal here: Todo.
                Ran::BadArtifact(_) => Grade::Todo,
            }
        }
        // `trap <reason>`: the corpus says this program TRAPS at run time for the given reason. Grade by
        // what the compiler DID:
        //  - trapped, and the actual trap REASON matches the corpus reason → Pass;
        //  - trapped for a DIFFERENT reason (or an unclassifiable message) → Todo (a real trap fired, but we
        //    can't confirm it is the SAME one — treat conservatively, never a false Pass);
        //  - ran to a VALUE → Fail (a program the corpus says traps produced a value — a miscompile);
        //  - declined/rejected (compile-time refusal, e.g. a constant-folded overflow caught as CDZ0302) →
        //    Todo (refused, not miscompiled — the runtime trap isn't reached because the compiler rejects
        //    the ill-formed program first; aligning that is a separate design choice).
        // Trap-reason matching normalizes both sides to a canonical trap KIND (`classify` → `TrapCode`), so the
        // corpus's `divide by zero` matches wasmtime's `integer divide by zero`, `overflow` matches
        // `integer overflow`, `index out of bounds` matches `out of bounds memory access`, etc.
        "trap" => match ran {
            Ran::Value(v, _, _) => Grade::Fail(format!("expected a trap, ran → {v}")),
            // A broken artifact for a case that should TRAP is still a miscompile (the backend was asked
            // for a runnable artifact that traps and emitted un-compilable source instead).
            Ran::BadArtifact(e) => {
                Grade::Fail(format!("expected a trap, artifact did not build: {e}"))
            }
            // An ARTIFACT-ICE actual (compile-success-but-unloadable component) is a compiler bug, never the
            // expected trap → FAIL, before the kind comparison (breaker's B1; a value-expectation already
            // FAILs a trap, this closes the trap-expectation channel where it classified to no TrapCode → Todo).
            Ran::Trap(actual) if is_artifact_ice(actual) => Grade::Fail(format!(
                "expected a trap, but the compiled artifact failed to LOAD (an ICE — invalid component): {actual}"
            )),
            Ran::Trap(actual) => {
                // EXPECTED side: an explicit trap CODE id (`from_id`, preferred stable form) or a legacy
                // English reason (`classify`, back-compat). Compare by CODE to the actual runtime reason.
                let want = TrapCode::from_id(payload).or_else(|| classify(payload));
                match (want, classify(actual)) {
                    // Both classify AND agree → the expected trap fired.
                    (Some(w), Some(g)) if w == g => Grade::Pass,
                    // Both classify to KNOWN trap codes but DIFFER → a hard disagreement (miscompile or
                    // wrong-kind expectation), graded FAIL like a wrong output value. With semantic CDZ07xx
                    // codes a mismatched KIND between two traps is a real disagreement, not a hidden Todo
                    // (breaker's grading-gap catch). Mirror of `cdz_corpus_grade::grade_trial`.
                    (Some(w), Some(g)) => Grade::Fail(format!(
                        "expected trap {} ({payload}) but trapped {} ({actual}) — wrong trap kind",
                        w.code(),
                        g.code()
                    )),
                    // Reason doesn't classify (or expectation doesn't) — a real trap, but unconfirmed.
                    _ => Grade::Todo,
                }
            }
            // An ICE-signature code-less decline on a case that should TRAP is a compiler bug → FAIL; a
            // coded/honest decline stays Todo (mirror of the output arm + cdz_corpus_grade).
            Ran::Declined {
                code: None,
                message,
            } if is_ice_signature(message) => Grade::Fail(format!(
                "ICE (compiler bug) on a case expecting a trap: {message}"
            )),
            Ran::Declined { .. } => Grade::Todo,
        },
        // `declines`: the corpus says the compiler DECLINES to emit a component for this well-formed
        // program (a shape it does not yet realize — e.g. a type with no boundary representation), the
        // "decline rather than miscompile" outcome (reference-compiler.md §A Type Without A Boundary
        // Representation Declines At The Boundary; cross-component-interop.md §A Not-Yet-Supported
        // Cross-Component Shape Declines Rather Than Miscompiles). Grade by what the compiler DID:
        //  - declined (codeless OR coded — either is a refusal to emit) → Pass (the guard held);
        //  - ran to a VALUE → Fail (it EMITTED a component and produced a value — the miscompile this
        //    guard exists to catch, the honest analogue of a wrong `output` value);
        //  - trapped / broken artifact → Fail (a component WAS emitted — not the clean decline pinned).
        // The Pass side is DELIBERATELY wide (any `Declined`, coded or not): a case migrated from `todo`
        // to `declines` pins "this must not emit", not a specific diagnostic code (that is `error CODE`'s
        // job). Should the compiler later gain a coded rejection for the same shape, this still passes;
        // should it later EMIT the shape, the case flips to Fail and the corpus is updated to `output`.
        "declines" => {
            // `declines` (any refusal passes) or `declines (message "phrase")` (the decline's diagnostic
            // must additionally contain the pinned phrase, case-sensitive — pins WHY it declines, e.g. the
            // float-compare "IEEE partial order" redirect, without pinning a specific code).
            let (_, msg_phrase) = split_message_clause(payload);
            match ran {
                Ran::Declined { message, .. } => match msg_phrase {
                    None => Grade::Pass,
                    Some(phrase) if message.contains(phrase) => Grade::Pass,
                    Some(phrase) => Grade::Fail(format!(
                        "declined but message did not contain {phrase:?} (got: {message:?})"
                    )),
                },
                Ran::Value(v, _, _) => Grade::Fail(format!("expected a decline, ran → {v}")),
                Ran::Trap(t) => Grade::Fail(format!("expected a decline, trapped: {t}")),
                Ran::BadArtifact(e) => {
                    Grade::Fail(format!("expected a decline, artifact did not build: {e}"))
                }
            }
        }
        _ => Grade::Todo,
    }
}

/// The default corpus: every `spec/semantics/NN-*.sexp`, sorted for stable order. Corpus files follow
/// the `NN-feature` naming convention (a numeric prefix) — only digit-led stems are corpus files (never
/// an ordinary docs `.md` like `README.md`). The corpus is `.sexp`-only (the markdown-literate `.md`
/// twin feature was removed per operator direction — the s-expression source is the single form).
/// Parse a `--shard` spec `"I/N"` into 1-based `(i, n)`, validating `1 <= i <= n` and `n >= 1`.
fn parse_shard(spec: &str) -> Result<(usize, usize), String> {
    let (i, n) = spec
        .split_once('/')
        .ok_or_else(|| format!("expected `I/N` (e.g. `1/8`), got `{spec}`"))?;
    let i: usize = i
        .trim()
        .parse()
        .map_err(|_| format!("shard index `{i}` is not a number"))?;
    let n: usize = n
        .trim()
        .parse()
        .map_err(|_| format!("shard count `{n}` is not a number"))?;
    if n == 0 {
        return Err("shard count N must be >= 1".into());
    }
    if i == 0 || i > n {
        return Err(format!("shard index I must be in 1..={n} (got {i})"));
    }
    Ok((i, n))
}

// ============================================================================================
// gate baseline — a committed per-case verdict snapshot, so a REGRESSION (a case that used to pass
// and now doesn't) fails `gate --check` even while the pass/todo/fail totals drift.
// ============================================================================================

/// The committed baseline file for a target: `<repo>/spec/semantics/.gate-baseline` for the default
/// wasm gate, and a target-suffixed sibling (`.gate-baseline-rust`) for another backend — so each
/// backend has its OWN regression baseline and one does not clobber the other's.
fn baseline_path(paths: &Paths, target: GateTarget) -> PathBuf {
    let name = match target {
        GateTarget::Wasm => ".gate-baseline".to_string(),
        GateTarget::Rust => ".gate-baseline-rust".to_string(),
        GateTarget::RustAsync => ".gate-baseline-rust-async".to_string(),
        GateTarget::Cadenza => ".gate-baseline-cadenza".to_string(),
    };
    paths.repo.join("spec/semantics").join(name)
}

/// Write the current verdicts as the baseline: one `verdict\tdescription` line per case, sorted by
/// description so the file is stable and a diff is meaningful.
fn save_baseline(paths: &Paths, verdicts: &[(String, Verdict)], target: GateTarget) {
    // De-dupe by description before writing: a run's verdicts shouldn't contain dup descriptions, but
    // writing a canonical (sorted, unique-by-description) file means a `gate --save` also CLEANS UP any
    // duplicate lines a `merge=union` merge introduced into the committed baseline. Last verdict wins
    // per description (matches the map-load in `check_baseline`).
    let mut by_desc: std::collections::BTreeMap<String, Verdict> =
        std::collections::BTreeMap::new();
    for (d, v) in verdicts {
        by_desc.insert(d.clone(), *v);
    }
    let body = serialize_baseline(&by_desc);
    std::fs::write(baseline_path(paths, target), body).expect("write baseline");
}

/// Compare current verdicts to the baseline. Returns the process exit code: non-zero if any case
/// REGRESSED (baseline pass → now not pass) or a baseline case vanished. Newly-passing cases and
/// new cases are reported but do not fail the check.
/// `subset` (a `--shard` run) scopes the compare to the cases actually run: it flags REGRESSIONS among
/// them but does NOT treat baseline cases absent from this run as "vanished" (they belong to the other
/// shards). A full run (`subset = false`) keeps the vanished check, which catches a case silently dropped
/// from the corpus. Vanished-detection for a sharded nightly is left to the full-corpus `gate --save`
/// discipline (removing a case re-saves the baseline), so no shard needs it.
fn check_baseline(
    paths: &Paths,
    verdicts: &[(String, Verdict)],
    target: GateTarget,
    subset: bool,
) -> i32 {
    let path = baseline_path(paths, target);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "xtask gate --check: no baseline at {} (create it with `gate --save`)",
                path.display()
            );
            return 2;
        }
    };
    // Delegate to the canonical whole-pass baseline fold (xtask_support::compare_verdicts_baseline) — the
    // single source of truth v-corpus-harness blessed: the semantics `gate --check`, `gate-syntax --check`,
    // the rust/rust-async harvests, and the xtask-check-baseline leaf all grade through it. Verdict- and
    // exit-code-IDENTICAL to the former inline HashMap compare (same five invariants: pass→not-pass
    // regression / gained / vanished-on-full-run / failing gate-hole / tracked known-fail); the fold's
    // BTreeMap load additionally makes the reported lists deterministic (a strict improvement over the old
    // nondeterministic HashMap iteration order). The report wording below stays the semantics gate's own.
    let cmp = compare_verdicts_baseline(verdicts, &text, subset);

    if !cmp.conflict.is_empty() {
        eprintln!(
            "xtask gate --check: {} CONFLICTING duplicate case description(s) in {} — the same case \
             appears with DIFFERENT verdicts, so the map-keyed baseline silently masks one (last wins). \
             This is a real integrity error; regenerate with `cargo xtask gate --save` and check which \
             verdict is correct. Conflicting:",
            cmp.conflict.len(),
            path.display()
        );
        for d in &cmp.conflict {
            eprintln!("  •  {d}");
        }
        return 3;
    }
    if cmp.benign_dups > 0 {
        // Benign same-verdict dups (a `merge=union` artifact) are HARMLESS — the fold deduped them in
        // memory for the compare. `--check` is READ-ONLY (rewriting here would dirty the worktree + block
        // every agent's `fleet sync`); dedup-on-disk is `gate --save`'s job. (concierge-greenlit fix (a).)
        eprintln!(
            "xtask gate --check: {} benign (same-verdict) duplicate line(s) in {} — a merge=union \
             artifact, harmless (deduped in memory for the compare). Run `cargo xtask gate --save` to \
             rewrite the file clean; `--check` leaves it untouched.",
            cmp.benign_dups,
            path.display()
        );
    }

    if !cmp.gained.is_empty() {
        println!("\nnewly passing ({}):", cmp.gained.len());
        for g in &cmp.gained {
            println!("  +  {g}");
        }
    }
    if !cmp.regressed.is_empty() {
        println!("\nREGRESSED ({}):", cmp.regressed.len());
        for r in &cmp.regressed {
            println!("  -  {r}");
        }
        // KNOWN IN-PROCESS BLIND SPOT (concierge-greenlit interim, 2026-09-01; removed by the gate-delete
        // that re-points `--check` off this in-process path onto the per-case nix `corpus-<chapter>` execs).
        // The in-process gate does NOT capture the KIND_DIAGNOSTICS sidecar (only the nix per-case exec,
        // which feeds `--diagnostics`, does), so a case that COMPILES cleanly but pins a diagnostic-QUALITY
        // assert — `(warns …)` / `(warning …)` / `(fix …)` / `(count …)` — cannot confirm that assert here
        // and downgrades to `todo`, surfacing as a spurious `pass → todo` while it is GREEN on the
        // authoritative nix bar. This footer (fired only when some regression is `pass → todo`) tells a
        // lander to verify on nix before treating such a case as a real regression. A `pass → fail` is a
        // real miscompile — NOT this blind spot. (Root-caused after it false-red-blocked 4 verticals.)
        if cmp.regressed.iter().any(|r| r.contains("→ todo")) {
            println!(
                "\nNOTE: a `pass → todo` above on a case pinning a (warns)/(warning)/(fix)/(count) \
                 diagnostic-quality assert is very likely the KNOWN in-process diagnostic-capture blind \
                 spot — the in-process gate cannot capture the KIND_DIAGNOSTICS sidecar, so such a case \
                 (which compiles fine) downgrades to `todo` here while it is GREEN on the authoritative \
                 nix bar. Verify with `nix build .#checks.<sys>.corpus-<chapter>` (diagnostics-fed, \
                 baseline-enforcing) before treating it as a real regression; if green there, an --admin \
                 bypass is correct. (A `pass → fail` is a real miscompile, not this.)"
            );
        }
    }
    if !cmp.vanished.is_empty() {
        println!("\nvanished from the corpus ({}):", cmp.vanished.len());
        for v in &cmp.vanished {
            println!("  ?  {v}");
        }
    }
    if !cmp.failing.is_empty() {
        println!(
            "\nFAILING — a fail not caught by the pass-regression check ({}):",
            cmp.failing.len()
        );
        for f in &cmp.failing {
            println!("  x  {f}");
        }
    }
    if !cmp.tracked_fail.is_empty() {
        // Visible but NOT gate-redding: git-committed known-wrong pins (a deferred-fix compiler bug).
        println!(
            "\nKNOWN-FAIL — tracked known-wrong (baseline `fail`), not a gate failure ({}):",
            cmp.tracked_fail.len()
        );
        for f in &cmp.tracked_fail {
            println!("  ⊗  {f}");
        }
    }

    let code = cmp.exit_code();
    if code == 0 {
        println!(
            "\ngate --check: OK (no regressions vs baseline; {} newly passing)",
            cmp.gained.len()
        );
    } else {
        println!(
            "\ngate --check: FAIL ({} regressed, {} vanished, {} failing)",
            cmp.regressed.len(),
            cmp.vanished.len(),
            cmp.failing.len()
        );
    }
    code
}

/// A nix REMOTE-BUILDER / DAEMON transient in a gate's output — NOT a real test/clippy/compile failure.
/// In nix protocol the local multi-user daemon is the "remote" store, so the daemon/builder-hiccup shapes
/// are one false-RED family: a build-result the caller can't interpret ("Invalid BuildResult status from
/// remote"), a remote build failure, or a daemon-connection reset ("cannot open connection to remote
/// store"). Used by fleet's `gate_local_hold_advisory` to flag a re-run rather than a regression. Pure so
/// the match rule is unit-tested; kept narrow so a real error merely mentioning "remote" doesn't trip it.
fn fast_gate_output_is_remote_transient(output: &str) -> bool {
    output.contains("Invalid BuildResult status from remote")
        || output.contains("error: build failure on remote")
        || output.contains("cannot build on remote")
        || output.contains("cannot open connection to remote store")
}

/// A gate sub-check builder that was KILLED under load — NOT a real test/clippy/compile failure, and NOT a
/// nix daemon/remote transient either. Under sustained check-lease contention a sub-check derivation can be
/// SIGKILLed (the OOM-killer, a reaper, the loop/harness command-timeout) or SIGTERMed; nix reports that as
/// the builder "failed with exit code 137/143" (128+SIGKILL / 128+SIGTERM) or "failed due to signal 9/15".
/// Its output carries NO remote-transient signature, so [`fast_gate_output_is_remote_transient`] misses it
/// and `gate_local_hold_advisory` would wrongly label it a REAL regression — the false-HOLD class: an agent
/// then routes/retries a phantom failure and can get stuck (v-rcdzc-test-shrink hit this under the land-block
/// contention). This distinguishes the contention-kill shape so the advisory says RE-RUN (when the box is
/// quieter) rather than ROUTE. ADVISORY ONLY — it never changes the RED verdict, so a genuine failure that
/// merely LOOKS killed is at worst re-run, never merged. Pure so the match rule is unit-tested. NARROW BY
/// DESIGN: only the KILL signals (9 SIGKILL / 15 SIGTERM = external terminations from a reaper/OOM/timeout)
/// and their canonical exit codes — NOT a crash signal (SIGSEGV 11 / SIGABRT 6), which IS a real failure to
/// route, and not a stray "137" in a diff (the phrase is anchored to nix's builder-failure wording).
fn fast_gate_output_is_contention_kill(output: &str) -> bool {
    output.contains("failed with exit code 137") // 128 + 9 (SIGKILL: OOM-killer / reaper / hard timeout)
        || output.contains("failed with exit code 143") // 128 + 15 (SIGTERM)
        || output.contains("failed due to signal 9")
        || output.contains("failed due to signal 15")
}

// ============================================================================================
// check — the omnibus health check.
// ============================================================================================

/// Run the whole health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
/// the wasm runtime build, the excluded-but-host-checkable `cdz-wasm` crate (fmt/build/test/clippy on
/// its own manifest), and the behavior gate. Each step's full output is CAPTURED to a log file
/// rather than flooding the console; the console shows one ✓ per passing step. The first failing step
/// prints the whole captured log
/// (so an agent reads it in place instead of re-running with `| tail`) and its path, then exits.
/// Run `bash -n <file>` (parse-only, never executes) on each script; return `"<file>: <first stderr
/// line>"` for each that FAILS to parse. Fail-SOFT: if `bash` itself can't be launched (absent), returns
/// empty so the caller SKIPS — we never report a parse failure we couldn't actually test. Pure aside from
/// shelling out; unit-tested with a temp good/bad pair.
fn sh_syntax_errors(scripts: &[PathBuf]) -> Vec<String> {
    let mut bad = Vec::new();
    for s in scripts {
        let out = match std::process::Command::new("bash").arg("-n").arg(s).output() {
            Ok(o) => o,
            Err(_) => return Vec::new(), // bash absent → fail-soft skip of the whole lint
        };
        if !out.status.success() {
            bad.push(format!("{}: {}", s.display(), first_line(&out.stderr)));
        }
    }
    bad
}

/// Syntax-check the tracked fleet shell scripts (`fleet/*.sh`) with `bash -n` — a cheap gate guarding the
/// ONLY shell the fleet ships: `window.sh` (the launcher EVERY agent window runs) plus the disk-hygiene
/// scripts (`prune-stale-targets.sh`, `prune-tmp-inodes.sh`) the concierge cron calls. A syntax error in
/// any would break agent launch / disk hygiene FLEET-WIDE, and nothing else gates them (they are shell,
/// not part of the Rust build). Fail-soft when `bash` is absent (mirrors `duvet-check`). Cheap — a parse
/// of a few tiny files, no build.
fn fleet_scripts_syntax_lint(paths: &Paths) -> Result<(), String> {
    let dir = paths.repo.join("fleet");
    let mut scripts: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sh"))
            .collect(),
        Err(_) => return Ok(()), // no fleet/ dir here → nothing to check
    };
    scripts.sort();
    if scripts.is_empty() {
        return Ok(());
    }
    let bad = sh_syntax_errors(&scripts);
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} fleet shell script(s) fail `bash -n` (a syntax error would break agent launch / disk \
             hygiene fleet-wide):\n    {}",
            bad.len(),
            bad.join("\n    ")
        ))
    }
}

fn check(paths: &Paths, profile: &str) {
    // Acquire a fleet-wide check-lease FIRST (operator-mandated concurrency cap). This blocks until a
    // slot is free under the cap, and a vertical check yields to pr-sync's PRIORITY lease so the main
    // merge queue is never held up by one-off agent checks. pr-sync's gate sets `CDZ_CHECK_PRIORITY=1`
    // to take a priority (uncapped, never-waiting) slot. Held for the whole check; released on return
    // (RAII drop). Fail-open, so a lease-dir hiccup never blocks a developer's gate. `_lease` must
    // outlive the check body — do NOT drop it early (a `let _ =` would release the slot immediately).
    let priority = std::env::var("CDZ_CHECK_PRIORITY").is_ok_and(|v| v == "1" || v == "true");
    let _lease = fleet::acquire_check_lease(&paths.repo, priority);

    let mut log = Log::create(paths, "check");
    println!("check: logging to {}", log.path.display());

    // Each step runs its command with stdout+stderr appended to the log. Native workspace first:
    // formatting, build, test, then clippy. `fmt --check` and clippy `-D warnings` are HARD gates —
    // the workspace is cargo-fmt-clean and clippy-clean, and this keeps it that way (a lint or a
    // stray format is a failing step, with the offending diff/lint captured in the log to read).
    // clippy MUST pass `--all-targets` to match CI (checks.yml runs `clippy --workspace --all-targets
    // -- -D warnings`): without it, clippy skips test/bench/example targets, so a `#[cfg(test)]`-only
    // lint passes this local gate + pr-sync's re-gate yet RED-lights CI (this happened — a
    // `unnecessary_get_then_check` in an lsp test slipped through to a hand-fix on trunk). Keep this in
    // lockstep with CI's clippy invocation.
    let repo = &paths.repo;
    log.step("fmt", "cargo fmt --all --check", repo);
    log.step("build", "cargo build --workspace", repo);
    // RUST_MIN_STACK=64M on the workspace test phase, matching the storeless-rerun/miri/bench steps.
    // WHY: a deep-recursion test that runs on libtest's own worker thread (default ~2MB stack) — not one
    // wrapped in rcdzc's explicit-stack `host::run_with_compiler_stack` worker — SIGABRTs "stack
    // overflow" under fleet build load, a NONDETERMINISTIC false-red that reds `check` for every vertical
    // + pr-sync's re-gate. This whack-a-mole class kept recurring (v-diagnostics deep-arith, v-syntax
    // deep-arena, v-runtime sum-payload) as per-test spawn-wrapper point-fixes; exporting the same 64MB
    // floor the sibling test steps already use immunizes the WHOLE workspace at once. Harmless for tests
    // that don't need it (a larger min stack only raises the floor). `RUST_MIN_STACK` governs threads
    // spawned WITHOUT an explicit stack_size, so it lifts libtest's harness threads but is (correctly)
    // a no-op for the compile worker, which sets its own stack.
    // CDZ_RUN_TIMEOUT_SECS=300 on the test phase (default is 30). cdz-run arms a WALL-CLOCK epoch deadline
    // (a background thread bumps the wasmtime engine epoch every 100ms regardless of the run's own CPU —
    // see cdz-run/src/lib.rs arm_epoch_ticker), so under heavy parallel `cargo test --workspace` (dozens of
    // concurrent wasmtime instances + build/link contention) a run that is MILLISECONDS of CPU can be
    // starved off-core past the 30s wall-clock bound → its store trips `wasm trap: interrupt` (sometimes
    // SIGABRT) even though the program is correct and terminating. This is a recurring, load-dependent
    // false-red (rcdzc-lib run_heap_value tests: pass in isolation / --test-threads=1, fail only under the
    // full parallel run — v-property-testing root-caused it 2026-07-21 after it flaked several MRs). Lifting
    // the bound to 300s for the test phase lets a starved-but-correct run finish while still catching a
    // GENUINE runaway loop (a real infinite loop blows 300s of wall-clock too). Harness-only (this env
    // scopes to the `test` step's child); production/CI cdz-run keeps the 30s default. Same shape as the
    // RUST_MIN_STACK floor added here for the sibling deep-recursion false-red class.
    log.step_env(
        "test",
        "cargo test --workspace",
        repo,
        &[
            ("RUST_MIN_STACK", "67108864"),
            ("CDZ_RUN_TIMEOUT_SECS", "300"),
        ],
    );
    log.step(
        "clippy",
        "cargo clippy --workspace --all-targets -- -D warnings",
        repo,
    );

    // The generated runtime-ABI table (`runtime_abi.rs`) MUST stay current with the runtime WIT — a
    // forgotten `cargo xtask codegen` after a WIT change would silently drift the compiler's import
    // ABI from the runtime. `codegen --check` regenerates in memory and fails if the committed file is
    // stale, so this is a HARD GATE (like `fmt --check`) — nobody has to remember to regenerate.
    let xtask = std::env::current_exe().expect("current exe");
    let xtask = xtask.to_string_lossy().to_string();
    log.step("codegen", &format!("{xtask} codegen --check"), repo);

    // The wasm runtime is EXCLUDED from the native workspace, so a plain `cargo build` skips it — a
    // silent gap the check closes by building it explicitly for its target.
    let rt = paths.seed.join("crates/cdz-runtime");
    log.step_env(
        "wasm-runtime",
        "cargo build --release --target wasm32-unknown-unknown",
        &rt,
        // The runtime builds core/alloc/std from source (deterministic panic=immediate-abort); its
        // build-std is a -Z feature, enabled on the stable pin via RUSTC_BOOTSTRAP.
        &[("RUSTC_BOOTSTRAP", "1")],
    );

    // cdz-wasm (the browser guide's wasm-bindgen wrapper) is ALSO excluded from the native workspace
    // (its own `[workspace]`, so a native `cargo build` never compiles wasm-bindgen). That left it
    // OUTSIDE the gate — a change to its diagnostic/fix marshaling could break the guide with the check
    // still green. It is `crate-type = ["cdylib", "rlib"]`, so it fmt/clippy/build/test-checks on the
    // HOST target against its own manifest (no wasm32 / `wasm-pack` needed) — close the gap here, the
    // same way the wasm runtime's exclusion is closed above. `-D warnings` keeps it clippy-clean too.
    // Scope every step to `-p cdz-wasm`: cdz-wasm's path-deps reach the native workspace crates, so a
    // bare `--all`/`--workspace` here would re-check THEM (and trip on the native workspace's own
    // pre-existing rustfmt drift). `-p cdz-wasm` checks only cdz-wasm's own package.
    let wasm = paths.seed.join("crates/cdz-wasm");
    log.step("cdz-wasm-fmt", "cargo fmt -p cdz-wasm --check", &wasm);
    log.step("cdz-wasm-build", "cargo build -p cdz-wasm", &wasm);
    log.step("cdz-wasm-test", "cargo test -p cdz-wasm", &wasm);
    log.step(
        "cdz-wasm-clippy",
        // `--all-targets` for the same reason as the workspace clippy above: catch `#[cfg(test)]`-only
        // lints locally. cdz-wasm has no dedicated CI clippy job (it's gated only here), so this step
        // is its sole clippy coverage — all the more reason to lint its test targets too.
        "cargo clippy -p cdz-wasm --all-targets -- -D warnings",
        &wasm,
    );

    // The behavior gate — invoke this same xtask binary. Use `gate --check` (vs the baseline) when a
    // baseline exists, so `check` asks "did anything REGRESS?" rather than "are there any known
    // gaps?" — a green check means the library is healthy AND the compiler didn't backslide. With no
    // baseline, fall back to a plain `gate`. The gate's own summary is short, so let it print to the
    // console (it is the useful signal) while its verbose build noise still lands in the log.
    // The omnibus `check` runs the default (WASM) gate; the Rust-backend gate is a separate opt-in
    // (`gate --target rust --check`) with its own baseline, so it does not gate `check` (it needs
    // `rustc` per case and is slower).
    let gate_cmd = if baseline_path(paths, GateTarget::Wasm).exists() {
        format!("{xtask} --profile {profile} gate --check")
    } else {
        format!("{xtask} --profile {profile} gate")
    };
    log.step_show("gate", &gate_cmd, repo);

    // OPTIMIZATION-LEVEL-EQUIVALENCE gate — a HARD BLOCKING merge gate (operator directive 2026-07-17): a
    // program that produces a DIFFERENT observable outcome across optimization levels (O0..O3) is an
    // unsound optimization, and must RED the gate + reject the MR, not merely be reported. `gate
    // --opt-sweep` runs every corpus program at every level and hard-fails (exit non-zero) on any
    // cross-level divergence. Because `check` is exactly what pr-sync re-gates with, adding it here makes
    // it blocking on merge across ALL levels in one place. With the `PassManager` pipeline currently empty
    // every level runs identically, so this is green today and stands guard over every future Core pass
    // (an unsound pass caught before it lands — the §9 both-backend-miscompile guard). This blocking step
    // sweeps the WASM backend; the sweep ALSO honors `--target rust`/`rust-async` (each level threaded
    // through the rustc pipeline) for on-demand both-backend verification, but — like the rust behavior
    // gate itself — that is opt-in and NOT part of blocking `check` (rustc-per-case × 4 levels is too slow
    // to run on every dev check). ~parallelized so the full wasm corpus completes in a couple min.
    log.step(
        "opt-sweep",
        &format!("{xtask} --profile {profile} gate --opt-sweep"),
        repo,
    );

    // RC-LEAK gate — a CORE SUBSET of the `..._leaves_no_live_objects` probe family (Perceus/FBIP reclaim
    // discipline). These compose a program with the DEBUG-COUNTERS runtime and assert the runtime's
    // `live-objects` counter nets to the expected count after a round-trip — a DIRECT rc-invariant witness
    // that catches a reclaim leak (a missing/off-by-one drop) which the value + drop-import tests CANNOT see
    // (the value is correct; only the heap-cell count is wrong). They are `#[ignore]`d because they need the
    // debug-counters runtime in the store, so a plain `cargo test` skips them and NO gate/CI ran them — a
    // real coverage gap (a leak regression was invisible to `check`). Run a CORE-4 here (concierge ruling
    // 2026-07-18 (c): the pr-sync hot path is a documented contention point, so the FULL 12-probe family
    // (~60s) runs in the NIGHTLY CI job, and `check` runs only the cheapest core invariants ~20s): the
    // dup/drop BASELINE (`perceus_balance`), a BORROWING-OP reclaim (`runtime_value_eq`), an OWNED-SHELL
    // reclaim (`option_expect_…`), and an OWNED-TEMP producer (`owned_temporary_list_producers`) — the
    // classes that let real leaks slip past value gates (the SumExpect/MatchSum leaks). Each probe SKIPS
    // GRACEFULLY (prints + returns) when the debug store is absent, so a dev `check` without `cargo xtask
    // build` is not a hard fail; but pr-sync rebuilds the store per batch, so in the integrator's flow (the
    // one that guards trunk) this is a HARD leak-regression gate. `--test-threads=1` keeps the shared global
    // live-objects counter deterministic. NOTE: a SKIP is logged (the probes' own eprintln) so a silent
    // no-gate is visible.
    // RUST_MIN_STACK=64M for consistency with every other rcdzc `--lib` / workspace test invocation
    // (storeless-rerun, `cargo test --workspace`, miri, bench). These CORE-4 probes are shallow
    // rc-invariant witnesses, not deep-recursion tests, so today they don't need it — but this runs the
    // deep-recursion-prone rcdzc lib test binary, so a future probe added to this list is protected by
    // the floor rather than becoming the next whack-a-mole overflow. Uniform floor = one less way to
    // regress. (No-op for the compile worker, which sets its own stack_size.)
    log.step_env(
        "rc-leak-probes",
        "cargo test -p rcdzc --lib -- --ignored --test-threads=1 \
         perceus_balance_leaves_no_live_objects \
         runtime_value_eq_leaves_no_live_objects \
         option_expect_over_an_owned_some_shell_leaves_no_live_objects \
         owned_temporary_list_producers_leave_no_live_objects",
        repo,
        &[("RUST_MIN_STACK", "67108864")],
    );

    // The Cadenza-SOURCE @test suites — libraries written IN Cadenza (the CAD `Solid` model library and
    // the self-hosted compiler-ml port), run via `cdz test` (compile a separate wasm test component from
    // each `@test` def, run it under wasmtime, PASS if it returns / FAIL if it traps). These are NOT
    // corpus cases, so the behavior `gate` above does not cover them; without this step a change could
    // break a Cadenza library and `check` would stay green. `cdz test` shells to a sibling `cdz-run`
    // (resolving the value-heap runtime by content address from the store the wasm-runtime step above
    // populated) — the `build` step already produced both `cdz` and `cdz-run` under `target/debug`
    // (workspace members), so this step only runs them. A directory arg runs that dir's `Project.cdz`
    // suite. CI runs the SAME suites as its own `cad-tests` job (checks.yml); this closes the local /
    // pr-sync-re-gate gap so the omnibus `check` covers them too.
    let subdir = if profile == "dev" { "debug" } else { profile };
    let cdz = paths
        .repo
        .join("target")
        .join(subdir)
        .join("cdz")
        .display()
        .to_string();
    for suite in [
        "implementation/cad",
        "implementation/compiler-ml",
        "implementation/iterators",
        "implementation/choreography",
        "implementation/music",
        "implementation/des",
    ] {
        let name = format!("cdz-test {suite}");
        let cmd = format!("{cdz} test {suite}");
        // The compiler-ml sweep (~25-30min: ~73 cases each compiling a Cadenza program through the ML
        // compiler AND running it under wasmtime) is the dominant per-batch gate cost, and pr-sync
        // re-runs the whole `check` several times per batch (store-rebuild / re-plan / retry) even when
        // nothing changed. Content-cache its GREEN verdict keyed on (cdz binary ‖ compiler-ml tree) so
        // an unchanged (compiler, corpus) skips the re-run; any real change flips the key → full sweep.
        // The other two suites are quick, so run them plainly.
        // All suites get a generous suite-level wall-clock cap so a true HANG fails LOUD + NAMED instead
        // of silently wedging the gate (the "env kill" that looked like a signal was cf-corpus-all-pass
        // hanging with no output). compiler-ml additionally content-caches its green verdict.
        if suite == "implementation/compiler-ml" {
            // Run the heavy compiler-ml sweep ONE FILE AT A TIME under a tight per-file cap so a single
            // runaway compile (a def that blows up the compiler and never exits) fails LOUD + NAMED at
            // the per-file cap instead of burning the whole generous 45min suite budget and freezing the
            // backlog — the concierge's runaway-compile gate-unblock ask (2026-07-20). Same green cache
            // key, so an unchanged (compiler, corpus) still skips the whole sweep.
            let cache = CachedStep::new(paths, &name, Path::new(&cdz), &paths.repo.join(suite));
            log.step_cached_per_file(
                &name,
                &cdz,
                &paths.repo.join(suite),
                repo,
                cache.as_ref(),
                suite_timeout_for(suite),
            );
        } else {
            log.step_timed(&name, &cmd, repo, suite_timeout_for(suite));
        }
    }

    // STORELESS-rerun gate — the recurring local-green/CI-red class: a cdz/rcdzc test drives the
    // value-heap runtime, PASSES pr-sync's store-having gate (the store is built per-batch), but FAILS
    // CI's bare `test` job (`cargo test --workspace` with NO store) when its store-skip guard is missing
    // or wrong — the author forgot the `store_present()`/`find_runtime_wasm→None` skip. This step
    // reproduces the storeless CI condition LOCALLY by pointing `CADENZA_STORE` at an EMPTY temp dir and
    // re-running exactly the two crates every incident came from (rcdzc `--lib` + the cdz integration
    // tests): a correctly-guarded test SKIPS (its guard now reports the store absent — the guards +
    // resolvers honor `CADENZA_STORE` uniformly), so the rerun stays GREEN; a missing/wrong guard RUNS,
    // hits "no runtime in the store", and FAILS here — catching the CI-red BEFORE the MR lands rather
    // than as an integrator fix-forward. Concierge ruling (2026-07-19): the surgical, contention-aware
    // fix vs a full second `cargo test --workspace`.
    //
    // CONTENT-CACHED (2026-07-19): this re-runs the full `rcdzc --lib` (2176 tests) storeless, ~230s on a
    // loaded host — too costly to pay EVERY batch when a store-guard can only regress if the rcdzc TEST
    // sources change. Key the cached verdict on `rcdzc/src` (the rcdzc `--lib` unit tests). rcdzc's tests
    // are `#[cfg(test)]`, so they are NOT compiled into the release `cdz` binary — keying on the binary
    // alone would MISS an edit to `rcdzc/src/tests.rs` (the file holding the store-guarded
    // `run_heap_value`/`find_runtime_wasm` tests) and wrongly skip the rerun though the guarded set changed
    // (PR#648 / Copilot catch). So the source tree is the load-bearing input. (The `cdz/tests` tree +
    // `cdz` binary were dropped from the key 2026-08-09 alongside removing the cdz storeless entries — the
    // cdz CLI test suite was deleted (`5f14b9c40`), so `cdz/tests` no longer exists; keying on a missing
    // path made `new_multi` return None → the cache silently DISABLED, paying the ~230s sweep every batch.)
    // rcdzc/src changing flips the key → full storeless re-run; unchanged → skip the sweep. Fail-open (no
    // cache handle -> always run), and only a just-observed GREEN is ever recorded (a false green is
    // impossible).
    let storeless_cache = CachedStep::new_multi(
        paths,
        "storeless-rerun",
        &paths.seed.join("crates/rcdzc/src/lib.rs"),
        &[&paths.seed.join("crates/rcdzc/src")],
    );
    if let Some(cache) = &storeless_cache
        && cache.is_green()
    {
        println!(
            "  ✓ storeless-rerun (cached green @ {} — rcdzc/src unchanged)",
            cache.short_key()
        );
    } else {
        log.step_native("storeless-rerun", || storeless_rerun(paths));
        if let Some(cache) = &storeless_cache {
            cache.record_green();
        }
    }

    // Citation-coverage regression gate: fail if a `//=` / `//#` duvet citation was deleted/stranded
    // (live cited < the committed floor). Skips only when `duvet` isn't installed; a present-but-
    // erroring duvet (a stranded citation) FAILS loudly. The `duvet-check` command was decomposed into
    // the standalone `xtask-duvet-check` crate + `apps.duvet-check` (v-xtask-decompose), so `xtask
    // duvet-check` no longer has a built-in arm — it routes through the all-nix compat bridge
    // (`Cmd::External`) to `nix run <worktree>#duvet-check`, running the cached crane bin against this
    // worktree (CDZ_REPO_ROOT). No `--profile` to thread (the nix app is its own build).
    log.step_show("duvet-check", &format!("{xtask} duvet-check"), repo);

    // Corpus-hygiene lint: the `(needs …)` capability tag is RETIRED (the grade mechanism no longer
    // early-returns on it @d572403 — decline is the sole "todo" signal; see
    // `issues/DIRECTIVE-retire-needs-tag.md`). The clause is inert but concurrent corpus work keeps
    // re-introducing it (the parser's clause loop has a catch-all, so a stray `(needs …)` is silently
    // ignored, not rejected — removing the parser arm would NOT stop re-introduction). This lint is the
    // durable fix: it FAILS `check` on any `(needs …)` clause in `spec/semantics/*.sexp`, so a
    // re-introduced tag is a hard, self-explanatory error rather than silent rot.
    log.step_native("needs-free", || needs_free_lint(paths));

    // Baseline title-set agreement: the three gate baselines (`.gate-baseline`, `-rust`,
    // `-rust-async`) must cover the SAME set of case descriptions — the pass/todo VERDICTS legitimately
    // differ per backend (a case the rust backend declines is todo there, pass on wasm), but the SET of
    // cases is the corpus and is backend-independent. A corpus MR that runs the default `gate --save`
    // (wasm only) + lands leaves the rust/async baselines missing the new/renamed titles, so
    // `gate --check --target rust` goes red on clean trunk and stays that way silently (this omnibus
    // `check` gates only the WASM baseline). This lint catches that title-set divergence AT CHECK TIME
    // — cheap (it reads the three baseline files, no rust rebuild) — so the fix is a `gate --save
    // --target rust`+`rust-async` heal, caught now rather than ticks later.
    log.step_native("baseline-titles-agree", || {
        baseline_titles_agree_lint(paths)
    });

    // Within-file duplicate-title lint (companion to the cross-file agree-lint above): a baseline with the
    // SAME case-title on 2+ lines — typically a `gate --save`/hand-append leaving a stale extra verdict —
    // hard-errors `gate --check` when the verdicts conflict (pass+todo), reddening that target FLEET-WIDE.
    // The agree-lint compares title SETS so it can't see a within-file dup; this catches it at MR time.
    // Cheap (reads the three baseline files, no rebuild), like the agree-lint.
    log.step_native("baseline-no-dup-titles", || {
        baseline_no_dup_titles_lint(paths)
    });
    // Fleet shell-script syntax gate (v-fleet-tooling territory): `bash -n` the tracked `fleet/*.sh` —
    // `window.sh` (the launcher EVERY agent runs) + the disk-hygiene scripts. A syntax error there breaks
    // agent launch / disk hygiene fleet-wide and nothing else gates them (they aren't in the Rust build).
    // Cheap (parse a few tiny files) + fail-soft when `bash` is absent.
    log.step_native("fleet-scripts-syntax", || fleet_scripts_syntax_lint(paths));
    // Emoji-ban (operator directive 2026-08-07): FAIL the check on any emoji/pictographic/dingbat char in
    // an `implementation/**/*.rs` source COMMENT. Cheap (a text walk, no rebuild) + comment-scoped so it
    // never touches functional emoji in string/char literals (Unicode test strings, output markers) or the
    // legitimate technical typography (em-dash/arrows/math) it deliberately allows. See `emoji_free_lint`.
    log.step_native("emoji-free", || xtask_support::emoji_free_lint(&paths.repo));
    // File-size lint (operator directive seq-274): FAIL on any `implementation/**/*.rs` over 512 KiB —
    // GitHub stops syntax-highlighting above ~512 KB, so an oversized source file is un-highlighted +
    // hard to review. Cheap (a stat walk, no rebuild). A grandfather allowlist carries the files already
    // over the limit at adoption (each pending a split); the lint blocks NEW oversized files and shrinks
    // as the allowlisted ones are split. See `xtask_support::file_size_lint`.
    log.step_native("file-size", || xtask_support::file_size_lint(&paths.repo));
    // Cross-crate #[path] source-include lint (operator directive seq-275): FAIL on any `#[path = "…"]`
    // whose target resolves OUTSIDE the including crate's own src/ (into a sibling crate) — the source-share
    // that breaks crates.io publishability. Same-crate `#[path]` is fine. A grandfather allowlist carries
    // the current 4 cross-crate includes (cdz-runtime->cadenza-ast x3 under seq-273; cdz-num->cdz-runtime
    // bigint.rs) and shrinks as they convert to deps. See `xtask_support::cross_crate_path_include_lint`.
    log.step_native("cross-crate-path-include", || {
        xtask_support::cross_crate_path_include_lint(&paths.repo)
    });
    // fmt-ENFORCEMENT (operator directive seq-282): every canonical-domain `.cdz` SOURCE must be canonically
    // formatted (`cdz fmt --check`) — "the formatter dictates style, enforced". ENABLED 2026-08-30 after
    // v-syntax's fmt-all landed (#6317 compiler-ml + #6319 cad/music/des/iterators/choreography): all 6 domain
    // `src/` dirs are `cdz fmt --check`-canonical (exit 0, idempotent, AST/comment-neutral per seq-285). SCOPE
    // = concierge decision (A): the 6 canonical domain project `src/` dirs ONLY. It deliberately EXCLUDES
    // (B) cdz-platform contracts/guests (v-platform's zone — tracked follow-up, coordinate with them), the
    // Project.cdz manifests + rcdzc `.bin` (raw whole-`implementation` recursion noise), and (C) spec/semantics
    // `.sexp` (blocked on an s-expr-reader trailing-input parse gap — a separate reader-fix; the operator did
    // ask for `.sexp`, so it stays a tracked follow-up, but it cannot enforce until the reader parses them).
    // Expand this scope as (B)/(C) land. The AUTHORITATIVE fleet-wide enforcement is v-nix's nix `cdzFmtCheck`
    // (gate-local runs nix checks, not `cargo xtask check`); this is the local-`cargo xtask check` companion
    // (mirrors how the emoji lint lives in BOTH check() and the nix check) — the nix-check wiring is coordinated
    // with v-nix separately. Per-project `cdz fmt --check` over each `src/` (NOT the whole tree) so manifests/
    // `.bin`/cdz-platform/`.sexp` stay out of scope.
    //
    // RE-DISABLED 2026-08-30 (same day): the enable above was PREMATURE. The `cdz fmt` PRINTER is a MOVING
    // TARGET — after v-syntax's #6319 fmt-all, four more printer PRs landed the SAME day (#6329 seq-86/87/89,
    // #6335, #6338 seq-92/93, #6341 seq-95) WITHOUT re-fmting the corpus, so ALL 6 domain src dirs are now
    // non-canonical again (verified `cdz fmt --check` exit 1 on every one). Enforcing a fmt gate while the
    // printer churns just reds the fleet on every printer bump. TRUE precondition = the printer is FROZEN
    // (operator signals "printer stable") AND the corpus is canonical.
    // RE-ENABLED 2026-08-30: concierge relayed the operator PRINTER-FREEZE signal ("no more formatting changes,
    // everything looks great") — the full stream landed (#6329/#6335/#6338/#6341/#6352 seq-101 paren-fix/#6355
    // seq-96/97 flush), and the trailing PRs re-fmt'd the corpus, so all 6 domain src dirs are canonical under
    // the frozen printer (verified `cdz fmt --check` exit 0 + a fresh fmt-all is a NO-OP). v-nix un-holds + wires
    // the merge-required nix `cdzFmtCheck` (#6323) in lockstep with this flip.
    // WIDENED to `.sexp` 2026-08-31: fmt-normalized the 34 spec/semantics/*.sexp + added `spec/semantics` (after the
    // operator ruling-B printer-readability deps #6808/#6816 landed). — but TEMPORARILY REVERTED to .cdz-only below.
    // TEMP (C) 2026-08-31 (concierge-APPROVED): the .sexp portion is now ADVISORY (dropped from this merge-required
    // gate) — KEEP the 6 .cdz dirs merge-required. WHY: v-parser-corpus's inc-6 comment-round-trip series touches
    // printer.rs nearly every batch (#6863/#6868/#6874…), re-rendering gated .sexp (13-strings doc-header) → the
    // merge-required .sexp gate reded per-batch → a re-fmt treadmill (#6873/#6880). The .sexp fmt gate is COSMETIC
    // (canonicalization, not behavior), so advisory-until-stable is low-risk + stops the churn; .cdz stays enforced.
    // RE-PROMOTE `spec/semantics` here (+ v-nix nix cdzFmtCheck) AFTER the inc-6 comment-round-trip series COMPLETES
    // AND v-syntax declares the comment/doc PRINTER stable — do ONE final `cdz fmt spec/semantics` then flip it back.
    const CDZ_FMT_CHECK_ENFORCE: bool = true;
    if CDZ_FMT_CHECK_ENFORCE {
        log.step(
            "cdz-fmt-check",
            "cargo run -q -p cdz -- fmt --check implementation/compiler-ml/src implementation/cad/src \
             implementation/music/src implementation/des/src implementation/iterators/src \
             implementation/choreography/src",
            repo,
        );
    }
    // Mandate-enforcement lint: NO LONGER an inline step here (v-xtask-decompose 2026-08-28). The
    // mandate lint now lives in the STANDALONE `xtask-mandates` crate + the nix `mandateLintCheck`
    // (rewired to `cargo run -p xtask-mandates`), which gate-local folds into its fail-set — the
    // authoritative merge gate. Keeping an inline call here would require `xtask` to DEPEND on
    // `xtask-mandates`, defeating the independent-caching win (operator: no xtask→subcrate dep). Run it
    // via `nix run .#lint-mandates` (or `cargo xtask lint-mandates`, redirected to the app by v-ft's wrapper).
    // WARN-only: surface the byte-len-bounds-scalar-String.at latent-ASCII shape (concierge ruling C
    // part 2). Never reds the gate — always Ok unless the corpus can't be enumerated.
    log.step_native("bytelen-scalar-walk-warn", || {
        bytelen_scalar_walk_warn_lint(paths)
    });

    println!("\ncheck: all green ✓  (full log: {})", log.path.display());
}

/// The case DESCRIPTIONS in a gate-baseline file, in file order — the `verdict\tdescription` lines,
/// skipping `#` header/blank lines, taking field 2. The shared parse behind `baseline_titles` (a
/// `BTreeSet`, for the cross-file agreement lint). The within-file dup lint reads verdicts too, so it
/// parses lines directly ([`conflicting_dup_titles`]) rather than through this title-only iterator.
fn baseline_title_iter(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_v, d)| d.to_string()))
}

/// The set of case DESCRIPTIONS in a gate-baseline file. Pure, so the agreement lint is unit-tested
/// without the filesystem.
fn baseline_titles(text: &str) -> std::collections::BTreeSet<String> {
    baseline_title_iter(text).collect()
}

/// Assert the three gate baselines (wasm / rust / rust-async) cover the SAME set of case descriptions.
/// A title present in one baseline but absent from another is a stale-baseline error (typically a
/// wasm-only `gate --save` that left the rust baselines behind). Returns an actionable error naming the
/// diverging titles + the heal command; `Ok(())` when all three agree (or a baseline is absent —
/// nothing to compare, and `gate --check` already errors on a missing baseline).
pub(crate) fn baseline_titles_agree_lint(paths: &Paths) -> Result<(), String> {
    let targets = [
        (".gate-baseline", GateTarget::Wasm),
        (".gate-baseline-rust", GateTarget::Rust),
        (".gate-baseline-rust-async", GateTarget::RustAsync),
    ];
    let mut sets: Vec<(&str, std::collections::BTreeSet<String>)> = Vec::new();
    for (name, target) in targets {
        let path = baseline_path(paths, target);
        // A baseline that doesn't EXIST yet is not a divergence (the rust baselines are opt-in to
        // create; `gate --check --target <t>` errors on a missing one) — skip it. But a real read
        // ERROR (permissions, transient IO, invalid UTF-8) must FAIL LOUDLY, not be silently treated as
        // absent (which would pass the lint having checked nothing for that file). Discriminate on the
        // error kind, not a blanket Ok-else-skip.
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("cannot read baseline {}: {e}", path.display())),
        };
        sets.push((name, baseline_titles(&text)));
    }
    if sets.len() < 2 {
        return Ok(());
    }
    // The union of all titles is the reference set; report any baseline missing any of them.
    let mut union: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, s) in &sets {
        union.extend(s.iter().cloned());
    }
    let mut problems: Vec<String> = Vec::new();
    for (name, s) in &sets {
        let missing: Vec<&String> = union.difference(s).collect();
        if !missing.is_empty() {
            problems.push(format!(
                "{name} is missing {} title(s) present in another baseline:\n    {}",
                missing.len(),
                missing
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
                    .join("\n    ")
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the gate baselines disagree on their case-title SET (a wasm-only `gate --save` likely left \
         the rust baselines stale). Heal with `cargo xtask gate --save --target rust` and `--target \
         rust-async` (re-run each backend's gate + re-save), so all three cover the same cases. \
         Divergences:\n  {}",
        problems.join("\n  ")
    ))
}

/// The CONFLICTING duplicate case-titles in a baseline: a title that appears on 2+ lines with DIFFERENT
/// verdicts. Pure so it's unit-tested without the filesystem. A benign same-verdict repeat (the routine
/// `merge=union` artifact — both merge sides append their copy of an unchanged row) is NOT returned: it's
/// harmless (the description-keyed baseline map collapses it, no verdict is masked), exactly as
/// `check_baseline` and `canonicalize_baseline_text` treat it. Only a same-title/DIFFERENT-verdict dup is
/// dangerous (the map's last-wins silently masks one verdict), so only that is flagged. Malformed lines
/// (no tab / unknown verdict tag) are skipped for this classification. Returns first-seen order, stable.
fn conflicting_dup_titles(text: &str) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    let mut conflicting: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            match seen.insert(d.to_string(), verdict) {
                None => {}
                Some(prev) if prev == verdict => {} // benign same-verdict dup — tolerated
                // A conflict; record the title once (first conflicting sighting) in encounter order.
                Some(_) if !conflicting.iter().any(|t| t == d) => conflicting.push(d.to_string()),
                Some(_) => {} // already-recorded conflict on this title
            }
        }
    }
    conflicting
}

/// Assert no gate baseline has the same case-title on 2+ lines with CONFLICTING verdicts. A
/// same-title/different-verdict dup is how a bad `gate --save` / hand-append masks a verdict: the
/// description-keyed baseline map is last-wins, so `gate --check` HARD-ERRORS on it, reddening that target
/// FLEET-WIDE until healed (recurred 3× — corpus-bugfix's `545c8e44a`, `6a3e7906e`). This lint pins the
/// class at MR time. VERDICT-AWARE (fixed 2026-08-10, v-wasm-opt trunk-RED report): it tolerates BENIGN
/// same-verdict duplicate lines — the routine `merge=union` artifact from parallel baseline appends, which
/// `gate --check` itself deems harmless (the map collapses them, no masking) and passes. The old lint
/// flagged ANY repeated title, so a benign union-merge dup (e.g. 506 same-verdict repeats on trunk) red
/// `cargo xtask check` fleet-wide even though `gate --check` passed — a self-inconsistency that blocked
/// every check-gated MR. Now the lint and `gate --check` agree: benign is fine, only a conflict fails.
/// Returns an actionable error naming the conflicting title(s) + heal command; `Ok(())` when no baseline
/// has a conflicting dup (or a baseline is absent — nothing to check).
pub(crate) fn baseline_no_dup_titles_lint(paths: &Paths) -> Result<(), String> {
    let targets = [
        (".gate-baseline", GateTarget::Wasm),
        (".gate-baseline-rust", GateTarget::Rust),
        (".gate-baseline-rust-async", GateTarget::RustAsync),
    ];
    let mut problems: Vec<String> = Vec::new();
    for (name, target) in targets {
        let path = baseline_path(paths, target);
        // Absent baseline: nothing to check (rust baselines are opt-in). A real read error fails loudly,
        // mirroring `baseline_titles_agree_lint` — never silently pass having read nothing.
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("cannot read baseline {}: {e}", path.display())),
        };
        let conflicts = conflicting_dup_titles(&text);
        if !conflicts.is_empty() {
            problems.push(format!(
                "{name} has {} case-title(s) with CONFLICTING verdicts on 2+ lines:\n    {}",
                conflicts.len(),
                conflicts.join("\n    ")
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "a gate baseline has the SAME case-title on 2+ lines with DIFFERENT verdicts — the map-keyed \
         baseline masks one (last-wins) and `gate --check` hard-errors, reddening that target FLEET-WIDE. \
         (Benign same-verdict duplicate lines — the routine `merge=union` artifact — are tolerated; only a \
         verdict CONFLICT fails this lint.) Heal by removing the wrong line (keep the correct verdict), or \
         re-run `cargo xtask gate --save --target <t>` / `cargo xtask canonicalize-baselines` to rewrite the \
         file clean. Conflicting:\n  {}",
        problems.join("\n  ")
    ))
}

/// The 1-based line numbers in `text` that open a `(needs …)` clause — a line whose first
/// non-whitespace token is `(needs` (the shape the corpus uses and the codemod strips). Pure, so the
/// lint's matching is unit-tested without touching the filesystem.
fn needs_clause_lines(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with("(needs "))
        .map(|(i, _)| i + 1)
        .collect()
}

/// The first-argument IDENTIFIER of every `(<head> <arg> …)` call in `program` whose head is exactly
/// `head` (e.g. `String.byte-len`, `String.at`). A cheap text scan: find each `(<head> ` occurrence and
/// take the next whitespace-delimited token, keeping it only if it's a bare identifier (not `(`, a string
/// literal, or a number) — a string being indexed/measured is always a bound variable in the corpus. Used
/// by the Tier-2 byte-len-vs-scalar-walk lint; pure so the heuristic is unit-tested without the filesystem.
fn call_first_ident_args(program: &str, head: &str) -> std::collections::BTreeSet<String> {
    let needle = format!("({head} ");
    let mut out = std::collections::BTreeSet::new();
    let mut rest = program;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        // The first token after the head is the receiver arg. A bare identifier is our target; skip a
        // nested call `(`, a string literal `"`, or a numeric literal (those aren't the string var).
        let tok: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ')' && *c != '(')
            .collect();
        if !tok.is_empty() && !tok.starts_with('"') && !token_is_numeric_literal(&tok) {
            out.insert(tok);
        }
        rest = after;
    }
    out
}

/// Does this bare token START a numeric literal (so `call_first_ident_args` must skip it, per its
/// "skip a numeric literal" contract)? A leading ASCII digit (`0`, `42`) OR a sign followed by a digit
/// (`-1`, `+1`) — the signed case the first-char-digit-only check missed (PR#836 review), which let a
/// signed arg be collected as a spurious "identifier". A lone `-`/`+` (an operator head, not a number)
/// is NOT numeric, so it correctly stays a non-match here. Pure so the classification is unit-pinned.
fn token_is_numeric_literal(tok: &str) -> bool {
    let mut chars = tok.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('-' | '+') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// The Tier-2 warn-lint heuristic (concierge ruling C part 2): the string identifiers `x` in `program`
/// that are BOTH measured by `String.byte-len(x)` AND scalar-indexed by `String.at(x …)`/`String.slice(x
/// …)` — the latent-ASCII bug shape (byte-len bounds a CODEPOINT walk, wrong for multi-byte UTF-8). Returns
/// the offending idents (empty = clean). EXCLUSIONS fall out for free: a `Bytes.at`/`Bytes.len` walk never
/// produces a `String.byte-len` match; a byte-len used only as output has no `String.at(x)` on the same x.
/// Same-x co-occurrence only — a case that RENAMES x between measure and walk slips (Tier-1-only, accepted
/// per the ruling: warn-level, note holds the rest). Pure; unit-tested against the fixture set.
fn bytelen_scalar_walk_idents(program: &str) -> Vec<String> {
    let measured = call_first_ident_args(program, "String.byte-len");
    let mut walked = call_first_ident_args(program, "String.at");
    walked.extend(call_first_ident_args(program, "String.slice"));
    measured.intersection(&walked).cloned().collect()
}

/// Scan every `spec/semantics/*.sexp` for a `(needs …)` clause (the retired capability tag) and return
/// an actionable error listing each `file:line` if any survive. Returns `Ok(())` when needs-free.
fn needs_free_lint(paths: &Paths) -> Result<(), String> {
    let dir = paths.repo.join("spec/semantics");
    let mut hits: Vec<String> = Vec::new();
    // A lint must FAIL LOUDLY if it cannot enumerate its inputs — a silent `unwrap_or_default()` here
    // would make the whole check pass VACUOUSLY (0 files scanned) on an unreadable corpus dir, hiding a
    // re-introduced clause. Propagate the error instead.
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("cannot read corpus dir {}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read a corpus dir entry in {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|x| x == "sexp") {
            files.push(path);
        }
    }
    files.sort();
    // The corpus always ships many `spec/semantics/*.sexp`; zero means we're scanning the wrong tree (a
    // bad `paths.repo`) — treat it as a hard error, not a vacuous pass.
    if files.is_empty() {
        return Err(format!(
            "no *.sexp corpus files found under {}",
            dir.display()
        ));
    }
    for file in &files {
        // A file we can enumerate but not READ could hide a `(needs …)` clause — fail, don't skip.
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read corpus file {}: {e}", file.display()))?;
        let rel = file.strip_prefix(&paths.repo).unwrap_or(file).display();
        for line_no in needs_clause_lines(&text) {
            hits.push(format!("{rel}:{line_no}"));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    Err(format!(
        "found {} `(needs …)` clause(s) — this tag is RETIRED (see \
         issues/DIRECTIVE-retire-needs-tag.md). Fix: delete each `(needs …)` clause; decline is the \
         sole todo signal (a case the compiler declines already grades Todo automatically). At:\n  {}",
        hits.len(),
        hits.join("\n  ")
    ))
}

/// Split a corpus `.sexp` file into per-case TEXT blocks, each starting at a top-level `(case ` opener (the
/// text from one `(case ` up to the next). Cheap + scope-correct for a text-scan lint: the byte-len/String.at
/// co-occurrence must be checked WITHIN one case (else two unrelated cases could spuriously combine). The
/// leading preamble before the first `(case ` (comments) is dropped. Pairs each block with the case TITLE
/// (the quoted string after `(case `) for a readable warning. Pure; unit-tested.
fn split_corpus_cases(text: &str) -> Vec<(String, String)> {
    let mut cases = Vec::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if line.trim_start().starts_with("(case ") {
            if let Some(block) = cur.take() {
                cases.push(block);
            }
            cur = Some(String::new());
        }
        if let Some(block) = cur.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    if let Some(block) = cur.take() {
        cases.push(block);
    }
    cases
        .into_iter()
        .map(|block| {
            // Title = the quoted string after `(case `; fall back to the first line.
            let title = block
                .split_once("(case ")
                .and_then(|(_, r)| r.split('"').nth(1))
                .unwrap_or_else(|| block.lines().next().unwrap_or(""))
                .to_string();
            (title, block)
        })
        .collect()
}

/// Tier-2 WARN-level lint (concierge ruling C part 2): scan every `spec/semantics/*.sexp` case for the
/// byte-len-bounds-a-scalar-String.at shape (`bytelen_scalar_walk_idents`) and PRINT a warning per hit.
/// WARN-only — always returns `Ok(())` (never fails the gate): the same-x heuristic is deliberately
/// imprecise (misses a renamed x, and a rare genuine mixed case could false-positive), so it SURFACES the
/// latent-ASCII risk without blocking. Fails loudly only if it cannot enumerate the corpus (a vacuous pass
/// would hide the class), mirroring `needs_free_lint`.
fn bytelen_scalar_walk_warn_lint(paths: &Paths) -> Result<(), String> {
    let dir = paths.repo.join("spec/semantics");
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("cannot read corpus dir {}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read a corpus dir entry in {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|x| x == "sexp") {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "no *.sexp corpus files found under {}",
            dir.display()
        ));
    }
    let mut warnings: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read corpus file {}: {e}", file.display()))?;
        let rel = file.strip_prefix(&paths.repo).unwrap_or(file).display();
        for (title, block) in split_corpus_cases(&text) {
            let idents = bytelen_scalar_walk_idents(&block);
            if !idents.is_empty() {
                warnings.push(format!("{rel}: case {title:?} — String.byte-len + String.at/slice on the same string ({})", idents.join(", ")));
            }
        }
    }
    if warnings.is_empty() {
        return Ok(());
    }
    // WARN, not fail: print to stderr and still return Ok.
    eprintln!(
        "xtask check [WARN] byte-len-bounds-scalar-walk: {} case(s) measure a string with String.byte-len \
         (a BYTE count) then scalar-index the SAME string with String.at/String.slice (a CODEPOINT walk) — \
         wrong for multi-byte UTF-8 (the latent-ASCII bug, concierge ruling C). Prefer a codepoint-length \
         bound (e.g. String.scalar-len) for a String.at walk, or byte-walk via Bytes.at/Bytes.len. If the walk \
         is genuinely byte-correct, ignore this warn. At:\n  {}",
        warnings.len(),
        warnings.join("\n  ")
    );
    Ok(())
}

/// Reproduce CI's STORELESS `test` job locally: run the two crates every store-guard-miss incident came
/// from (`rcdzc --lib` + the cdz integration tests) with `CADENZA_STORE` pointed at an EMPTY temp dir, so
/// a value-heap-driving test whose store-skip guard is MISSING or WRONG runs, fails to resolve the
/// runtime, and FAILS here — catching the local-green/CI-red BEFORE the MR lands. A correctly-guarded
/// test SKIPS (the guards + resolvers honor `CADENZA_STORE` uniformly, reporting the empty store absent),
/// so the rerun is GREEN. `Ok(())` = every test either passed or store-skipped; `Err(msg)` names the
/// crate whose storeless run failed. Test-EXECUTION only — `check`'s `build`/`test` steps already
/// compiled these crates, and the guarded tests don't run the runtime.
fn storeless_rerun(paths: &Paths) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TICK: AtomicU64 = AtomicU64::new(0);
    let tick = TICK.fetch_add(1, Ordering::Relaxed);
    // An EMPTY store dir — the whole mechanism: the guards resolve `CADENZA_STORE` first and, finding it
    // empty, report the store ABSENT so a runtime-driving test skips. Unique per invocation (PID + atomic
    // tick; `Date`/rng unavailable) so concurrent gate workers never share/clobber it.
    let empty_store =
        std::env::temp_dir().join(format!("cdz-storeless-{}-{tick}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&empty_store) {
        return Err(format!(
            "could not create the empty storeless temp dir {}: {e}",
            empty_store.display()
        ));
    }

    // The recurring storeless incident this still guards: rcdzc erased-newtype. Scoped, not a full
    // `--workspace`, so this stays a fraction of a second test phase (concierge contention ruling).
    //
    // The two cdz `--test` entries (test_manifest_cli sum-property, run_emitted_cli run-emitted-decline)
    // were REMOVED 2026-08-09: the cdz CLI integration-test suite was consolidated then DELETED wholesale
    // (`5f14b9c40` dropped `cdz/tests/suite/**`, 60 files), so those `--test <name>` targets no longer
    // exist and `cargo test -p cdz --test test_manifest_cli` errored `no test target named …` — a stale
    // reference that false-failed `xtask check` for every worktree before it ran a single test (v-inference
    // flagged). With the tests gone there is nothing left to storeless-rerun for cdz; rcdzc --lib remains
    // the live coverage. Re-add a cdz entry ONLY if a runtime-driving cdz test with a store-skip guard
    // returns.
    let store_str = empty_store.to_string_lossy().to_string();
    let runs: [(&str, &[&str]); 1] = [("rcdzc --lib", &["test", "-p", "rcdzc", "--lib"])];

    let mut failures: Vec<String> = Vec::new();
    for (label, args) in runs {
        let out = std::process::Command::new("cargo")
            .args(args)
            .current_dir(&paths.repo)
            .env("CADENZA_STORE", &store_str)
            // rcdzc's `--lib` suite has deep-recursion tests that overflow the default 8MB thread stack
            // under gate-batch load (the known dev-profile `rcdzc-compile` stack trip — nondeterministic,
            // load-dependent). Bake the same 64MB bump the miri + bench rcdzc runs use, so a known-good
            // base never reds this step on a stack overflow (which would flat-line trunk — it did, ~29min
            // / backlog 173, before this). Harmless for the cdz test targets that don't need it.
            .env("RUST_MIN_STACK", "67108864")
            .output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                // A storeless failure = an under-guarded runtime-driving test (the CI-red this catches).
                // Surface the captured tail so the offending test is named in the check log.
                let stderr = String::from_utf8_lossy(&o.stderr);
                let tail: String = stderr
                    .lines()
                    .rev()
                    .take(20)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                failures.push(format!(
                    "`{label}` FAILED storeless (missing/wrong store-skip guard):\n{tail}"
                ));
            }
            Err(e) => failures.push(format!("`{label}` could not launch: {e}")),
        }
    }

    let _ = std::fs::remove_dir_all(&empty_store);

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} storeless rerun(s) FAILED — a value-heap-driving test ran without its store-skip guard, \
             which reds CI's storeless `cargo test --workspace` (no store). Fix: add a \
             `store_present()`/`find_runtime_wasm→None` skip to the offending test so it skips when \
             `CADENZA_STORE` resolves to an empty/absent store.\n\n{}",
            failures.len(),
            failures.join("\n\n")
        ))
    }
}

/// A captured-output log for a multi-step command. Each step's child process writes its stdout and
/// stderr into one appended file; the console stays quiet on success and gets the whole log on the
/// first failure (with the path), so there is nothing to re-run to see what happened.
struct Log {
    path: PathBuf,
    file: std::fs::File,
}

impl Log {
    /// Open `target/xtask-logs/<cmd>-<timestamp>.log` (timestamped, so runs don't overwrite).
    fn create(paths: &Paths, cmd: &str) -> Log {
        let dir = paths.repo.join("target/xtask-logs");
        std::fs::create_dir_all(&dir).expect("create xtask-logs dir");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("{cmd}-{stamp}.log"));
        let file = std::fs::File::create(&path).expect("create log file");
        Log { path, file }
    }

    /// Run a step with its output captured to the log; print `✓ name` on success. On failure, dump
    /// the whole log to the console and exit. `cmd` is a `program arg arg…` string run in `dir`.
    fn step(&mut self, name: &str, cmd: &str, dir: &Path) {
        self.run_step(name, cmd, dir, false, &[]);
    }

    /// Like `step`, but with extra environment variables set on the child. Used for the wasm-runtime
    /// build, which needs `RUSTC_BOOTSTRAP=1` to enable the runtime's `build-std` (see
    /// `cdz-runtime/.cargo/config.toml`) on the stable pin — without leaking that into the native
    /// build/test/clippy steps.
    fn step_env(&mut self, name: &str, cmd: &str, dir: &Path, env: &[(&str, &str)]) {
        self.run_step(name, cmd, dir, false, env);
    }

    /// Like `step`, but the child's output ALSO streams to the console (tee) — for a step whose own
    /// output is a concise, useful signal (the gate's tally), not build noise.
    fn step_show(&mut self, name: &str, cmd: &str, dir: &Path) {
        self.run_step(name, cmd, dir, true, &[]);
    }

    /// Run a NATIVE (in-process) check step — a closure that returns `Ok(())` on pass or `Err(msg)`
    /// with an actionable message on fail. Used for a check that isn't a subprocess (e.g. a corpus
    /// lint that scans files). Same console/log contract as `step`: `✓ name` on success; on failure
    /// the message is written to the log and dumped, then exit non-zero.
    fn step_native(&mut self, name: &str, check: impl FnOnce() -> Result<(), String>) {
        use std::io::Write;
        writeln!(self.file, "\n==== {name}: (native) ====").ok();
        match check() {
            Ok(()) => {
                self.file.flush().ok();
                println!("  ✓ {name}");
            }
            Err(msg) => {
                writeln!(self.file, "{msg}").ok();
                self.file.flush().ok();
                eprintln!("  ✗ {name} — FAILED");
                self.dump_and_exit(name);
            }
        }
    }

    fn run_step(&mut self, name: &str, cmd: &str, dir: &Path, show: bool, env: &[(&str, &str)]) {
        use std::io::Write;
        writeln!(self.file, "\n==== {name}: {cmd} ====").ok();
        self.file.flush().ok();

        // Split the command string into program + args (our commands have no quoted args).
        let mut parts = cmd.split_whitespace();
        let program = parts.next().expect("non-empty command");
        let args: Vec<&str> = parts.collect();

        // Capture stdout+stderr. (A true live tee would need thread-per-pipe; capture-then-print is
        // enough here — steps are short and the console output is the point, not streaming.)
        let mut command = std::process::Command::new(program);
        command.args(&args).current_dir(dir);
        for (k, v) in env {
            command.env(k, v);
        }
        let out = command.output().unwrap_or_else(|e| {
            eprintln!("  ✗ {name} — could not launch: {e}");
            std::process::exit(1);
        });
        self.file.write_all(&out.stdout).ok();
        self.file.write_all(&out.stderr).ok();
        self.file.flush().ok();
        if show {
            std::io::stdout().write_all(&out.stdout).ok();
            std::io::stderr().write_all(&out.stderr).ok();
        }

        if out.status.success() {
            println!("  ✓ {name}");
        } else {
            eprintln!("  ✗ {name} — FAILED");
            self.dump_and_exit(name);
        }
    }

    /// Like `step`, but with a hard wall-clock `timeout` on the whole command. On timeout the child is
    /// killed and the step FAILS LOUDLY — the log records `TIMED OUT after Ns` naming the step, then
    /// `dump_and_exit` surfaces it. This converts a silent multi-minute HANG (a stuck `cdz test` suite
    /// that emits no output and no TOTAL — which every observer, including the concierge, misread as an
    /// "environmental kill") into an instantly-actionable, auto-bisectable named failure. Any captured
    /// partial output (the last case it was on) is written to the log first, so the hang is localized.
    fn step_timed(&mut self, name: &str, cmd: &str, dir: &Path, timeout: std::time::Duration) {
        use std::io::Write;
        writeln!(
            self.file,
            "\n==== {name}: {cmd} (timeout {}s) ====",
            timeout.as_secs()
        )
        .ok();
        self.file.flush().ok();

        let mut parts = cmd.split_whitespace();
        let program = parts.next().expect("non-empty command");
        let args: Vec<&str> = parts.collect();

        let child = std::process::Command::new(program)
            .args(&args)
            .current_dir(dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                eprintln!("  ✗ {name} — could not launch: {e}");
                std::process::exit(1);
            });

        let started = std::time::Instant::now();
        match wait_with_timeout(child, timeout) {
            Ok(Some(out)) => {
                self.file.write_all(&out.stdout).ok();
                self.file.write_all(&out.stderr).ok();
                self.file.flush().ok();
                if out.status.success() {
                    println!("  ✓ {name}");
                } else {
                    eprintln!("  ✗ {name} — FAILED");
                    self.dump_and_exit(name);
                }
            }
            // Killed at the deadline — a true hang. Write partial output (localizes the stuck case), then
            // fail loudly with the elapsed time and step name.
            Ok(None) => {
                let elapsed = started.elapsed().as_secs();
                writeln!(
                    self.file,
                    "\n{name}: TIMED OUT after {elapsed}s (killed) — the suite hung (no completion). \
                     The last case above is where it stuck; this is a HANG, not a slow pass. Raise \
                     CDZ_SUITE_TIMEOUT_SECS only if it is genuinely slow-but-passing."
                )
                .ok();
                self.file.flush().ok();
                eprintln!("  ✗ {name} — TIMED OUT after {elapsed}s (hang)");
                self.dump_and_exit(name);
            }
            Err(e) => {
                eprintln!("  ✗ {name} — wait failed: {e}");
                self.dump_and_exit(name);
            }
        }
    }

    /// Run the compiler-ml suite ONE FILE AT A TIME under a tight PER-FILE wall-clock cap, memoizing the
    /// whole-suite GREEN verdict on a content `key`: if a green verdict for this exact key is already
    /// cached (a prior check this batch ran the identical sweep and it passed), SKIP the re-run and reuse
    /// it. Only GREEN is cached — a RED always re-runs fresh (so the failure log is current and a stale
    /// cache can never manufacture a false green). This kills the dominant per-batch waste: pr-sync
    /// re-running the ~25-30min compiler-ml sweep multiple times per batch even when (compiler, corpus)
    /// didn't change between runs.
    ///
    /// Running per file (vs one whole-suite process under a generous suite ceiling) is the runaway-compile
    /// unblock (2026-07-20): a single pathological def can blow up the ML compiler and NEVER exit, and
    /// under a whole-suite step it burns the ENTIRE 45min budget before failing — and pr-sync re-runs
    /// `check` several times a batch, so ONE bad file froze the whole ~200-MR backlog for ~1h. Running
    /// `cdz test <file>` per file, each bounded by `ml_per_file_timeout`, means the runaway file (and
    /// only it) fails LOUD + NAMED at the tight cap while every innocent file still runs to a verdict —
    /// so the gate COMPLETES and the offending file is auto-bisectable, exactly the concierge's ask.
    ///
    /// This is semantically identical to `cdz test <suite_dir>`: dir-mode already loops the runner's
    /// `run_test_file` over the same file list (each file resolving its own import closure but running
    /// only its OWN `@test`s), so splitting the loop into the harness changes only WHERE each file's
    /// wall-clock is bounded, not WHAT runs. Files are the suite's TRACKED `src/*.cdz` (via `git
    /// ls-files`, path-sorted) — tracked-only so a local untracked scratch file can't diverge the local
    /// gate from pr-sync's clean worktree. FAIL-OPEN: if enumeration finds no files (a manifest
    /// restructure that moves tests out of `src/`), fall back to a whole-suite run so a coupling drift
    /// degrades to today's behavior, never to a false green. The whole-suite GREEN verdict is cached on
    /// the same (binary ‖ tree) key, so an unchanged (compiler, corpus) skips the whole per-file sweep.
    fn step_cached_per_file(
        &mut self,
        name: &str,
        cdz_bin: &str,
        suite_dir: &Path,
        repo: &Path,
        cache: Option<&CachedStep>,
        whole_suite_timeout: std::time::Duration,
    ) {
        use std::io::Write;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        if let Some(c) = cache
            && c.is_green()
        {
            writeln!(
                self.file,
                "\n==== {name}: CACHED green (key {}) ====",
                c.short_key()
            )
            .ok();
            self.file.flush().ok();
            println!(
                "  ✓ {name} (cached — unchanged compiler + corpus since a green run this batch)"
            );
            return;
        }

        // Enumerate the suite's TRACKED `src/*.cdz`, path-sorted (git ls-files is already sorted).
        let src = suite_dir.join("src");
        let files: Vec<PathBuf> = std::process::Command::new("git")
            .args(["ls-files", "-z", "--"])
            .arg(&src)
            .current_dir(repo)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                o.stdout
                    .split(|&b| b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("cdz"))
                    .collect()
            })
            .unwrap_or_default();

        // Fail-open: no tracked src/*.cdz found (unexpected — manifest moved the tests) → run the whole
        // suite the old way so we never skip coverage or manufacture a false green.
        if files.is_empty() {
            writeln!(
                self.file,
                "\n==== {name}: per-file enumeration empty — falling back to whole-suite run ===="
            )
            .ok();
            self.file.flush().ok();
            let cmd = format!("{cdz_bin} test {}", suite_dir.display());
            self.step_timed(name, &cmd, repo, whole_suite_timeout);
            if let Some(c) = cache {
                c.record_green();
            }
            return;
        }

        let per_file = ml_per_file_timeout();

        // WARM-ONCE before the parallel sweep (operator P0, gate <10min). The per-file pool below runs
        // each file as its OWN `cdz test <file>` process for runaway-compile localization; on a COLD
        // provider cache the concurrent workers that share a closure ALL miss and re-emit the ~1360-def
        // shared provider in parallel (the N×-redundant-emit race — the 8 `sread-eval-*` files each cold-
        // emitting the whole closure). One SERIAL `cdz test <suite_dir> --warm-only` emits + persists each
        // closure group's provider to the shared default-store cache ONCE, then exits without running
        // tests, so every per-file run below HITS instead of racing the emit. Same cdz binary + cwd as the
        // workers → same providers dir → they share the warmed cache with no env.
        //
        // Best-effort: a warm-only RED means the suite is genuinely parse/check-broken — but the per-file
        // sweep is the AUTHORITATIVE gate (it localizes the failing file), so a failed/timed-out warm just
        // forgoes the cache benefit this run rather than failing the gate here. Log the outcome either way.
        // Capture whether warm SUCCEEDED — the per-file jobs cap is coupled to it (a cold sweep after a
        // warm timeout stays at the conservative cap so it doesn't race the per-file timeout; see ml_test_jobs).
        let warm_succeeded;
        {
            let warm_started = std::time::Instant::now();
            writeln!(
                self.file,
                "\n──── warm-once: {cdz_bin} test {} --warm-only ────",
                suite_dir.display()
            )
            .ok();
            self.file.flush().ok();
            match std::process::Command::new(cdz_bin)
                .arg("test")
                .arg(suite_dir)
                .arg("--warm-only")
                .current_dir(repo)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map(|child| wait_with_timeout(child, whole_suite_timeout))
            {
                Ok(Ok(Some(out))) => {
                    self.file.write_all(&out.stdout).ok();
                    self.file.write_all(&out.stderr).ok();
                    let secs = warm_started.elapsed().as_secs();
                    let ok = out.status.success();
                    if ok {
                        writeln!(
                            self.file,
                            "  ✓ warm-once OK in {secs}s — per-file sweep will HIT the cache"
                        )
                        .ok();
                    } else {
                        writeln!(self.file, "  ⚠ warm-once returned non-zero in {secs}s (suite may be broken; the per-file sweep below localizes it) — proceeding uncached").ok();
                    }
                    // Only a clean exit means the providers are warmed; a non-zero exit did NOT populate the
                    // cache reliably, so treat it as not-warmed → conservative jobs cap.
                    warm_succeeded = ok;
                }
                Ok(Ok(None)) => {
                    writeln!(self.file, "  ⚠ warm-once TIMED OUT at the suite cap — proceeding uncached (per-file sweep is authoritative)").ok();
                    warm_succeeded = false;
                }
                Ok(Err(e)) => {
                    writeln!(
                        self.file,
                        "  ⚠ warm-once wait error ({e}) — proceeding uncached"
                    )
                    .ok();
                    warm_succeeded = false;
                }
                Err(e) => {
                    writeln!(
                        self.file,
                        "  ⚠ warm-once launch error ({e}) — proceeding uncached"
                    )
                    .ok();
                    warm_succeeded = false;
                }
            }
            self.file.flush().ok();
        }

        // Now that the warm outcome is known, size the per-file pool: 4 if warmed (cheap HITs), else the
        // conservative 2 (a cold sweep must not race the per-file cap at 4-way — reviewer FYI). Log the header.
        let jobs = ml_test_jobs(files.len(), warm_succeeded);
        writeln!(
            self.file,
            "\n==== {name}: {} file(s), {jobs} concurrent job(s) (warm {}), per-file cap {}s ====",
            files.len(),
            if warm_succeeded { "HIT" } else { "COLD" },
            per_file.as_secs()
        )
        .ok();
        self.file.flush().ok();

        // Run the files CONCURRENTLY (bounded worker pool pulling from a shared cursor), each still
        // under the per-file cap. The suite's wall-clock is a SUM of per-file full-pipeline compiles
        // dominated by one ~480s file, so overlapping them collapses the serial ~45min sum toward that
        // single-file floor. Each worker captures its own file's output + verdict into its own slot;
        // AFTER all workers join, the main thread writes them to the log IN FILE ORDER and dump-exits on
        // the FIRST failure during that ordered replay — so the captured log and the "FAILED at <file>"
        // localization are byte-for-byte what the old serial loop produced, independent of the order
        // workers happened to finish in. (This is first-failure in the replay, not failure latency: every
        // file has already run by then.) No shared mutable state across workers: each spawns its own child
        // and writes only its own slot.
        let n = files.len();
        let slots: Vec<Mutex<Option<PerFileResult>>> = (0..n).map(|_| Mutex::new(None)).collect();
        let cursor = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..jobs {
                let cursor = &cursor;
                let slots = &slots;
                let files = &files;
                scope.spawn(move || {
                    loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= files.len() {
                            break;
                        }
                        let file = &files[i];
                        // Build the child directly from (binary, "test", file) rather than parsing a
                        // command STRING — a whitespace-split would corrupt any path containing a space.
                        let started = std::time::Instant::now();
                        let verdict = match std::process::Command::new(cdz_bin)
                            .arg("test")
                            .arg(file)
                            .current_dir(repo)
                            .stdout(std::process::Stdio::piped())
                            .stderr(std::process::Stdio::piped())
                            .spawn()
                        {
                            Err(e) => PerFileVerdict::LaunchErr(e.to_string()),
                            Ok(child) => match wait_with_timeout(child, per_file) {
                                Ok(Some(out)) => PerFileVerdict::Ran {
                                    stdout: out.stdout,
                                    stderr: out.stderr,
                                    ok: out.status.success(),
                                },
                                Ok(None) => PerFileVerdict::TimedOut {
                                    elapsed: started.elapsed().as_secs(),
                                },
                                Err(e) => PerFileVerdict::WaitErr(e.to_string()),
                            },
                        };
                        *slots[i].lock().unwrap() = Some(PerFileResult { verdict });
                    }
                });
            }
        });

        // Ordered write-out: the scope above has joined every worker, so all files have run. Replay the
        // captured output/verdicts in FILE ORDER and dump-exit on the FIRST failure in that replay —
        // identical log + localization to the old serial loop.
        for (i, file) in files.iter().enumerate() {
            let fname = file.display().to_string();
            writeln!(self.file, "\n──── {cdz_bin} test {fname} ────").ok();
            let result = slots[i]
                .lock()
                .unwrap()
                .take()
                // A slot is always Some after the scope joins (every index i < n is claimed exactly once
                // by fetch_add); treat a None as a launch/panic anomaly rather than silently skipping.
                .unwrap_or(PerFileResult {
                    verdict: PerFileVerdict::WaitErr("worker produced no result".into()),
                });
            match result.verdict {
                PerFileVerdict::Ran { stdout, stderr, ok } => {
                    self.file.write_all(&stdout).ok();
                    self.file.write_all(&stderr).ok();
                    self.file.flush().ok();
                    if !ok {
                        eprintln!("  ✗ {name} — FAILED at {fname}");
                        self.dump_and_exit(name);
                    }
                }
                // The runaway-compile case: this ONE file blew past the per-file cap. Fail loud + named
                // so it is immediately bisectable, instead of the old whole-suite hang that burned 45min
                // with no localization.
                PerFileVerdict::TimedOut { elapsed } => {
                    writeln!(
                        self.file,
                        "\n{name}: {fname} TIMED OUT after {elapsed}s (killed) — a single compile ran \
                         past the per-file cap and never exited (runaway compile), not a slow pass. This \
                         file is the offending def to bisect/quarantine. Raise \
                         CDZ_ML_PER_FILE_TIMEOUT_SECS only if it is genuinely slow-but-passing."
                    )
                    .ok();
                    self.file.flush().ok();
                    eprintln!(
                        "  ✗ {name} — TIMED OUT at {fname} after {elapsed}s (runaway compile)"
                    );
                    self.dump_and_exit(name);
                }
                PerFileVerdict::LaunchErr(e) => {
                    eprintln!("  ✗ {name} — could not launch ({fname}): {e}");
                    std::process::exit(1);
                }
                PerFileVerdict::WaitErr(e) => {
                    eprintln!("  ✗ {name} — wait failed ({fname}): {e}");
                    self.dump_and_exit(name);
                }
            }
        }

        println!(
            "  ✓ {name} ({} files, {jobs} jobs, per-file capped)",
            files.len()
        );
        // Every file passed — record the whole-suite green on the same key the cache-hit path checks.
        if let Some(c) = cache {
            c.record_green();
        }
    }

    /// Print the whole captured log to the console (so the failure is readable without re-running)
    /// plus its path, then exit non-zero.
    fn dump_and_exit(&self, failed_step: &str) -> ! {
        eprintln!("\n──── full log ({}) ────", self.path.display());
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            eprint!("{text}");
        }
        eprintln!("──── end log ────");
        eprintln!(
            "\ncheck: FAILED at `{failed_step}` — full log above and at {}",
            self.path.display()
        );
        std::process::exit(1);
    }
}

/// A content-keyed GREEN-verdict cache for one expensive `check` step (the compiler-ml `cdz test` sweep,
/// ~25-30min). The key is a hash of everything that can change the verdict — the `cdz` compiler binary
/// (which embeds rcdzc) plus the suite's source tree — so an unchanged (compiler, corpus) reuses the
/// cached green and a change to EITHER forces a fresh run (no coverage loss). Only green is stored; red
/// re-runs fresh. The marker is a file `<cache_dir>/<name>.<key>.green`; its presence == green.
struct CachedStep {
    marker: PathBuf,
    key: String,
}

impl CachedStep {
    /// Build the cache handle for step `name` over inputs (`binary`, `tree`). Returns `None` (caching
    /// disabled → always run) if the key can't be computed — a missing binary/tree, or an unreadable
    /// cache dir. Fail-open to a real run is always safe; the only unsafe outcome (a false green) is
    /// impossible because we only ever CACHE a verdict we just observed green.
    fn new(paths: &Paths, name: &str, binary: &Path, tree: &Path) -> Option<CachedStep> {
        Self::new_multi(paths, name, binary, &[tree])
    }

    /// Like `new`, but the key covers SEVERAL source trees — for a step whose inputs span more than one
    /// dir (e.g. the storeless rerun, whose guarded tests live in BOTH `cdz/tests` AND `rcdzc/src`: the
    /// rcdzc `#[cfg(test)]` unit tests are NOT compiled into the release `cdz` binary, so keying on the
    /// binary alone would miss an edit to `rcdzc/src/tests.rs` — the exact file holding the rcdzc
    /// store-guarded tests). Each tree is hashed and folded into the key in the given order, domain-
    /// separated from the binary and from each other so a byte can't migrate between inputs.
    fn new_multi(paths: &Paths, name: &str, binary: &Path, trees: &[&Path]) -> Option<CachedStep> {
        let bin_bytes = std::fs::read(binary).ok()?;
        let mut combined = content_address(&bin_bytes);
        for tree in trees {
            combined.push(':');
            combined.push_str(&hash_tree(tree)?);
        }
        let key = content_address(combined.as_bytes());
        let dir = paths.repo.join("target/xtask-cache");
        std::fs::create_dir_all(&dir).ok()?;
        let safe_name = name.replace(['/', ' '], "_");
        let marker = dir.join(format!("{safe_name}.{key}.green"));
        Some(CachedStep { marker, key })
    }

    /// Is a green verdict for this exact key already recorded?
    fn is_green(&self) -> bool {
        self.marker.exists()
    }

    /// Record a green verdict (best-effort; a write failure just means the next run recomputes).
    fn record_green(&self) {
        let _ = std::fs::write(&self.marker, &self.key);
    }

    fn short_key(&self) -> &str {
        &self.key[..self.key.len().min(12)]
    }
}

// ============================================================================================
// roundtrip — the syntax surfaces round-trip on every corpus program.
// ============================================================================================

// ============================================================================================
// emit — compile a program to a component and write it out (the compile-only half of `run`).
// ============================================================================================

fn emit(paths: &Paths, profile: &str, file: &Path, from: &str, out: Option<PathBuf>) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    if !file.exists() {
        eprintln!("xtask emit: no such file: {}", file.display());
        std::process::exit(1);
    }
    let tools = build_tools(paths, profile);
    let out = out.unwrap_or_else(|| file.with_extension("wasm"));

    // cdz-syntax convert <file> | rcdzc - -o <out>.
    let syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    let ast = syntax.wait_with_output().expect("wait cdz-syntax");
    if !ast.status.success() {
        std::process::exit(ast.status.code().unwrap_or(1));
    }
    let mut rcdzc = Command::new(&tools.rcdzc)
        .args(["compile", "-", "-o"])
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    rcdzc.stdin.take().unwrap().write_all(&ast.stdout).ok();
    let status = rcdzc.wait().expect("wait rcdzc");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    println!("wrote {}", out.display());
}

/// CANONICALIZE a built runtime component for content-addressing: strip ALL custom sections
/// (`wasm-tools strip -a`), which removes the non-deterministic `producers` sections that embed
/// tool-version strings — `rustc 1.95.0 (<commit>)`, `wit-component X.Y`, `cargo-component X.Y`,
/// `clang …`, `wit-bindgen …`. Those version strings differ across machines/toolchains, so hashing the
/// RAW build made `REQUIRED_RUNTIME_HASH` machine-specific (a program pinned one hash on box A that box
/// B's rebuild never reproduced). Stripping them makes the hash reproducible for a given rustc RELEASE
/// (a residual `/rustc/<commit>/…/raw_vec/mod.rs` panic-location string still lives in a data segment —
/// program data, not a custom section, so `strip` can't remove it; it is identical for the same rustc
/// commit, so it does not break reproducibility across machines sharing a toolchain). Both the stored
/// artifact AND the hash use the stripped bytes, so a composed program's imported hash matches the
/// stored file. Requires `wasm-tools` on PATH (`cargo install wasm-tools`); a missing binary is a hard
/// error (the hash would otherwise silently regress to the non-reproducible raw form).
pub(crate) fn canonicalize_runtime(raw_wasm_path: &Path) -> Vec<u8> {
    let out = raw_wasm_path.with_extension("stripped.wasm");
    let status = std::process::Command::new("wasm-tools")
        .arg("strip")
        .arg("-a")
        .arg(raw_wasm_path)
        .arg("-o")
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "wasm-tools strip failed ({s}) on {} — cannot canonicalize the runtime",
                raw_wasm_path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "wasm-tools not runnable ({e}); install it with `cargo install wasm-tools` so the \
                 runtime hash is reproducible (it strips the tool-version `producers` sections)"
            );
            std::process::exit(1);
        }
    }
    let bytes = std::fs::read(&out)
        .unwrap_or_else(|e| panic!("read stripped runtime {}: {e}", out.display()));
    let _ = std::fs::remove_file(&out);
    bytes
}

/// The value-heap runtime's NFC-dependency import interface — the bare WIT name the built heap imports
/// (`cadenza:nfc/normalize`). [`stamp_nfc_into_heap`] rewrites it to the SELF-DESCRIBING content-addressed
/// form `cadenza:nfc/normalize@0.0.0+<hash>`. Kept as one constant so `build` and `codegen` stamp the same
/// interface (mirrors `rcdzc`'s codegen'd `NFC_IFACE`).
pub(crate) const NFC_IFACE: &str = "cadenza:nfc/normalize";

/// Stamp the NFC component's content address INLINE into a built value-heap component's `cadenza:nfc/normalize`
/// import, turning the bare import into the self-describing `cadenza:nfc/normalize@0.0.0+<nfc_hash>` — so a
/// runtime resolves its NFC dependency purely from the import name (zero runtime indirection; no `runtime.toml`
/// / mapping file passed to any executable — operator directive 2026-08-23). Mirrors how a program's
/// `cadenza:runtime/heap@0.0.0+<hash>` import carries the runtime address. Shells out to the
/// `cdz-component-rewrite` CLI (NOT a lib dep — operator review on #3082), which re-encodes the component's
/// import section. Applied to the RAW heap BEFORE `canonicalize_runtime` (strip -a removes any `producers`
/// the re-encode adds, so the stamped+stripped bytes are what gets hashed + stored). Returns the stamped
/// `.wasm` path (a sibling of the raw build output). `nfc_hash` is the NFC component's own content address.
pub(crate) fn stamp_nfc_into_heap(repo: &Path, raw_heap: &Path, nfc_hash: &str) -> PathBuf {
    // Build the rewriter CLI (idempotent: cargo no-ops when warm). SHELL OUT to it, never link it.
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "cdz-component-rewrite",
            "--bin",
            "cdz-component-rewrite",
        ])
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => {
            eprintln!(
                "failed to build cdz-component-rewrite ({other:?}) — needed to stamp the NFC import"
            );
            std::process::exit(1);
        }
    }
    let bin = repo.join("target/release/cdz-component-rewrite");
    let out = raw_heap.with_extension("nfc-stamped.wasm");
    let mapping = format!("{NFC_IFACE}=0.0.0+{nfc_hash}");
    let status = std::process::Command::new(&bin)
        .arg(raw_heap)
        .arg(&out)
        .arg(&mapping)
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => {
            eprintln!(
                "cdz-component-rewrite failed ({other:?}) stamping {NFC_IFACE} into {} — the heap's NFC \
                 import could not be made self-describing",
                raw_heap.display()
            );
            std::process::exit(1);
        }
    }
    out
}

/// `cargo component build --release --target wasm32-unknown-unknown` in <seed>/crates/<crate>,
/// returning the produced .wasm path. `cmd!` runs the child in the pushed crate dir and returns an
/// `Err` on a non-zero exit (already echoing the command), so a build failure surfaces cleanly.
pub(crate) fn build_component(sh: &Shell, seed: &Path, crate_dir: &str, artifact: &str) -> PathBuf {
    build_component_with_features(sh, seed, crate_dir, artifact, &[])
}

/// As [`build_component`], but with a list of cargo `--features` to enable — e.g. the runtime's
/// `debug-counters` (its `live-objects` leak counter). A feature-flagged build writes to the SAME
/// `target/.../<artifact>.wasm` path as the default, so a caller that needs BOTH must read each build's
/// bytes before the next overwrites (see `codegen`'s two-build hashing). Same crate dir, target, and
/// error handling as the default build.
pub(crate) fn build_component_with_features(
    sh: &Shell,
    seed: &Path,
    crate_dir: &str,
    artifact: &str,
    features: &[&str],
) -> PathBuf {
    let dir = seed.join("crates").join(crate_dir);
    let _pushed = sh.push_dir(&dir);
    let build = if features.is_empty() {
        cmd!(
            sh,
            "cargo component build --release --target wasm32-unknown-unknown"
        )
    } else {
        let feats = features.join(",");
        cmd!(
            sh,
            "cargo component build --release --target wasm32-unknown-unknown --features {feats}"
        )
    };
    // The runtime's `.cargo/config.toml` builds core/alloc/std from source with
    // `panic = immediate-abort` (see the comment there) so the emitted bytes — and thus the content
    // hash every program pins — are byte-identical across host architectures. `build-std` is a `-Z`
    // feature, so enable it on the stable pin with RUSTC_BOOTSTRAP. Without this the wasm build hard-
    // errors (the config's rustflags parse a nightly option that build-std would otherwise unlock).
    let build = build.env("RUSTC_BOOTSTRAP", "1");
    if let Err(e) = build.run() {
        eprintln!("cargo component build failed for {crate_dir}: {e}");
        std::process::exit(1);
    }
    dir.join(format!(
        "target/wasm32-unknown-unknown/release/{artifact}.wasm"
    ))
}

#[cfg(test)]
mod trap_grading_tests {
    use super::*;

    /// Serializes the tests that MUTATE process-global env (`CDZ_SUITE_TIMEOUT_SECS` /
    /// `CDZ_ML_PER_FILE_TIMEOUT_SECS`). `cargo test` runs this binary MULTI-THREADED, and env is
    /// process-global — so a test that `set_var`s a timeout var races a sibling that `remove_var`s it or
    /// READS it (via `suite_timeout_for`/`ml_per_file_timeout`). That flaked
    /// `ml_per_file_timeout_reads_env_with_hang_bound_default` in a gate batch (pr-sync report): its
    /// `per-file < suite` assert reads `CDZ_SUITE_TIMEOUT_SECS` while `suite_timeout_reads_env…`
    /// concurrently set it to 480, making `720 < 480` false. Every env-touching test locks this FIRST so
    /// they run mutually exclusive — no `serial_test` dep needed. (Same class as the process-global
    /// metric-counter contamination.) A poisoned lock (a panicking sibling) is recovered — the env is
    /// restored by each test's own cleanup, so a stale poison must not wedge the rest.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── collect_warnings: ALL warning diagnostics off a clean-compile stderr (inc2, operator seq353) ─
    #[test]
    fn collect_warnings_gathers_every_warning_as_a_set() {
        // A clean compile can emit MULTIPLE warnings — collect ALL, not first-wins.
        let stderr = b"cdz: warning [CDZ0306] (node 9): unused binding: `y` is never used\n\
                       cdz: warning [CDZ0306] (node 12): unused binding: `z` is never used\n\
                       cdz: warning [CDZ0213] (node 3): this match arm is unreachable";
        let ws = collect_warnings(stderr);
        assert_eq!(ws.len(), 3);
        assert_eq!(ws[0].0, "CDZ0306");
        assert!(ws[0].1.contains("unused binding"));
        assert_eq!(ws[2].0, "CDZ0213");
        assert!(ws[2].1.contains("unreachable"));
    }

    #[test]
    fn collect_warnings_is_total_and_ignores_non_warning_lines() {
        // No warnings → empty; error lines / garbage are ignored; never panics.
        assert!(collect_warnings(b"").is_empty());
        assert!(collect_warnings(b"cdz: error [CDZ0101] (node 1): unbound").is_empty());
        assert!(collect_warnings(b"random noise\nno diagnostics here").is_empty());
        // A single warning with no trailing `: ` message still yields the code + empty message.
        let ws = collect_warnings(b"cdz: warning [CDZ0305]");
        assert_eq!(ws, vec![("CDZ0305".to_string(), String::new())]);
    }

    // ── first_error_diag: code+message recovery off cdz compile stderr (portable-diagnostic-test
    //    capability, operator seq353 — the message half) ───────────────────────────────────────────
    // ── split_message_clause + grade_trial message-clause matching (the payoff: portable diagnostic
    //    MESSAGE assertions, operator seq353) ────────────────────────────────────────────────────────
    #[test]
    fn split_message_clause_extracts_code_and_phrase() {
        assert_eq!(split_message_clause("CDZ0201"), ("CDZ0201", None));
        assert_eq!(
            split_message_clause("CDZ0201 (message \"malformed record\")"),
            ("CDZ0201", Some("malformed record"))
        );
        // declines form: no leading code, just the clause.
        assert_eq!(
            split_message_clause("(message \"IEEE partial order\")"),
            ("", Some("IEEE partial order"))
        );
        // Malformed clause (unterminated / empty) → phrase not asserted, never panics.
        assert_eq!(split_message_clause("CDZ0201 (message \"").0, "CDZ0201");
        assert_eq!(
            split_message_clause("CDZ0201 (message \"\")"),
            ("CDZ0201", None)
        );
    }

    #[test]
    fn grade_error_with_message_clause_requires_the_substring() {
        let declined = |code: &str, msg: &str| Ran::Declined {
            code: Some(code.to_string()),
            message: msg.to_string(),
        };
        // CODE matches + message contains the phrase → Pass.
        assert!(matches!(
            grade_trial(
                "error CDZ0101 (message \"unbound name\")",
                &declined("CDZ0101", "unbound name `foo`")
            ),
            Grade::Pass
        ));
        // CODE matches but message MISSING the phrase → Fail (the pin caught a message drift).
        assert!(matches!(
            grade_trial(
                "error CDZ0101 (message \"did you mean\")",
                &declined("CDZ0101", "unbound name `foo`")
            ),
            Grade::Fail(_)
        ));
        // No message clause → CODE alone decides (back-compat, unchanged).
        assert!(matches!(
            grade_trial("error CDZ0101", &declined("CDZ0101", "anything")),
            Grade::Pass
        ));
        // Case-sensitive: wrong case does NOT match (case is load-bearing per v-diagnostics).
        assert!(matches!(
            grade_trial(
                "error CDZ0101 (message \"Unbound\")",
                &declined("CDZ0101", "unbound name `foo`")
            ),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn grade_declines_with_message_clause_pins_the_reason() {
        let declined_uncoded = |msg: &str| Ran::Declined {
            code: None,
            message: msg.to_string(),
        };
        // Uncoded decline whose message contains the pinned redirect phrase → Pass.
        assert!(matches!(
            grade_trial(
                "declines (message \"IEEE partial order\")",
                &declined_uncoded("cannot compare Float64 by IEEE partial order; use </<=/>/>=")
            ),
            Grade::Pass
        ));
        // Bare `declines` (no clause) still passes on any decline (back-compat, unchanged).
        assert!(matches!(
            grade_trial("declines", &declined_uncoded("whatever")),
            Grade::Pass
        ));
        // Message clause present but not contained → Fail.
        assert!(matches!(
            grade_trial(
                "declines (message \"partial order\")",
                &declined_uncoded("some unrelated decline reason")
            ),
            Grade::Fail(_)
        ));
    }

    /// Anti-vacuous guard for the `(warns …)` pin (operator seq353 inc2 + the target-aware-skip
    /// follow-up). The inc1 `(message …)` clause once graded VACUOUSLY (the reader stripped the clause,
    /// so a garbage message still passed); this fixture pins that a warns pin genuinely grades — a wrong
    /// message OR a wrong code FAILS on the wasm witness — AND that the pin is correctly SKIPPED (not
    /// failed) on rust / rust-async, where the harness cannot observe compile warnings. So warns can
    /// never silently regress to vacuous, and the target-skip can never silently regress to always-fail.
    #[test]
    fn grade_warns_pin_is_non_vacuous_on_wasm_and_skipped_off_wasm() {
        // A case: program compiled to `value 42` AND emitted a real CDZ0306 unused-binding warning.
        let record = |warn_code: &str, warn_msg: &str| CorpusRecord {
            description: "unused-binding warns fixture".to_string(),
            program: "(do (def (main) (let ((unused 99)) 42)) (export main))".to_string(),
            modules: Vec::new(),
            peers: Vec::new(),
            trials: vec![xtask_support::Trial {
                call: None,
                expect: "output (: 42 Int64)".to_string(),
            }],
            needs: Vec::new(),
            host_responses: Vec::new(),
            host_calls: Vec::new(),
            warns: vec![(warn_code.to_string(), Some(warn_msg.to_string()))],
            wit_world: None,
            component_name: None,
            live_objects: None,
            live_objects_known_leak: false,
        };
        // The run: a value 42 whose compile emitted `unused binding `unused``.
        let ran = vec![Ran::Value(
            "(: 42 Int64)".to_string(),
            Vec::new(),
            vec![("CDZ0306".to_string(), "unused binding `unused`".to_string())],
        )];

        // WASM (the witness): correct code + contained message → Pass.
        assert!(matches!(
            grade_ran(&record("CDZ0306", "unused binding"), &ran, GateTarget::Wasm),
            Grade::Pass
        ));
        // WASM: garbage MESSAGE → Fail (non-vacuous — a wrong phrase is caught).
        assert!(matches!(
            grade_ran(&record("CDZ0306", "TOTAL GARBAGE"), &ran, GateTarget::Wasm),
            Grade::Fail(_)
        ));
        // WASM: garbage CODE → Fail (non-vacuous — a wrong code is caught).
        assert!(matches!(
            grade_ran(&record("CDZ9999", "unused binding"), &ran, GateTarget::Wasm),
            Grade::Fail(_)
        ));
        // RUST / RUST-ASYNC: the warns pin is SKIPPED — even a garbage message Passes (the value 42
        // still matches, and warns is not asserted off the wasm witness). This is the observability-gap
        // skip, NOT a coverage loss: the warning provably fired (it is compile.rs-emitted), the harness
        // just cannot see it here. If this ever FAILS, the target-skip regressed to grading off-wasm.
        assert!(matches!(
            grade_ran(&record("CDZ9999", "TOTAL GARBAGE"), &ran, GateTarget::Rust),
            Grade::Pass
        ));
        assert!(matches!(
            grade_ran(
                &record("CDZ9999", "TOTAL GARBAGE"),
                &ran,
                GateTarget::RustAsync
            ),
            Grade::Pass
        ));
    }

    #[test]
    fn first_error_diag_reads_code_and_message_from_a_coded_reject() {
        // The shape verified on trunk: `cdz: error [CDZ0101] (node 4): unbound name `nope``.
        let (code, msg) = first_error_diag(b"cdz: error [CDZ0101] (node 4): unbound name `nope`");
        assert_eq!(code.as_deref(), Some("CDZ0101"));
        assert_eq!(msg, "unbound name `nope`");
    }

    #[test]
    fn first_error_diag_reads_an_uncoded_decline_message() {
        // A codeless decline (prose redirect) — no `[CODE]`, message after `error: `.
        let (code, msg) = first_error_diag(
            b"cdz: error: cannot compare Float64 by IEEE partial order; use </<=/>/>=",
        );
        assert_eq!(code, None);
        assert!(msg.contains("IEEE partial order"));
    }

    #[test]
    fn first_error_diag_is_consistent_and_total() {
        // A coded line with no ` : ` message still yields the code + an empty message (no panic).
        let (code, msg) = first_error_diag(b"cdz: error [CDZ0303]");
        assert_eq!(code.as_deref(), Some("CDZ0303"));
        assert_eq!(msg, "");
        // No error line at all → (None, "") — never panics.
        assert_eq!(
            first_error_diag(b"warning: unused\nplain noise"),
            (None, String::new())
        );
        assert_eq!(first_error_diag(b""), (None, String::new()));
    }

    #[test]
    fn needs_clause_lint_flags_only_leading_needs_clauses() {
        // A retired `(needs …)` clause is caught wherever it opens a line (leading whitespace ok),
        // reported by 1-based line number — the corpus's exact shape (`  (needs  sum-type-declaration)`).
        let corpus = "(case\n  (program …)\n  (needs  sum-type-declaration)\n  (output 1))\n";
        assert_eq!(needs_clause_lines(corpus), vec![3]);

        // A needs-free case yields nothing (the post-strip / steady state).
        let clean = "(case\n  (program …)\n  (output 1))\n";
        assert!(needs_clause_lines(clean).is_empty());

        // Only a LEADING `(needs` token counts — a `needs` mentioned mid-line or as prose (e.g. a
        // comment / a `(def needs …)`) must NOT false-positive and redden an innocent corpus edit.
        let innocent =
            "; this generation needs nothing\n(def (needs-check x) x)\n  (call needs 1)\n";
        assert!(needs_clause_lines(innocent).is_empty());

        // Multiple clauses across a file are all reported, in order.
        let many = "(needs a)\nx\n  (needs b)\n";
        assert_eq!(needs_clause_lines(many), vec![1, 3]);
    }

    #[test]
    fn fast_gate_remote_transient_matches_the_nix_signature_not_ordinary_clippy_output() {
        // A nix daemon/remote-builder signature → transient (a gate re-run is warranted), not a code failure.
        assert!(fast_gate_output_is_remote_transient(
            "cargo-clippy-rcdzc-clippy> Checking cadenza-ast v0.0.0\nerror: Invalid BuildResult status from remote"
        ));
        assert!(fast_gate_output_is_remote_transient(
            "error: build failure on remote 'ssh://builder'"
        ));
        // The daemon-connection-reset family (same instability as the CI resets) → transient too.
        assert!(fast_gate_output_is_remote_transient(
            "error: cannot open connection to remote store 'daemon': read of 32768 bytes: Connection reset by peer"
        ));
        // A bare "Connection reset by peer" WITHOUT the store-connection phrase must NOT trip it.
        assert!(!fast_gate_output_is_remote_transient(
            "test tcp_client ... FAILED\n  assertion failed: Connection reset by peer"
        ));
        // A REAL clippy/test failure must NOT be misread as a transient.
        assert!(!fast_gate_output_is_remote_transient(
            "error[E0308]: mismatched types\n  --> src/lib.rs:10:5"
        ));
        assert!(!fast_gate_output_is_remote_transient(
            "error: this lint expectation is unfulfilled"
        ));
        // The word "remote" alone (a doc comment / git-remote mention) must not trip it.
        assert!(!fast_gate_output_is_remote_transient(
            "note: the remote branch is ahead; a clippy warning about remote_data follows"
        ));
        assert!(!fast_gate_output_is_remote_transient(""));
    }

    #[test]
    fn fast_gate_contention_kill_matches_a_killed_builder_not_a_real_or_crashing_failure() {
        // A sub-check builder SIGKILLed (137 = 128+9) / SIGTERMed (143 = 128+15) under load → contention-kill,
        // NOT a regression → RE-RUN. This is the false-HOLD class: no remote-transient signature, so the old
        // advisory called it REAL.
        assert!(fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-oracle.drv' failed with exit code 137"
        ));
        assert!(fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-corpus.drv' failed with exit code 143"
        ));
        assert!(fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-wasm-runtime-build.drv' failed due to signal 9 (SIGKILL)"
        ));
        assert!(fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-oracle.drv' failed due to signal 15 (SIGTERM)"
        ));
        // A CRASH signal (SIGSEGV 11 / SIGABRT 6) is a REAL failure to ROUTE, not a contention-kill.
        assert!(!fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-oracle.drv' failed due to signal 11 (SIGSEGV)"
        ));
        assert!(!fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-oracle.drv' failed due to signal 6 (SIGABRT)"
        ));
        // An ordinary non-kill build failure (exit 1 / a real code error) must NOT trip it.
        assert!(!fast_gate_output_is_contention_kill(
            "error: builder for '/nix/store/abc-oracle.drv' failed with exit code 1"
        ));
        assert!(!fast_gate_output_is_contention_kill(
            "error[E0308]: mismatched types\n  --> src/lib.rs:10:5"
        ));
        // A stray "137" NOT anchored to nix's builder-failure wording must not trip it.
        assert!(!fast_gate_output_is_contention_kill(
            "note: 137 tests passed; exit code 0"
        ));
        assert!(!fast_gate_output_is_contention_kill(""));
    }

    #[test]
    fn nix_run_external_argv_forwards_cmd_and_args_to_the_flake_app() {
        // A bare unrecognized subcommand → `nix run <flake>#<cmd>` (no trailing `--`).
        assert_eq!(
            nix_run_external_argv(&["lint-mandates".to_string()], "/w"),
            vec!["run".to_string(), "/w#lint-mandates".to_string()]
        );
        // With args → `nix run <flake>#<cmd> -- <args…>` (args forwarded verbatim after the `--`).
        assert_eq!(
            nix_run_external_argv(
                &[
                    "gate".to_string(),
                    "--files".to_string(),
                    "x.sexp".to_string()
                ],
                "/w"
            ),
            vec![
                "run".to_string(),
                "/w#gate".to_string(),
                "--".to_string(),
                "--files".to_string(),
                "x.sexp".to_string()
            ]
        );
    }

    #[test]
    fn bytelen_scalar_walk_warns_on_same_string_and_excludes_bytes_and_output() {
        // Tier-2 heuristic (concierge ruling C part 2): flag iff a case measures a string with
        // String.byte-len AND scalar-indexes the SAME string with String.at/slice. Validated against the
        // fixture shapes corpus-bugfix handed (3 should-WARN + 2 should-NOT-warn exclusions).

        // should-WARN: byte-len(s) bounds a String.at(s) walk on the same s.
        let parse_int = "(go s 0 (String.byte-len s) 0) (match (String.at s i) …)";
        assert_eq!(bytelen_scalar_walk_idents(parse_int), vec!["s".to_string()]);

        // should-WARN: la = byte-len(a), then String.at(a …) — same string a via a let-bound measure.
        let lev = "(def la (String.byte-len a)) (match (String.at a (- i 1)) …)";
        assert_eq!(bytelen_scalar_walk_idents(lev), vec!["a".to_string()]);

        // should-WARN: String.slice(s …) bounded by byte-len(s).
        let slice = "(go s 0 (String.byte-len s) 0) (match (String.slice s i (+ i 1)) …)";
        assert_eq!(bytelen_scalar_walk_idents(slice), vec!["s".to_string()]);

        // should-NOT-warn (a): a Bytes walk (Bytes.at/Bytes.len) — no String.byte-len match at all.
        let bytes = "(go b 0 (Bytes.len b) 0) (match (Bytes.at b i) …)";
        assert!(bytelen_scalar_walk_idents(bytes).is_empty());

        // should-NOT-warn (b): byte-len as pure OUTPUT — measured, never String.at-indexed.
        let output = "(def (main n) (String.byte-len (build n \"\")))";
        assert!(bytelen_scalar_walk_idents(output).is_empty());

        // should-NOT-warn: byte-len(x) and String.at(y) on DIFFERENT strings — no same-x co-occurrence.
        let diff = "(String.byte-len x) (String.at y i)";
        assert!(bytelen_scalar_walk_idents(diff).is_empty());

        // should-NOT-warn (PR#836 regression): a SIGNED numeric first-arg must not be misclassified as an
        // identifier. `(String.at -1 …)` has a numeric receiver (nonsense to warn on), and it isn't the
        // same "string" as the byte-len'd s, so the co-occurrence must stay empty — before the fix `-1`
        // was collected as an ident, and `(String.byte-len -1)`/`(String.at -1)` would spuriously match.
        let signed = "(String.byte-len -1) (String.at -1 i)";
        assert!(
            bytelen_scalar_walk_idents(signed).is_empty(),
            "a signed numeric literal must not be collected as a string identifier"
        );
    }

    #[test]
    fn token_is_numeric_literal_covers_unsigned_and_signed_and_excludes_bare_signs() {
        // Unsigned digits.
        assert!(token_is_numeric_literal("0"));
        assert!(token_is_numeric_literal("42"));
        // Signed literals — the PR#836 case the first-char-digit-only check missed.
        assert!(token_is_numeric_literal("-1"));
        assert!(token_is_numeric_literal("+1"));
        assert!(token_is_numeric_literal("-255"));
        // A bare sign (an operator head like `-`/`+`, not a number) is NOT a numeric literal — it must
        // stay a non-match so a real operator token isn't silently swallowed by the numeric skip.
        assert!(!token_is_numeric_literal("-"));
        assert!(!token_is_numeric_literal("+"));
        // Ordinary identifiers.
        assert!(!token_is_numeric_literal("s"));
        assert!(!token_is_numeric_literal("string-var"));
        assert!(!token_is_numeric_literal(""));
    }

    #[test]
    fn split_corpus_cases_scopes_each_case_block() {
        // The lint must scope co-occurrence WITHIN one case: a byte-len in case A + a String.at in case B
        // must NOT combine. split_corpus_cases yields one (title, block) per `(case ` opener.
        let corpus = "; preamble comment\n(case \"first\"\n  (input (String.byte-len s)))\n(case \"second\"\n  (input (String.at s i)))\n";
        let cases = split_corpus_cases(corpus);
        assert_eq!(cases.len(), 2, "two cases");
        assert_eq!(cases[0].0, "first");
        assert_eq!(cases[1].0, "second");
        // Each block sees only its own text → neither alone triggers the co-occurrence (byte-len in one,
        // String.at in the other), so per-case scanning stays quiet — the cross-case false-positive is avoided.
        assert!(bytelen_scalar_walk_idents(&cases[0].1).is_empty());
        assert!(bytelen_scalar_walk_idents(&cases[1].1).is_empty());
    }

    #[test]
    fn needs_free_lint_fails_loudly_when_it_cannot_enumerate_inputs() {
        // A lint that can't read its inputs must FAIL, not vacuously pass (PR#519 Copilot): an
        // unreadable/absent corpus dir or an empty one would otherwise green the whole `check` having
        // scanned nothing, hiding a re-introduced clause. `paths` points at a repo whose
        // `spec/semantics` does not exist → read_dir errors → hard Err.
        let tick = std::process::id();
        let root = std::env::temp_dir().join(format!("needs-lint-empty-{tick}"));
        // Pre-clear: the dir is keyed only on pid, so a prior run that panicked before its cleanup
        // could leave stale contents that contaminate the empty-dir assertion below. Start clean.
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths {
            seed: root.join("implementation/seed"),
            repo: root.clone(),
        };
        let err = needs_free_lint(&paths).expect_err("missing corpus dir must be a hard error");
        assert!(err.contains("cannot read corpus dir"), "got: {err}");

        // An EXISTING but empty spec/semantics (0 *.sexp) is also a hard error, not a vacuous pass.
        std::fs::create_dir_all(root.join("spec/semantics")).unwrap();
        let err = needs_free_lint(&paths).expect_err("empty corpus dir must be a hard error");
        assert!(err.contains("no *.sexp corpus files"), "got: {err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn baseline_titles_extracts_descriptions_ignoring_verdict_and_header() {
        // Descriptions come from the `verdict\tdescription` lines; `#` header + blanks are skipped, and
        // the VERDICT is dropped (only the case-title SET matters — verdicts differ per backend).
        let text =
            "# gate baseline\npass\ta case that passes\ntodo\ta case the backend declines\n\n";
        let titles = baseline_titles(text);
        assert!(titles.contains("a case that passes"));
        assert!(titles.contains("a case the backend declines"));
        assert_eq!(titles.len(), 2);
    }

    #[test]
    fn gate_check_fails_on_any_fail_not_just_a_pass_regression() {
        // v-nix gate hole: `--check` only flagged a baseline pass→fail regression, so a case ABSENT from
        // the baseline (or a baselined todo) that FAILS slipped past as "not a regression" — making the
        // fleet landing bar weaker than plain `gate`. A fail is never an accepted baseline state, so
        // `--check` must exit non-zero on ANY current fail.
        let tick = std::process::id();
        let root = std::env::temp_dir().join(format!("gate-check-fail-{tick}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("spec/semantics")).unwrap();
        std::fs::write(
            root.join("spec/semantics/.gate-baseline"),
            "# gate baseline\npass\tbaselined pass\ntodo\tbaselined todo\n",
        )
        .unwrap();
        let paths = Paths {
            seed: root.join("implementation/seed"),
            repo: root.clone(),
        };
        let v = |pairs: &[(&str, Verdict)]| -> Vec<(String, Verdict)> {
            pairs.iter().map(|(d, x)| (d.to_string(), *x)).collect()
        };

        // An UNBASELINED case that fails → non-zero (the hole this closes; both baselined cases hold).
        assert_ne!(
            check_baseline(
                &paths,
                &v(&[
                    ("baselined pass", Verdict::Pass),
                    ("baselined todo", Verdict::Todo),
                    ("brand new migrated case", Verdict::Fail),
                ]),
                GateTarget::Wasm,
                false,
            ),
            0,
            "an unbaselined fail must fail --check"
        );
        // A baselined todo→fail (a declined case now miscompiling) must ALSO fail.
        assert_ne!(
            check_baseline(
                &paths,
                &v(&[
                    ("baselined pass", Verdict::Pass),
                    ("baselined todo", Verdict::Fail),
                ]),
                GateTarget::Wasm,
                false,
            ),
            0,
            "a baselined todo→fail must fail --check"
        );
        // Control: no fails, both baselined cases hold → OK (exit 0).
        assert_eq!(
            check_baseline(
                &paths,
                &v(&[
                    ("baselined pass", Verdict::Pass),
                    ("baselined todo", Verdict::Todo),
                ]),
                GateTarget::Wasm,
                false,
            ),
            0,
            "no fails + no regressions → --check passes"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn conflicting_dup_titles_flags_only_verdict_conflicts_not_benign_repeats() {
        // The within-file dup lint is VERDICT-AWARE (v-wasm-opt trunk-RED fix 2026-08-10): only a
        // same-title/DIFFERENT-verdict dup is dangerous (the map-keyed baseline masks one via last-wins).
        // A benign same-verdict repeat — the routine `merge=union` artifact — must be TOLERATED, matching
        // `gate --check`'s own benign-dedup, else a union-merge dup reds `cargo xtask check` fleet-wide.
        // Conflict (pass vs todo on the same title) → flagged, once, in first-conflict order.
        assert_eq!(
            conflicting_dup_titles(
                "# gate baseline\npass\tdup case\ntodo\tdup case\npass\tunique case\n"
            ),
            vec!["dup case".to_string()]
        );
        // BENIGN same-verdict repeat (the merge=union artifact — the 506-dup trunk RED) → NOT flagged.
        assert!(
            conflicting_dup_titles("pass\tsame case\npass\tsame case\npass\tother\n").is_empty(),
            "a same-verdict duplicate is a harmless merge=union artifact, tolerated like `gate --check`"
        );
        // A three-way with a conflict buried among benign repeats → still caught, reported once.
        assert_eq!(
            conflicting_dup_titles("pass\tc\npass\tc\ntodo\tc\n"),
            vec!["c".to_string()],
            "a benign repeat followed by a conflicting verdict on the same title is a conflict"
        );
        // The set form (agree-lint's) still collapses a repeated title to one entry — unchanged behavior.
        assert_eq!(
            baseline_titles("# gate baseline\npass\tdup case\ntodo\tdup case\npass\tunique case\n")
                .len(),
            2,
            "the set form collapses the dup (the agree-lint compares SETS across files)"
        );
    }

    #[test]
    fn baseline_titles_agree_lint_flags_a_wasm_only_stale_rust_baseline() {
        // Simulate the recurring bug: wasm baseline has a NEW case the rust baselines lack (a wasm-only
        // `gate --save`). The lint must flag rust + rust-async as missing that title, with the heal cmd.
        let tick = std::process::id();
        let root = std::env::temp_dir().join(format!("baseline-agree-{tick}"));
        // Pre-clear: pid-keyed dir, so a prior panicked run's leftovers could skew the title-set diff.
        let _ = std::fs::remove_dir_all(&root);
        let sem = root.join("spec/semantics");
        std::fs::create_dir_all(&sem).unwrap();
        let paths = Paths {
            seed: root.join("implementation/seed"),
            repo: root.clone(),
        };
        // wasm has the extra title; rust + async share the old set only.
        std::fs::write(
            sem.join(".gate-baseline"),
            "pass\tshared case\npass\tNEW wasm-only case\n",
        )
        .unwrap();
        std::fs::write(sem.join(".gate-baseline-rust"), "todo\tshared case\n").unwrap();
        std::fs::write(sem.join(".gate-baseline-rust-async"), "todo\tshared case\n").unwrap();
        let err = baseline_titles_agree_lint(&paths).expect_err("a title-set divergence must fail");
        assert!(err.contains("NEW wasm-only case"), "got: {err}");
        assert!(
            err.contains("gate --save --target rust"),
            "heal cmd named: {err}"
        );

        // Now make all three agree (verdicts still differ) → passes: the SET matches, verdicts are free.
        std::fs::write(
            sem.join(".gate-baseline-rust"),
            "todo\tshared case\ntodo\tNEW wasm-only case\n",
        )
        .unwrap();
        std::fs::write(
            sem.join(".gate-baseline-rust-async"),
            "pass\tshared case\ntodo\tNEW wasm-only case\n",
        )
        .unwrap();
        assert!(
            baseline_titles_agree_lint(&paths).is_ok(),
            "agreeing title-sets with differing verdicts must pass"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn baseline_titles_agree_lint_skips_absent_but_fails_on_a_read_error() {
        // PR#522: a MISSING baseline (NotFound) is legitimately skipped (rust baselines are opt-in),
        // but a real read ERROR must FAIL LOUDLY rather than be silently treated as absent.
        let tick = std::process::id();
        let root = std::env::temp_dir().join(format!("baseline-readerr-{tick}"));
        let _ = std::fs::remove_dir_all(&root);
        let sem = root.join("spec/semantics");
        std::fs::create_dir_all(&sem).unwrap();
        let paths = Paths {
            seed: root.join("implementation/seed"),
            repo: root.clone(),
        };

        // Only the wasm baseline exists; rust + async are ABSENT (NotFound → skipped). With <2 present
        // there's nothing to compare → Ok (a missing opt-in baseline is not a divergence).
        std::fs::write(sem.join(".gate-baseline"), "pass\tsome case\n").unwrap();
        assert!(
            baseline_titles_agree_lint(&paths).is_ok(),
            "a single present baseline (others NotFound) must pass, not error"
        );

        // Now make the rust baseline UNREADABLE-as-UTF-8 (invalid bytes) — a real read error, NOT
        // absence. The lint must FAIL loudly, not silently skip it as if missing.
        std::fs::write(sem.join(".gate-baseline-rust"), [0xff, 0xfe, 0x00]).unwrap();
        let err = baseline_titles_agree_lint(&paths).expect_err(
            "an unreadable (non-UTF-8) baseline must be a hard error, not a silent skip",
        );
        assert!(err.contains("cannot read baseline"), "got: {err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sh_syntax_errors_flags_only_the_unparseable_script() {
        // The fleet-scripts-syntax gate: `bash -n` catches a syntax break in window.sh / the hygiene
        // scripts before it ships. Skip if bash isn't on this host (the lint is fail-soft there anyway).
        if std::process::Command::new("bash")
            .arg("-n")
            .arg("/dev/null")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("cdz-shlint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.sh");
        let bad = dir.join("bad.sh");
        std::fs::write(&good, "#!/usr/bin/env bash\nif true; then echo ok; fi\n").unwrap();
        // Missing `fi` — a parse error `bash -n` must catch.
        std::fs::write(&bad, "#!/usr/bin/env bash\nif true; then echo oops\n").unwrap();
        let flagged = sh_syntax_errors(&[good.clone(), bad.clone()]);
        assert_eq!(flagged.len(), 1, "only the unparseable script is flagged");
        assert!(
            flagged[0].contains("bad.sh"),
            "the flagged entry names the offending script"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gate_inprocess_advisory_reason_and_cached_attr_mapping() {
        // Reason arms, in precedence order (caller has already excluded --save/--check/--shard).
        assert!(gate_inprocess_reason(true, false).contains("--case"));
        assert!(gate_inprocess_reason(false, true).contains("CDZ_GATE_INPROCESS"));
        assert!(gate_inprocess_reason(false, false).contains("unavailable"));
        // --case wins even when the env is also set (structural reason first).
        assert!(gate_inprocess_reason(true, true).contains("--case"));

        // The cached-attr hint: NN-feature files map to a per-file check for all 3 corpus targets;
        // non-NN files do not.
        assert_eq!(
            corpus_check_attr(GateTarget::Wasm, Some("13-strings")).as_deref(),
            Some("corpus-13-strings")
        );
        assert_eq!(
            corpus_check_attr(GateTarget::Rust, Some("05-compound-types")).as_deref(),
            Some("corpus-rust-05-compound-types")
        );
        assert_eq!(
            corpus_check_attr(GateTarget::RustAsync, Some("14c-effects-and-handlers")).as_deref(),
            Some("corpus-rust-async-14c-effects-and-handlers")
        );
        // A non-`NN-feature` stem (e.g. a scratch file) has no cached check → in-process is the only path.
        assert_eq!(
            corpus_check_attr(GateTarget::Wasm, Some("zz-scratch")),
            None
        );
    }

    #[test]
    fn run_failure_diagnostic_distinguishes_crash_from_clean_rejection() {
        // A SILENT-DEATH crash: stderr is ONLY the informational provenance banner (+ host traces) → no
        // diagnostic → classified as "run died without output" (BadArtifact). breaker's B1-sibling.
        assert!(!run_failure_has_diagnostic(
            "cdz: live-objects run on value-heap runtime abc123 (--runtime override)"
        ));
        assert!(!run_failure_has_diagnostic(
            "cdz: live-objects run on value-heap runtime abc123\nhost-call\tprint\nhost-arg\tprint\thi"
        ));
        assert!(!run_failure_has_diagnostic(""));
        // A clean COMPOSE/INSTANTIATE rejection or run error emits a STRUCTURED diagnostic → keep as the trap
        // reason (the 5 peer-compose-reject corpus cases pin this; must NOT be reclassified).
        assert!(run_failure_has_diagnostic(
            "cdz-run: peer `cadenza:math/api` op `add` signature mismatch: ..."
        ));
        assert!(run_failure_has_diagnostic(
            "cdz: live-objects run on value-heap runtime abc123\ncdz-run: peer `m/api` does not export op `sub`"
        ));
    }

    #[test]
    fn opt_sweep_outcome_key_distinguishes_observably_different_runs() {
        // The opt-sweep's blocking guarantee rests on `sweep_outcome_key`: two runs are level-equivalent
        // iff their keys are EQUAL. So the key MUST separate every observably-different outcome — a value
        // change, a value-vs-trap, a trap-KIND change, a decline-code change — else a real cross-level
        // divergence (an unsound optimization) would compare equal and slip past the gate.
        let val = |v: &str| Ran::Value(v.to_string(), vec![], vec![]);
        // Distinct values → distinct keys (a level that changed the computed value is caught).
        assert_ne!(sweep_outcome_key(&val("42")), sweep_outcome_key(&val("43")));
        // Same value → same key (a level that only reshuffled emit bytes is NOT a divergence).
        assert_eq!(sweep_outcome_key(&val("42")), sweep_outcome_key(&val("42")));
        // Value vs trap → distinct (a level that turned a result into a trap, or vice versa, is caught).
        assert_ne!(
            sweep_outcome_key(&val("0")),
            sweep_outcome_key(&Ran::Trap("divide by zero".into()))
        );
        // Distinct trap KINDS → distinct keys.
        assert_ne!(
            sweep_outcome_key(&Ran::Trap("divide by zero".into())),
            sweep_outcome_key(&Ran::Trap("integer overflow".into()))
        );
        // Distinct decline CODES → distinct keys.
        assert_ne!(
            sweep_outcome_key(&Ran::Declined {
                code: Some("CDZ0302".into()),
                message: String::new(),
            }),
            sweep_outcome_key(&Ran::Declined {
                code: Some("CDZ0304".into()),
                message: String::new(),
            })
        );
    }

    #[test]
    fn opt_sweep_outcome_key_separates_distinct_unclassified_traps() {
        // The review-fix-3 invariant (PR #529 Copilot): an UNCLASSIFIED trap reason must NOT collapse to a
        // single "other" bucket, or two genuinely-different unclassified traps would compare EQUAL and a
        // real cross-level trap divergence among them would be MISSED. Both reasons are `classify` = None,
        // so the key falls back to the raw first line and they must differ.
        assert_eq!(classify("novel host failure A"), None);
        assert_eq!(classify("novel host failure B"), None);
        assert_ne!(
            sweep_outcome_key(&Ran::Trap("novel host failure A".into())),
            sweep_outcome_key(&Ran::Trap("novel host failure B".into())),
            "distinct unclassified traps must not collapse to equal keys"
        );
        // A multi-line trap keys on its FIRST line only (stable across incidental trailing noise).
        assert_eq!(
            sweep_outcome_key(&Ran::Trap("novel host failure A\n  at frame 3".into())),
            sweep_outcome_key(&Ran::Trap("novel host failure A\n  at frame 7".into()))
        );
    }

    #[test]
    fn grade_trial_trap_arm_matches_by_reason() {
        // A trapping run whose reason matches the corpus reason → Pass.
        assert!(matches!(
            grade_trial(
                "trap divide by zero",
                &Ran::Trap("cdz-run: trap: wasm trap: integer divide by zero: bt".to_string())
            ),
            Grade::Pass
        ));
        // Trapped, and BOTH reasons classify to KNOWN (but DIFFERENT) trap codes → Fail — a wrong-trap-kind
        // disagreement is a hard signal, NOT a hidden Todo (corpus-grade #4469 / breaker grading-gap; the
        // semantic CDZ07xx codes make a kind mismatch a real disagreement). `index out of bounds` and
        // `integer overflow` are both classified kinds, so a mismatch between them Fails like a wrong value.
        assert!(matches!(
            grade_trial(
                "trap index out of bounds",
                &Ran::Trap("cdz-run: trap: wasm trap: integer overflow: bt".to_string())
            ),
            Grade::Fail(_)
        ));
        // Trapped, but the ACTUAL reason does NOT classify (a novel/unknown trap) → Todo, never a false
        // Pass: a real trap fired but it can't be confirmed to be the SAME kind the corpus pinned. (This is
        // the `_` arm that survives #4469 — only a mismatch between two KNOWN codes is a Fail.)
        assert!(matches!(
            grade_trial(
                "trap divide by zero",
                &Ran::Trap(
                    "cdz-run: trap: wasm trap: some novel unclassified failure: bt".to_string()
                )
            ),
            Grade::Todo
        ));
        // A program the corpus says traps that instead ran to a value → Fail (the miscompile signal).
        assert!(matches!(
            grade_trial(
                "trap divide by zero",
                &Ran::Value("5".to_string(), vec![], vec![])
            ),
            Grade::Fail(_)
        ));
        // A compile-time rejection (the overflow caught as CDZ0302 before running) → Todo, not Fail.
        assert!(matches!(
            grade_trial(
                "trap integer overflow",
                &Ran::Declined {
                    code: Some("CDZ0302".to_string()),
                    message: String::new(),
                }
            ),
            Grade::Todo
        ));
    }

    #[test]
    fn grade_trial_output_arm_uses_the_shared_structural_value_canon() {
        // The Output arm now single-sources `cdz_corpus_grade::canonical_output_value` (SLICE 1) — this
        // pins that fold on the AUTHORITATIVE gate --check path (the #7273-divergence fix). Both the
        // expected payload and the run value are canon'd + compared, so annotation + rendering variance
        // normalize away.
        //
        // Bare value matches the bare-value expectation → Pass.
        assert!(matches!(
            grade_trial(
                "output (: 42 I64)",
                &Ran::Value("42".to_string(), vec![], vec![])
            ),
            Grade::Pass
        ));
        // STRUCTURAL equivalence: the run value crosses as the FULL `(: v T)` form (the wasm ABI escape),
        // the expected payload is the same form — canon extracts the value subtree on BOTH sides → Pass.
        assert!(matches!(
            grade_trial(
                "output (: 42 I64)",
                &Ran::Value("(: 42 I64)".to_string(), vec![], vec![])
            ),
            Grade::Pass
        ));
        // A genuine value mismatch → Fail (the miscompile signal), not a false Pass.
        assert!(matches!(
            grade_trial(
                "output (: 42 I64)",
                &Ran::Value("43".to_string(), vec![], vec![])
            ),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn recursive_sum_render_terminates_via_a_helper_fn() {
        // The rust-gate value renderer is TYPE-DIRECTED. A RECURSIVE user sum's type unfolds infinitely
        // (`IntList = Cons(Tuple Int64 IntList) | Nil` → Cons → Tuple → IntList → …), so INLINING the render
        // per payload never terminates — the codegen itself diverges (a native stack overflow building the
        // render expression, which aborted the whole rust gate). The fix routes a user sum through a
        // generated recursive helper `fn`, so the recursion is at Rust RUNTIME over the finite value. This
        // test pins that `cdz_render_expr` TERMINATES on the recursive sum (a regression here reappears as a
        // hang/overflow, not a wrong string) and emits the helper + a call, not an infinite inline.
        let mut sums = std::collections::HashMap::new();
        sums.insert(
            "IntList".to_string(),
            vec![
                (
                    "Cons".to_string(),
                    vec!["(Tuple Int64 IntList)".to_string()],
                ),
                ("Nil".to_string(), Vec::new()),
            ],
        );
        let expr = cdz_render_expr(
            "IntList",
            &sums,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        // A recursive helper `fn __render_IntList` is generated and the value is rendered by CALLING it —
        // the self-referential `IntList` payload position becomes a recursive call, not another inline match.
        assert!(
            expr.contains("fn __render_IntList("),
            "a recursive sum must render via a generated helper fn: {expr}"
        );
        assert!(
            expr.contains("__render_IntList(&"),
            "the render must CALL the helper (runtime recursion over the finite value): {expr}"
        );
        // The helper's Cons arm recurses through the tuple to the nested IntList via the SAME helper.
        assert!(
            expr.matches("__render_IntList").count() >= 2,
            "the recursive payload position must re-enter the helper: {expr}"
        );

        // A NON-recursive user sum still renders (an inline call to its own helper, no infinite unfold).
        let mut mono = std::collections::HashMap::new();
        mono.insert(
            "Sign".to_string(),
            vec![
                ("Neg".to_string(), Vec::new()),
                ("Zero".to_string(), Vec::new()),
                ("Pos".to_string(), Vec::new()),
            ],
        );
        let s = cdz_render_expr(
            "Sign",
            &mono,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            s.contains("fn __render_Sign(") && s.contains("(Pos unit)"),
            "{s}"
        );
    }

    #[test]
    fn record_type_renders_structurally_not_via_display() {
        // A record TYPE's `render_name` is CAPITALIZED `(Record …)` (a1c9bc09). The renderer must match that
        // capitalized head and render the record STRUCTURALLY — a `format!("(record (a {}) (b {}))", …)`
        // reading each field positionally — NOT fall through to the scalar `format!("{}", __r)`, which would
        // ask the emitted Rust tuple `(i64, i64)` (a record erases to a positional tuple) to `Display` and
        // fail rustc E0277. (Regression: the head was matched lowercase `record`, which stopped matching a
        // `(Record …)` note after a1c9bc09, failing every record-escape case on the rust gate.)
        let sums = std::collections::HashMap::new();
        let expr = cdz_render_expr(
            "(Record (a Int64) (b Int64))",
            &sums,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            expr.contains("#record((") && expr.contains("(__r).0") && expr.contains("(__r).1"),
            "a record type must render structurally by field position, not Display the whole tuple: {expr}"
        );
        assert!(
            !expr.trim_start().starts_with("format!(\"{}\""),
            "must not fall through to the scalar Display path: {expr}"
        );
        // A record whose field is itself a tuple composes — the inner `(Tuple …)` renders `(tuple …)`.
        let nested = cdz_render_expr(
            "(Record (x Int64) (y (Tuple Int64 Int64)))",
            &sums,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            nested.contains("#record((") && nested.contains("#tuple("),
            "a record field that is a tuple renders structurally: {nested}"
        );
    }

    #[test]
    fn newtype_type_renders_by_its_erased_inner() {
        // An erased NEWTYPE's `render_name` is the bare name `Pt`; its runtime value IS the inner type (the
        // tag erased). The renderer must resolve `Pt` through its `cdz-newtype` descriptor to the inner type
        // and render STRUCTURALLY — NOT fall to the scalar `Display` of the erased Rust tuple `(i64,i64)`
        // (rustc E0277). A newtype over a compound renders the bare compound value (no `Pt` wrapper).
        let sums = std::collections::HashMap::new();
        let mut newtypes = std::collections::HashMap::new();
        newtypes.insert("Pt".to_string(), "(Tuple Int64 Int64)".to_string());
        let expr = cdz_render_expr(
            "Pt",
            &sums,
            &newtypes,
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            expr.contains("#tuple(") && expr.contains("(__r).0") && expr.contains("(__r).1"),
            "a newtype over a tuple renders the bare inner tuple, not Display: {expr}"
        );
        assert!(
            !expr.trim_start().starts_with("format!(\"{}\""),
            "must not fall through to the scalar Display path: {expr}"
        );
        // A newtype over a SCALAR resolves to that scalar (Display is correct for an Int64).
        let mut nt2 = std::collections::HashMap::new();
        nt2.insert("UserId".to_string(), "Int64".to_string());
        let s = cdz_render_expr(
            "UserId",
            &sums,
            &nt2,
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            s.contains("format!(\"{}\""),
            "a newtype over Int64 renders the scalar: {s}"
        );
    }

    #[test]
    fn empty_tuple_type_renders_as_the_literal_tuple() {
        // An EMPTY tuple `(Tuple)` — a variant's explicit empty-tuple payload (type `(Tuple)`, distinct from
        // `Unit`) — must render the literal `(tuple)`, NOT fall through to a scalar `Display` of the erased
        // Rust `()` (which does not implement `Display` → rustc E0277). Two bugs made it fall through:
        // `parse_head_type("(Tuple)", "Tuple")` returned `None` (its whitespace-after-head guard rejected the
        // empty exact match), and even matched it would `format!("(tuple )", …)` with a trailing space.
        let sums = std::collections::HashMap::new();
        let nt = std::collections::HashMap::new();
        assert_eq!(
            parse_head_type("(Tuple)", "Tuple").as_deref(),
            Some(&[][..]),
            "an empty (Tuple) parses as a zero-arg Tuple head"
        );
        let expr = cdz_render_expr(
            "(Tuple)",
            &sums,
            &nt,
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            expr, "\"#tuple()\".to_string()",
            "an empty tuple renders the literal `#tuple()`, no path read, no trailing space: {expr}"
        );
        // The whole-token guard still rejects a longer head — `(TupleX …)` must NOT match `Tuple`.
        assert_eq!(parse_head_type("(TupleX A)", "Tuple"), None);
    }

    #[test]
    fn a_multi_payload_variant_renders_its_payloads_spread_flat() {
        // A MULTI-payload variant `(P Int64 (Option Int64))` renders SPREAD FLAT — `(P 5 (Some 5))`, each
        // payload a token under the variant name — matching the wasm value form, NOT the nested `(P (tuple 5
        // (Some 5)))`. A SINGLE-payload variant carrying a tuple `(Q (Tuple Int64 Int64))` keeps the nested
        // `(Q (tuple 5 5))`. The descriptor's token COUNT per variant is the arity that distinguishes them
        // (before, a multi-payload variant collapsed to one `(Tuple …)` token, indistinguishable from a
        // single tuple payload → the rust gate rendered `(P (tuple …))` where wasm flattens).
        let nt = std::collections::HashMap::new();
        // The parser reads N payload tokens per variant: `P` has TWO, `Q` has ONE, `E` has ZERO.
        let ds = cdz_sum_descriptors(
            "// cdz-sum[W]: (P Int64 (Option Int64)) (E)\n// cdz-sum[V]: (Q (Tuple Int64 Int64)) (E)",
        );
        let w = &ds["W"];
        assert_eq!(
            w[0],
            (
                "P".to_string(),
                vec!["Int64".to_string(), "(Option Int64)".to_string()]
            )
        );
        assert_eq!(w[1], ("E".to_string(), Vec::<String>::new()));
        assert_eq!(
            ds["V"][0],
            ("Q".to_string(), vec!["(Tuple Int64 Int64)".to_string()])
        );

        // The multi-payload variant renders its two payloads FLAT — `(P {} {})` reading `(__p).0`/`(__p).1`.
        let expr = cdz_render_expr(
            "W",
            &ds,
            &nt,
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            expr.contains("(P {} {})") && expr.contains("(__p).0") && expr.contains("(__p).1"),
            "a multi-payload variant spreads its payloads flat under the name: {expr}"
        );
        assert!(
            !expr.contains("(P {})"),
            "a multi-payload variant must NOT render as one nested tuple payload: {expr}"
        );
        // A single-tuple-payload variant keeps the nested tuple — `(Q {})` where `{}` is `(tuple …)`.
        let vexpr = cdz_render_expr(
            "V",
            &ds,
            &nt,
            &std::collections::HashMap::new(),
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            vexpr.contains("(Q {})") && vexpr.contains("#tuple("),
            "a single tuple payload stays nested: {vexpr}"
        );
    }

    #[test]
    fn rust_call_arg_rebuilds_compound_values_as_rust_expressions() {
        // A bare SCALAR passes through verbatim — it is already a valid Rust literal.
        assert_eq!(rust_call_arg("20"), "20");
        assert_eq!(rust_call_arg("-1"), "-1");
        assert_eq!(rust_call_arg("true"), "true");
        // A `(tuple …)` becomes a Rust tuple.
        assert_eq!(rust_call_arg("(tuple 7 9)"), "(7, 9)");
        // A `(record …)` becomes a Rust tuple of its field VALUES in SORTED-KEY order — matching the
        // backend's record lowering. `(record (y 8) (x 3))` sorts x<y → `(3, 8)` regardless of source order.
        assert_eq!(rust_call_arg("(record (y 8) (x 3))"), "(3, 8)");
        assert_eq!(rust_call_arg("(record (x 3) (y 8))"), "(3, 8)");
        // NESTING composes: a tuple containing a record and a scalar.
        assert_eq!(
            rust_call_arg("(tuple (record (a 1) (b 2)) 5)"),
            "((1, 2), 5)"
        );
        // A ONE-element tuple/record needs a trailing comma so Rust reads it as a 1-tuple, not a paren-scalar.
        assert_eq!(rust_call_arg("(tuple 4)"), "(4,)");
        assert_eq!(rust_call_arg("(record (x 4))"), "(4,)");
        // A FLOAT SPECIAL-VALUE arg (`nan`/`inf`/`-inf`) is not a Rust value token → the `f64` constant.
        assert_eq!(rust_call_arg("nan"), "f64::NAN");
        assert_eq!(rust_call_arg("inf"), "f64::INFINITY");
        assert_eq!(rust_call_arg("-inf"), "f64::NEG_INFINITY");
        // A finite float literal is already valid Rust — passes through.
        assert_eq!(rust_call_arg("1.5"), "1.5");
        // NON-SCALAR entry args are marshalled through each type's own construction form (matching the
        // emitted library body), NOT the raw Cadenza text — the exported-entry non-scalar-arg class fix.
        // String → owned String.
        assert_eq!(rust_call_arg("\"abc\""), "\"abc\".to_string()");
        // Symbol `#"read"` → owned String too (a Symbol param emits as `String`; strip the `#` sigil).
        // Was emitted VERBATIM (`#"read"`) → rustc syntax error; the driver now marshals it like a String.
        assert_eq!(rust_call_arg("#\"read\""), "\"read\".to_string()");
        // BigInt `<digits>N` → `cdz_num::Big::from_i64` (in-i64).
        assert_eq!(rust_call_arg("100N"), "cdz_num::Big::from_i64(100)");
        // Rational `<int>R` → `Rational::new(n, 1)`; `<n>/<d>` → `Rational::new(n, d)`.
        assert_eq!(
            rust_call_arg("1R"),
            "cdz_num::Rational::new(cdz_num::Big::from_i64(1), cdz_num::Big::from_i64(1))"
        );
        assert_eq!(
            rust_call_arg("3/4"),
            "cdz_num::Rational::new(cdz_num::Big::from_i64(3), cdz_num::Big::from_i64(4))"
        );
        // Bytes `((. Bytes of) (list …))` → `vec![…u8]`.
        assert_eq!(
            rust_call_arg("((. Bytes of) (list 1 2 3))"),
            "vec![1u8, 2u8, 3u8]"
        );
        // A MALFORMED Bytes value (the `(list …)` shape absent) FALLS THROUGH to pass-through-verbatim —
        // NOT a silent empty `vec![]` (which would compile the wrong value, a harness miscompile, PR#507).
        // The raw text then reaches rustc/the backend, which rejects it (a loud error, not a wrong answer).
        assert_eq!(rust_call_arg("((. Bytes of) 5)"), "((. Bytes of) 5)");
        // List `(list …)` → `vec![…]`; Option/Result variants → the native enum.
        assert_eq!(rust_call_arg("(list 1 2 3)"), "vec![1, 2, 3]");
        assert_eq!(rust_call_arg("(Some 5)"), "Some(5)");
        assert_eq!(rust_call_arg("(None unit)"), "None"); // nullary — payload ignored
        assert_eq!(rust_call_arg("(Ok 7)"), "Ok(7)");
        // A list of tuples composes recursively.
        assert_eq!(
            rust_call_arg("(list (tuple 1 2) (tuple 3 4))"),
            "vec![(1, 2), (3, 4)]"
        );
    }

    #[test]
    fn rust_factory_param_count_splits_a_closure_factory_signature() {
        // A closure-FACTORY export (return type `Rc<dyn Fn(…)>`) → the factory's param count (the make/call
        // split point); an ordinary export → None (single call).
        let factory = "pub fn both(a: i64, b: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(factory, "both", false), Some(2));
        // A single-capture factory.
        let one = "pub fn scale(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(one, "scale", false), Some(1));
        // A NON-factory export (plain scalar return) → None.
        let plain = "pub fn add(a: i64, b: i64) -> i64 { … }";
        assert_eq!(rust_factory_param_count(plain, "add", false), None);
        // A compound-return export that is NOT a closure → None (only `Rc<dyn Fn` marks a factory).
        let vecret = "pub fn build(n: i64) -> Vec<i64> { … }";
        assert_eq!(rust_factory_param_count(vecret, "build", false), None);
        // A param type containing nested `<…>`/`(…)` must not miscount the param commas.
        let nested =
            "pub fn f(m: BTreeMap<i64, i64>, k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(nested, "f", false), Some(2));
        // MULTI-export module: a prefix `split("pub fn both")` also matches `pub fn both2(` — the fix must
        // pick the occurrence whose next char is the name boundary `(`, not the `both2` prefix (Copilot
        // PR#548). `both` has arity 2, `both2` has arity 3 — asking for `both` must NOT return 3.
        let multi = "pub fn both2(a: i64, b: i64, c: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … } \
                     pub fn both(a: i64, b: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(multi, "both", false), Some(2));
        assert_eq!(rust_factory_param_count(multi, "both2", false), Some(3));
        // Async factory: the name is followed by the generic list `<`, still a valid boundary. This test
        // string omits the env param, so its ONLY param `k` is a capture → Some(1) (nothing to discount).
        let async_fac =
            "pub async fn scale<E: CdzEnv>(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(async_fac, "scale", true), Some(1));
        // The REAL async emit prepends the gas/yield env param `__cdz_env: &mut __CdzE` — backend plumbing,
        // NOT a factory capture — so it must be DISCOUNTED: `scale(__cdz_env, k)` still has ONE capture `k`.
        // (This is why the discount detects the env param by name rather than a blind `-1`: the string above
        // has no env param and must stay Some(1), while this one has it and must ALSO be Some(1).)
        let async_real = "pub async fn scale<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(async_real, "scale", true), Some(1));
        // A NULLARY async factory (env param only, no captures) → Some(0), not an underflow.
        let async_nullary = "pub async fn mk<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(rust_factory_param_count(async_nullary, "mk", true), Some(0));
        // REGRESSION (v-rust-backend, latent panic): a `->` return arrow INSIDE the param list — from a
        // parameter whose type is itself a closure — must not be miscounted as an angle-bracket close
        // (which underflowed depth → `end` stayed 0 → `&after[1..0]` PANIC). A closure-PARAM consumer
        // that returns a scalar is NOT a factory → None (cleanly, no panic).
        let closure_param_consumer =
            "pub fn twice_plus(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { … }";
        assert_eq!(
            rust_factory_param_count(closure_param_consumer, "twice_plus", false),
            None
        );
        // A FACTORY whose own first param is a closure (arrow inside the param list) AND whose return is
        // a closure → still a factory; both params counted (the inner `->` no longer corrupts the walk).
        let factory_with_closure_param = "pub fn compose(g: std::rc::Rc<dyn Fn(i64) -> i64>, k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }";
        assert_eq!(
            rust_factory_param_count(factory_with_closure_param, "compose", false),
            Some(2)
        );
    }

    #[test]
    fn build_closure_consumer_call_synthesizes_the_producer_closure() {
        // A CONSUMER export takes a closure param; the driver builds it from a companion PRODUCER export.
        // (1) FACTORY producer (has a capture): the closure is `make_adder(<cap arg>)`, the scalar param
        // takes the next arg. `apply-it(g, x)` with `(call apply-it 100 7)` → `apply_it(make_adder(100), 7)`.
        let m = "pub fn make_adder(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … } \
                 pub fn apply_it(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { … }";
        assert_eq!(
            build_closure_consumer_call(m, "apply_it", &["100".into(), "7".into()], false)
                .as_deref(),
            Some("apply_it(prog::make_adder(100), 7)")
        );
        // (2) TWO closure params, ONE nullary PEELED producer (`mk` eta-peeled to `fn mk(x)->i64`): both
        // closures come from `mk` (REUSE — the host mints a fresh handle each), the scalar takes the arg.
        // `app2(f, g, x)` with `(call app2 5)` → `app2(<mk-closure>, <mk-closure>, 5)`. Pins DETERMINISTIC
        // multi-closure pairing (review ask #2): both wrap `prog::mk`, in param order, the scalar last.
        let m2 = "pub fn mk(x: i64) -> i64 { … } \
                  pub fn app2(f: std::rc::Rc<dyn Fn(i64) -> i64>, g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { … }";
        let call = build_closure_consumer_call(m2, "app2", &["5".into()], false).unwrap();
        assert_eq!(
            call,
            "app2((std::rc::Rc::new(prog::mk as fn(i64) -> i64) as std::rc::Rc<dyn Fn(i64) -> i64>), \
             (std::rc::Rc::new(prog::mk as fn(i64) -> i64) as std::rc::Rc<dyn Fn(i64) -> i64>), 5)"
        );
        // (3) A non-consumer (no closure param) → None (falls through to the factory/ordinary path).
        let m3 = "pub fn add(a: i64, b: i64) -> i64 { … }";
        assert_eq!(
            build_closure_consumer_call(m3, "add", &["1".into(), "2".into()], false),
            None
        );
    }

    #[test]
    fn build_closure_consumer_call_drives_the_async_factory_closure_through_block_on() {
        // ASYNC path (PR#813 review — the async branch was untested): a FACTORY-producer consumer must build
        // each closure via `block_on(prog::mk(&mut env, caps))`, bind it to a `let __gN` FIRST (so the
        // producer + consumer `&mut env` borrows don't overlap — E0499), then drive the consumer via
        // `block_on(prog::app(&mut env, __gN, scalars))`. The whole thing is one already-`prog::`-qualified,
        // already-`block_on`-wrapped block (the caller passes it verbatim — no double-wrap). Async sigs carry
        // the `&mut __CdzE` env param, which `parse_emitted_sig`/`is_env_param` skips for the arg mapping.
        let m = "pub async fn make_adder<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … } \
                 pub async fn apply_it<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { … }";
        assert_eq!(
            build_closure_consumer_call(m, "apply_it", &["100".into(), "7".into()], true)
                .as_deref(),
            Some(
                "{ let __g0 = block_on(prog::make_adder(&mut env, 100)); \
                 block_on(prog::apply_it(&mut env, __g0, 7)) }"
            )
        );
        // A non-consumer async export → None (the async nullary/args/factory shapes are handled by the
        // caller's own arg-threading, not here).
        let m2 =
            "pub async fn add<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, a: i64, b: i64) -> i64 { … }";
        assert_eq!(
            build_closure_consumer_call(m2, "add", &["1".into(), "2".into()], true),
            None
        );
    }

    #[test]
    fn build_closure_consumer_call_disambiguates_colliding_erasure_by_param_shape() {
        // COLLIDING ERASURE: a Tuple-arg factory (mka) and a Record-arg factory (mkb) BOTH emit
        // `Rc<dyn Fn((i64,i64)) -> i64>` — identical erased type. Without the shape notes the driver would
        // pair a consumer's Record-arg closure to the FIRST matching producer (mka, the tuple one) → wrong
        // value. The `// cdz-param-shapes[consumer]` note (the consumer's pre-erasure arrow) + each factory's
        // `// cdz-return` arrow disambiguate: appb (Record) must pair to mkb (Record), NOT mka (Tuple).
        let m = "// cdz-return[mka]: (-> (Tuple Int64 Int64) Int64)\n\
                 pub fn mka(k: i64) -> std::rc::Rc<dyn Fn((i64, i64)) -> i64> { … }\n\
                 // cdz-return[mkb]: (-> (Record (a Int64) (b Int64)) Int64)\n\
                 pub fn mkb(k: i64) -> std::rc::Rc<dyn Fn((i64, i64)) -> i64> { … }\n\
                 // cdz-param-shapes[appb]: (-> (Record (a Int64) (b Int64)) Int64)\n\
                 pub fn appb(h: std::rc::Rc<dyn Fn((i64, i64)) -> i64>, y: i64) -> i64 { … }";
        // appb's closure must build from mkb (Record), passing its capture arg (y consumed as the scalar):
        // `appb(prog::mkb(<cap>), <scalar>)`. The cap comes first; here the flat call `(call appb 9 6)` gives
        // mkb one cap (9) then appb's scalar (6).
        let call =
            build_closure_consumer_call(m, "appb", &["9".into(), "6".into()], false).unwrap();
        assert_eq!(
            call, "appb(prog::mkb(9), 6)",
            "the Record-arg consumer pairs to the RECORD factory (mkb), not the tuple mka: {call}"
        );
    }

    #[test]
    fn closure_param_type_extracts_the_balanced_type_and_ty_matches_is_exact_not_substring() {
        // `closure_param_type` must extract the BALANCED `Rc<dyn Fn(…)->…>` — stopping at the angle bracket
        // that matches `Rc<`, NOT the `>` inside the `->` return arrow, and NOT swallowing a trailing param.
        // A first-order closure param that is NOT last in the list yields ONLY the closure type.
        assert_eq!(
            closure_param_type("g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64"),
            Some("std::rc::Rc<dyn Fn(i64) -> i64>"),
            "a non-last closure param extracts the balanced closure type, excluding the trailing `, x: i64`"
        );
        // A HIGHER-ORDER closure param (its arg is itself a closure) extracts its WHOLE nested self.
        assert_eq!(
            closure_param_type(
                "g: std::rc::Rc<dyn Fn(std::rc::Rc<dyn Fn(i64) -> i64>) -> i64>, x: i64"
            ),
            Some("std::rc::Rc<dyn Fn(std::rc::Rc<dyn Fn(i64) -> i64>) -> i64>"),
            "a higher-order closure param extracts the full nested Rc<…>, excluding the trailing param"
        );
        // REGRESSION (github-liaison #1654): a higher-order PRODUCER must NOT false-pair to a FIRST-ORDER
        // consumer param via substring containment. mk is higher-order (`Rc<dyn Fn(Rc<dyn Fn(i64)->i64>)->
        // i64>`); app takes a FIRST-ORDER `g: Rc<dyn Fn(i64)->i64>` with a producer sibling `adder`. The
        // exact-match pairing must pick `adder` (the first-order producer), NOT `mk` (whose erased type
        // CONTAINS app's g type as a substring — the false match the old `contains` check admitted).
        let m = "pub fn mk(f: std::rc::Rc<dyn Fn(i64) -> i64>) -> i64 { … }\n\
                 pub fn adder(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { … }\n\
                 pub fn app(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { … }";
        let call =
            build_closure_consumer_call(m, "app", &["100".into(), "7".into()], false).unwrap();
        assert_eq!(
            call, "app(prog::adder(100), 7)",
            "app's first-order g pairs to the first-order factory `adder`, NOT the higher-order `mk`: {call}"
        );
    }

    #[test]
    fn cdz_render_bytes_list_emits_a_byte_int_list_render() {
        // A factory String result iterates its UTF-8 bytes; a Bytes result iterates the Vec<u8>. Both build
        // the `(b0 b1 …)` list<u8> form (a leading space per byte after the first → `()` when empty).
        let s = cdz_render_bytes_list("String");
        assert!(
            s.contains("(__r).bytes()")
                && s.contains("String::from(\"(\"")
                && s.contains("push(')')"),
            "String render iterates .bytes() into a paren byte list: {s}"
        );
        let b = cdz_render_bytes_list("Bytes");
        assert!(
            b.contains("(__r).iter().copied()") && b.contains("push_str(&__b.to_string())"),
            "Bytes render iterates the Vec<u8> into a paren byte list: {b}"
        );
    }

    #[test]
    fn is_safe_module_name_rejects_path_traversal_and_separators() {
        // Copilot PR#517 + #520: a package module name is used as a `<name>.sexp` filename, so it must be a
        // single safe path component or `dir.join` could escape the temp dir. Plain identifier-like names
        // pass; separators (`/`, `\`), `.`/`..` traversal, empty, and a Windows drive/ADS prefix (`C:foo` —
        // a `:`) are rejected — cross-platform (the checks are char-based, not `Path::components()`, which on
        // Linux would pass `a\b`/`C:foo` as a single Normal component).
        assert!(is_safe_module_name("lib"));
        assert!(is_safe_module_name("my-lib"));
        assert!(is_safe_module_name("mod_2"));
        assert!(!is_safe_module_name(""));
        assert!(!is_safe_module_name("."));
        assert!(!is_safe_module_name(".."));
        assert!(!is_safe_module_name("../evil"));
        assert!(!is_safe_module_name("a/b"));
        assert!(!is_safe_module_name("a\\b"));
        assert!(!is_safe_module_name("/etc/passwd"));
        // #520: a Windows drive/ADS prefix has no separator but is prefix/absolute on Windows — reject the `:`.
        assert!(!is_safe_module_name("C:foo"));
        assert!(!is_safe_module_name("C:\\foo"));
        assert!(!is_safe_module_name("stream:ads"));
    }

    #[test]
    fn wait_with_timeout_returns_output_for_a_fast_child() {
        // A child that exits well within the deadline yields its captured output + status.
        let child = std::process::Command::new("sh")
            .args(["-c", "printf hello; printf oops 1>&2; exit 0"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let out = wait_with_timeout(child, std::time::Duration::from_secs(10))
            .expect("wait ok")
            .expect("did not time out");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"hello");
        assert_eq!(out.stderr, b"oops");
    }

    #[test]
    fn wait_stages_with_timeout_returns_statuses_in_order_for_fast_children() {
        // All stages exit within the deadline → their ExitStatuses come back in pipeline order, so the
        // caller's "first failing stage sets the exit code" holds by index. Middle stage exits non-zero.
        let mk = |script: &str| {
            std::process::Command::new("sh")
                .args(["-c", script])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn sh")
        };
        let statuses = wait_stages_with_timeout(
            [
                ("s1", mk("exit 0")),
                ("s2", mk("exit 7")),
                ("s3", mk("exit 0")),
            ],
            std::time::Duration::from_secs(10),
        )
        .expect("all exited in time");
        assert!(statuses[0].success());
        assert_eq!(
            statuses[1].code(),
            Some(7),
            "middle stage's code preserved by index"
        );
        assert!(statuses[2].success());
    }

    #[test]
    fn wait_stages_with_timeout_kills_a_hanging_stage_and_names_the_first() {
        // The `xtask run` hang-bound (v-effects fresh-worktree hang, 2026-08-04): a stage that runs far
        // past the deadline is KILLED and reported as Timeout(name) — not waited out. s1 exits fast, s2
        // hangs; the first still-running stage (s2) is named. Tiny deadline so the test is fast; the
        // sleeper runs far longer, so it can only end via our kill.
        let start = std::time::Instant::now();
        let fast = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn fast");
        let hang = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleep");
        match wait_stages_with_timeout(
            [("s1-fast", fast), ("s2-hang", hang)],
            std::time::Duration::from_millis(150),
        ) {
            Err(StageWait::Timeout(stage)) => assert_eq!(stage, "s2-hang", "names the hung stage"),
            _ => panic!("a hanging stage must time out to Timeout(name)"),
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "timeout must kill promptly, not wait out the 30s sleeper"
        );
    }

    #[test]
    fn spans_overlap_detects_real_concurrency() {
        // Two spans that share time → overlap (real parallelism).
        assert!(spans_overlap(&[(0, 100), (50, 150)]));
        // Adjacent/serialized spans (B starts exactly when A ends) → NO overlap (the serialized case the
        // sanity-check must catch — parallelism was a no-op).
        assert!(!spans_overlap(&[(0, 100), (100, 200)]));
        assert!(!spans_overlap(&[(0, 100), (200, 300)]));
        // One span fully inside another → overlap.
        assert!(spans_overlap(&[(0, 500), (100, 200)]));
        // Unsorted input, three spans, only the last two overlap → still detected.
        assert!(spans_overlap(&[(300, 400), (0, 100), (350, 450)]));
        // A single span (or none) → nothing to overlap.
        assert!(!spans_overlap(&[(0, 100)]));
        assert!(!spans_overlap(&[]));
        // Degenerate zero-width spans never contribute an overlap.
        assert!(!spans_overlap(&[(50, 50), (50, 50)]));
    }

    #[test]
    fn wait_children_until_collects_every_independent_result() {
        // The concurrent-gate waiter: N children spawned up front run concurrently; each result stands
        // alone (a fast success + a non-zero exit both reported, in input order). Spawn 3 fast children.
        let mk = |script: &str| {
            std::process::Command::new("sh")
                .args(["-c", script])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn sh")
        };
        let results = wait_children_until(
            vec![
                ("green".into(), mk("exit 0")),
                ("red".into(), mk("exit 5")),
                ("green2".into(), mk("exit 0")),
            ],
            std::time::Duration::from_secs(10),
        );
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "green");
        assert!(matches!(&results[0].1, Ok(Some(s)) if s.success()));
        // The RED sibling stands alone — it doesn't abort the others (unlike the pipeline waiter).
        assert!(matches!(&results[1].1, Ok(Some(s)) if !s.success()));
        assert_eq!(results[2].0, "green2");
        assert!(matches!(&results[2].1, Ok(Some(s)) if s.success()));
    }

    #[test]
    fn wait_children_until_kills_a_hanging_child_but_keeps_the_others_results() {
        // A hung lane is KILLED at the deadline → Ok(None) (timed out, no verdict), while a fast sibling
        // still reports its real exit. Concurrent (not pipeline): one hang doesn't lose the others.
        let start = std::time::Instant::now();
        let fast = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn fast");
        let hang = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let results = wait_children_until(
            vec![("fast".into(), fast), ("hang".into(), hang)],
            std::time::Duration::from_millis(150),
        );
        assert!(
            matches!(&results[0].1, Ok(Some(s)) if s.success()),
            "the fast lane's real result survives"
        );
        assert!(
            matches!(&results[1].1, Ok(None)),
            "the hung lane is killed → Ok(None) (timed out)"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "must kill promptly at the deadline, not wait out the 30s sleeper"
        );
    }

    #[test]
    fn wait_with_timeout_kills_a_hanging_child() {
        // A child that would run far past the deadline is KILLED and reported as a timeout (`None`) —
        // the compile-hang / runaway-program bound. Use a tiny deadline so the test itself is fast; the
        // child sleeps far longer, so it can only end via our kill.
        let start = std::time::Instant::now();
        let child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let res = wait_with_timeout(child, std::time::Duration::from_millis(150)).expect("wait ok");
        assert!(res.is_none(), "a hanging child must time out to None");
        // It returned promptly at the deadline (killed), not after the child's own 30s sleep.
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "timeout must kill promptly, not wait out the child"
        );
    }

    #[test]
    fn cached_step_records_and_reuses_green_only() {
        // The core cache contract: a green verdict for an exact key is reusable; a DIFFERENT key
        // (compiler or corpus changed) is a miss → must re-run. Red is never stored (only record_green
        // exists), so a stale cache can never manufacture a false green.
        let repo = std::env::temp_dir().join(format!("cdz-cachedstep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        let tree = repo.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("x.cdz"), "one").unwrap();
        let binary = repo.join("cdz.bin");
        std::fs::write(&binary, b"compiler-v1").unwrap();
        let paths = Paths {
            repo: repo.clone(),
            seed: repo.clone(),
        };

        let c1 = CachedStep::new(&paths, "cdz-test x", &binary, &tree).expect("key computable");
        assert!(!c1.is_green(), "cold cache is a miss");
        c1.record_green();
        assert!(c1.is_green(), "after record_green, the same key is a hit");

        // Same inputs → same key → still a hit (this is the intra-batch reuse that saves the re-run).
        let c1b = CachedStep::new(&paths, "cdz-test x", &binary, &tree).expect("key computable");
        assert!(
            c1b.is_green(),
            "an identical (binary, tree) reuses the cached green"
        );

        // Change the corpus → different key → miss (full sweep re-runs; no coverage loss).
        std::fs::write(tree.join("x.cdz"), "one-changed").unwrap();
        let c2 = CachedStep::new(&paths, "cdz-test x", &binary, &tree).expect("key computable");
        assert!(
            !c2.is_green(),
            "a corpus change invalidates the cached green"
        );

        // Change the compiler binary → different key → miss.
        std::fs::write(tree.join("x.cdz"), "one").unwrap(); // restore corpus
        std::fs::write(&binary, b"compiler-v2").unwrap();
        let c3 = CachedStep::new(&paths, "cdz-test x", &binary, &tree).expect("key computable");
        assert!(
            !c3.is_green(),
            "a compiler-binary change invalidates the cached green"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn cached_step_multi_tree_key_covers_every_tree() {
        // The storeless-rerun cache keys on the cdz binary + TWO trees (cdz/tests + rcdzc/src) because
        // rcdzc's #[cfg(test)] tests aren't in the release binary (PR#648). A change to EITHER tree must
        // invalidate the cached green — the gap that binary-only keying missed.
        let repo = std::env::temp_dir().join(format!("cdz-cachemulti-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        let a = repo.join("a"); // stands in for cdz/tests
        let b = repo.join("b"); // stands in for rcdzc/src
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("t.rs"), "cdz-test-1").unwrap();
        std::fs::write(b.join("tests.rs"), "rcdzc-test-1").unwrap();
        let binary = repo.join("cdz.bin");
        std::fs::write(&binary, b"cdz-v1").unwrap();
        let paths = Paths {
            repo: repo.clone(),
            seed: repo.clone(),
        };
        let trees: [&Path; 2] = [&a, &b];

        let c1 = CachedStep::new_multi(&paths, "storeless-rerun", &binary, &trees)
            .expect("key computable");
        c1.record_green();
        assert!(
            CachedStep::new_multi(&paths, "storeless-rerun", &binary, &trees)
                .unwrap()
                .is_green(),
            "identical (binary + both trees) → hit"
        );

        // Change the SECOND tree (rcdzc/src) — the one the old binary-only key MISSED → must now be a miss.
        std::fs::write(b.join("tests.rs"), "rcdzc-test-2").unwrap();
        assert!(
            !CachedStep::new_multi(&paths, "storeless-rerun", &binary, &trees)
                .unwrap()
                .is_green(),
            "an rcdzc/src edit invalidates the cached green (the PR#648 gap is closed)"
        );

        // Restore tree b, change the FIRST tree (cdz/tests) → also a miss.
        std::fs::write(b.join("tests.rs"), "rcdzc-test-1").unwrap();
        std::fs::write(a.join("t.rs"), "cdz-test-2").unwrap();
        assert!(
            !CachedStep::new_multi(&paths, "storeless-rerun", &binary, &trees)
                .unwrap()
                .is_green(),
            "a cdz/tests edit invalidates the cached green"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn suite_timeout_reads_env_with_generous_default() {
        // Default is generous (6min for a normal suite, 45min for the heavy compiler-ml sweep) so a
        // slow-but-passing suite isn't false-failed; a positive override parses + applies to ALL suites;
        // zero/garbage fall back. (Env is process-global; set+clear around the assert.)
        // Serialize with the other env-mutating test (multi-threaded binary; see ENV_TEST_LOCK).
        let _env_guard = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: the ENV_TEST_LOCK guard makes env mutation exclusive across the env-touching tests.
        unsafe { std::env::remove_var("CDZ_SUITE_TIMEOUT_SECS") };
        assert_eq!(
            suite_timeout_for(""),
            std::time::Duration::from_secs(360),
            "normal suite default 6min"
        );
        assert_eq!(
            suite_timeout_for("implementation/compiler-ml"),
            std::time::Duration::from_secs(45 * 60),
            "compiler-ml gets the heavy-sweep cap (raises the false-hang threshold, not a quarantine)"
        );
        unsafe { std::env::set_var("CDZ_SUITE_TIMEOUT_SECS", "480") };
        assert_eq!(
            suite_timeout_for(""),
            std::time::Duration::from_secs(480),
            "override parses"
        );
        assert_eq!(
            suite_timeout_for("implementation/compiler-ml"),
            std::time::Duration::from_secs(480),
            "an explicit override applies to ALL suites, compiler-ml included (operator escape hatch)"
        );
        unsafe { std::env::set_var("CDZ_SUITE_TIMEOUT_SECS", "0") };
        assert_eq!(
            suite_timeout_for(""),
            std::time::Duration::from_secs(360),
            "zero rejected → default"
        );
        unsafe { std::env::set_var("CDZ_SUITE_TIMEOUT_SECS", "nope") };
        assert_eq!(
            suite_timeout_for(""),
            std::time::Duration::from_secs(360),
            "garbage → default"
        );
        unsafe { std::env::remove_var("CDZ_SUITE_TIMEOUT_SECS") };
    }

    #[test]
    fn ml_per_file_timeout_reads_env_with_hang_bound_default() {
        // The per-file cap is a HANG bound (1200s: covers the ~750-900s-under-load worst case, the value
        // pr-sync validated on its re-gates), tighter
        // than the whole-suite ceiling so ONE runaway compile fails fast+named instead of burning the 45min
        // suite budget; a positive override applies; zero/garbage fall back. (Env is process-global; a
        // separate env var, BUT the `per-file < suite` assert below READS CDZ_SUITE_TIMEOUT_SECS via
        // suite_timeout_for — so it races the suite-timeout test's writes unless serialized.)
        // Serialize with the other env-mutating test (multi-threaded binary; see ENV_TEST_LOCK) — this is
        // the race that flaked the batch-124 gate (pr-sync report).
        let _env_guard = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: the ENV_TEST_LOCK guard makes env mutation exclusive across the env-touching tests.
        unsafe { std::env::remove_var("CDZ_ML_PER_FILE_TIMEOUT_SECS") };
        assert_eq!(
            ml_per_file_timeout(),
            std::time::Duration::from_secs(1200),
            "default is the 1200s per-file hang bound (covers the ~750-900s-under-load legit-slow files)"
        );
        // Tighter than the whole-suite cap — the whole point (a runaway file can't eat the suite budget).
        assert!(
            ml_per_file_timeout() < suite_timeout_for("implementation/compiler-ml"),
            "per-file cap must be tighter than the whole-suite ceiling"
        );
        unsafe { std::env::set_var("CDZ_ML_PER_FILE_TIMEOUT_SECS", "300") };
        assert_eq!(
            ml_per_file_timeout(),
            std::time::Duration::from_secs(300),
            "override parses"
        );
        unsafe { std::env::set_var("CDZ_ML_PER_FILE_TIMEOUT_SECS", "0") };
        assert_eq!(
            ml_per_file_timeout(),
            std::time::Duration::from_secs(1200),
            "zero rejected → default"
        );
        unsafe { std::env::set_var("CDZ_ML_PER_FILE_TIMEOUT_SECS", "nope") };
        assert_eq!(
            ml_per_file_timeout(),
            std::time::Duration::from_secs(1200),
            "garbage → default"
        );
        unsafe { std::env::remove_var("CDZ_ML_PER_FILE_TIMEOUT_SECS") };
    }

    #[test]
    fn ml_test_jobs_clamps_default_and_override() {
        // Drives the PURE core with an injected override (`None` = unset/zero/garbage) so the test never
        // touches process-global env — cargo runs #[test]s in parallel, and a sibling test also reads env,
        // so env mutation here would be a data race (the reason `set_var`/`remove_var` are `unsafe`).
        // Default cap is COUPLED to the warm outcome: min(cores,4) when warm SUCCEEDED (cheap cache HITs,
        // 4-way doesn't race the per-file cap) vs min(cores,2) when it did NOT (a cold sweep needs ~2×
        // CPU/file — the original 4→2 downgrade's premise still holds on the cold path; reviewer FYI).
        // Clamped to the file count so we never spawn idle workers.
        assert_eq!(
            ml_test_jobs_from(None, 1, true),
            1,
            "default clamped to file count = EXACTLY 1 (not 0 — catches an accidental under-clamp)"
        );
        assert!(
            ml_test_jobs_from(None, 100, true) <= 4,
            "warmed default never exceeds the cap of 4 (post-warm the per-file runs are cheap cache HITs)"
        );
        assert!(
            ml_test_jobs_from(None, 100, false) <= 2,
            "COLD default never exceeds the conservative cap of 2 (a cold sweep must not race the per-file cap at 4-way)"
        );
        // The coupling is the whole point: on a many-core host, warmed must be able to exceed the cold cap.
        // (available_parallelism is >=1; on a >=4-core host warmed=4 > cold=2. Guard the invariant directly:
        // cold is never MORE than warmed for the same inputs.)
        assert!(
            ml_test_jobs_from(None, 100, false) <= ml_test_jobs_from(None, 100, true),
            "cold cap is never higher than the warmed cap"
        );
        assert!(
            ml_test_jobs_from(None, 100, true) >= 1,
            "always at least one job"
        );

        // A positive override applies REGARDLESS of warm outcome, still clamped to [1, file_count].
        assert_eq!(
            ml_test_jobs_from(Some(8), 100, true),
            8,
            "override applies below the file count (warmed)"
        );
        assert_eq!(
            ml_test_jobs_from(Some(8), 100, false),
            8,
            "override applies even on the cold path (operator lever overrides the conservative default)"
        );
        assert_eq!(
            ml_test_jobs_from(Some(8), 3, true),
            3,
            "override clamped down to the file count"
        );

        // Zero/garbage parse to `None` at the env boundary → default; the default is itself clamped, so
        // with 1 file it's 1.
        assert_eq!(
            ml_test_jobs_from(None, 1, true),
            1,
            "zero/garbage → default, clamped to a single file"
        );
        assert!(
            ml_test_jobs_from(None, 10, true) >= 1,
            "garbage → default (≥1)"
        );

        // A zero file count never yields a zero (or panicking) job count.
        assert_eq!(
            ml_test_jobs_from(None, 0, true),
            1,
            "empty file list still yields at least one worker"
        );
    }

    /// `env_closure_call_arg` maps flat applied args → the single `EnvClosure::call` `A`: `()` for 0 args,
    /// the bare arg for 1, a tuple for ≥2 — splitting on TOP-LEVEL commas only, with nesting balanced over
    /// `()`/`[]`/`{}`/`<>` and the `->` arrow's `>` NOT counted as a group close (github-liaison #2391 c1).
    #[test]
    fn env_closure_call_arg_tuples_top_level_args_only() {
        // Arity 0/1: `()` and the bare arg.
        assert_eq!(env_closure_call_arg(""), "()");
        assert_eq!(env_closure_call_arg("5"), "5");
        // Arity ≥2: a tuple of the top-level args.
        assert_eq!(env_closure_call_arg("3, 4"), "(3, 4)");
        // A compound arg keeps its INNER commas — `(a, (x, y))` is TWO top-level args → tupled as one `A`,
        // but the inner tuple's comma is not a top-level split (paren-balanced).
        assert_eq!(env_closure_call_arg("a, (x, y)"), "(a, (x, y))");
        // A SINGLE compound arg with inner commas is ONE arg → the bare arg (no extra wrap).
        assert_eq!(env_closure_call_arg("(1, 2)"), "(1, 2)");
        assert_eq!(env_closure_call_arg("vec![1, 2, 3]"), "vec![1, 2, 3]");
        // DEFENSIVE (c1): a `{}` block/struct-literal arg with inner commas stays ONE arg (brace-balanced),
        // not mis-split into several.
        assert_eq!(
            env_closure_call_arg("Foo { a: 1, b: 2 }"),
            "Foo { a: 1, b: 2 }"
        );
        // DEFENSIVE (c1): a closure-typed arg whose type spells `->` — the arrow's `>` must not underflow the
        // `<` depth, so an inner `,` after it is still seen as nested (this stays ONE arg).
        assert_eq!(
            env_closure_call_arg("x as Rc<dyn Fn(i64) -> (i64, i64)>"),
            "x as Rc<dyn Fn(i64) -> (i64, i64)>"
        );
    }
}

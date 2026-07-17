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

use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

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
    /// One-time checkout bootstrap: symlink the gitignored `.claude/{skills,commands}` at the tracked
    /// `skills/`/`commands/` at the repo root, so this checkout's Claude picks them up. Idempotent.
    Setup,
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
        /// A level that changes the outcome is a candidate miscompile (hard fail). Wasm-target only for now
        /// (rejects `--target rust`/`rust-async`); ignores `--save`/`--check` (no baseline — a same-run
        /// cross-level diff).
        #[arg(long, conflicts_with_all = ["save", "check"])]
        opt_sweep: bool,
    },
    /// The omnibus health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
    /// the wasm runtime build, and the behavior gate. Each step's output is captured to a log file
    /// (`target/xtask-logs/`); the console shows one ✓ per step, and the first failing step prints
    /// the whole log + its path.
    Check,
    /// Round-trip every corpus program through the syntax surfaces: `sexpr` must reproduce the exact
    /// binary AST; `ml` (the long-term syntax, allowed to canonicalize once) must round-trip to a
    /// FIXED POINT (`ml(ml(x)) == ml(x)`). Guards `cadenza-syntax` independently of the compiler.
    Roundtrip {
        /// The corpus `.sexp` files to check. [default: all of spec/semantics/*.sexp]
        files: Vec<PathBuf>,
    },
    /// Format Cadenza program file(s) through the printer, rewriting them in place.
    Fmt {
        /// The `.cdz`/`.sexp` files to format.
        files: Vec<PathBuf>,
        /// The surface to format to. [default: sexpr]
        #[arg(long, default_value = "sexpr")]
        to: String,
        /// Don't write; exit non-zero if any file is not already formatted (for CI).
        #[arg(long)]
        check: bool,
    },
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
    Codegen {
        /// Don't write; regenerate in memory and exit non-zero if the committed file is out of date.
        /// This is the STALENESS GATE (wired into `xtask check`): it makes a forgotten regeneration a
        /// hard failure rather than a silent drift, so the generated ABI can never fall behind the WIT.
        #[arg(long)]
        check: bool,
    },
    /// Run the runtime allocation benchmark (gross heap allocs per hot op) and diff against the
    /// committed baseline `spec/bench/.alloc-baseline`. Allocation count — not wall-clock — is the
    /// tracked metric: it is identical native↔wasm and deterministic, so it catches an allocation
    /// regression the way `gate --check` catches a behavior one. Exits non-zero on a regression.
    Bench {
        /// Record the current counts as the committed baseline, then exit.
        #[arg(long)]
        save: bool,
    },
    /// Citation-coverage regression gate (wired into `check`): run `duvet report`, count the `//=` /
    /// `//#` citation annotations, and fail if the count drops below the committed floor in
    /// `.duvet/coverage-floor.json` — a deleted/stranded citation turns the gate red. Gates on a
    /// machine-STABLE count, NOT the churny (gitignored) `.duvet/snapshot.txt`. Fail-soft: SKIPS (does
    /// not fail) when `duvet` isn't installed, so it never reddens `check` on a machine lacking the tool.
    DuvetCheck {
        /// Record the current counts as the new floor (what `v-duvet-coverage` runs after adding
        /// citations), then exit. Without it, enforce the committed floor.
        #[arg(long)]
        save: bool,
    },
    /// Run the cdz-runtime test suite under Miri — the UB oracle for the refcount/FBIP heap core.
    /// Miri interprets the tests and flags use-after-free, out-of-bounds, uninitialized reads, and
    /// aliasing violations (Stacked Borrows) that the normal test run cannot see. The runtime's
    /// tagged-pointer `Handle` stuffs immediates into a pointer's low bits, so this REQUIRES
    /// `-Zmiri-permissive-provenance` (strict provenance would flag every immediate); the recipe below
    /// sets it. Miri is ~100-1000x slower than native, so by default this runs the ALIASING-CRITICAL
    /// subset (the FBIP / reuse / shared-version / cursor-fork tests — the `unsafe as_mut/as_ref` +
    /// `mem::take`-reuse paths where a memory bug would live), not the whole (mostly pure-logic) suite.
    Miri {
        /// A test-name filter. Defaults to the aliasing-critical subset; pass e.g. `fbip` or a specific
        /// test, or an empty string to run the WHOLE suite (slow).
        #[arg(long, default_value = "fbip")]
        filter: String,
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
    /// Build the `cdz` LSP server and install the Cadenza VS Code extension into the local editor, in
    /// one command: builds `cdz` (release), installs the extension's npm deps, bakes the built binary's
    /// path into the extension, and symlinks `integrations/vscode` into every VS Code extensions dir
    /// found (`~/.vscode`, `~/.vscode-server`, forks). Reload the editor window afterward and open a
    /// `.cdz` file. Re-run after rebuilding `cdz`. No `code`/`vsce` CLI needed — the symlink IS the install.
    InstallLsp {
        /// Remove the extension symlinks (leaves the built binary + npm deps in place).
        #[arg(long)]
        uninstall: bool,
    },
    /// Orchestrate the autonomous-agent fleet: bring agents up as named tmux windows, tear them
    /// down, inspect the board, add/remove agents, and route inbox messages. The durable manifest is
    /// `.claude/fleet/registry.json`; see `.claude/fleet/AGENTS-fleet.md` for the agent contract.
    Fleet {
        #[command(subcommand)]
        cmd: fleet::FleetCmd,
    },
}

fn main() {
    let paths = Paths::resolve();
    let cli = Cli::parse();
    let profile = cli.profile.as_str();
    match cli.command {
        Cmd::Setup => setup(&paths),
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
        } => {
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
                },
            };
            if opt_sweep {
                gate_opt_sweep(&paths, profile, &gate_opts);
            } else {
                gate(&paths, profile, gate_opts);
            }
        }
        Cmd::Check => check(&paths, profile),
        Cmd::Roundtrip { files } => roundtrip(&paths, profile, files),
        Cmd::Fmt { files, to, check } => fmt(&paths, profile, files, &to, check),
        Cmd::Emit { file, from, out } => emit(&paths, profile, &file, &from, out),
        Cmd::Codegen { check } => codegen::run(&paths, check),
        Cmd::Bench { save } => bench::run(&paths, save),
        Cmd::DuvetCheck { save } => duvet_check::run(&paths, save),
        Cmd::Miri { filter } => miri(&paths, &filter),
        Cmd::GuideWasm { store } => guide_wasm(&paths, store),
        Cmd::InstallLsp { uninstall } => install_lsp::run(&paths, uninstall),
        Cmd::Fleet { cmd } => fleet::run(&paths, cmd),
    }
}

/// Run the cdz-runtime tests under Miri (the UB oracle). See the `Cmd::Miri` doc for why. Uses the
/// `nightly` toolchain + `-Zmiri-permissive-provenance` (mandatory for the tagged-pointer `Handle`) and
/// `-Zmiri-disable-isolation` (the tests read no external state, but this avoids isolation friction).
fn miri(paths: &Paths, filter: &str) {
    let rt = paths.seed.join("crates/cdz-runtime");
    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&rt)
        .args(["+nightly", "miri", "test", "--lib"])
        .env(
            "MIRIFLAGS",
            "-Zmiri-disable-isolation -Zmiri-permissive-provenance",
        )
        // The suite needs a deep stack for the pathological-depth tests, same as the normal run.
        .env("RUST_MIN_STACK", "67108864");
    if !filter.is_empty() {
        cmd.arg(filter);
    }
    eprintln!(
        "xtask miri: running cdz-runtime tests under Miri (filter: {:?}) — this is SLOW (~100-1000x)…",
        if filter.is_empty() {
            "<whole suite>"
        } else {
            filter
        }
    );
    match cmd.status() {
        Ok(s) if s.success() => eprintln!("xtask miri: OK (no UB reported)"),
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("xtask miri: failed to launch cargo miri: {e}");
            eprintln!("  is Miri installed? `rustup component add --toolchain nightly miri`");
            std::process::exit(1);
        }
    }
}

mod bench;
mod codegen;
mod duvet_check;
mod fleet;
mod install_lsp;

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
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate has a parent (the repo root)")
            .to_path_buf();
        let seed = repo.join("implementation/seed");
        Paths { repo, seed }
    }
}

/// One-time checkout bootstrap: point the gitignored `.claude/{skills,commands}` at the tracked
/// `skills/`/`commands/` at the repo root. `.claude/` is not committed (so a fresh clone or a new
/// worktree lacks these links), but the sources travel with the repo — this wires them up. Idempotent:
/// an already-correct link is left alone; a real directory or wrong link in the way is reported, not
/// clobbered, so a hand-made setup is never destroyed silently.
fn setup(paths: &Paths) {
    let claude = paths.repo.join(".claude");
    std::fs::create_dir_all(&claude).expect("create .claude dir");

    let mut any_change = false;
    for name in ["skills", "commands"] {
        // The source must exist to link at (it is tracked in the repo).
        if !paths.repo.join(name).is_dir() {
            eprintln!("  ! {name}: no `{name}/` at the repo root to link — skipped");
            continue;
        }
        let link = claude.join(name);
        let want = PathBuf::from("..").join(name); // relative: `.claude/<name>` → `../<name>`

        // Already the correct symlink? Leave it.
        if link.is_symlink() && std::fs::read_link(&link).ok() == Some(want.clone()) {
            println!("  ✓ .claude/{name} already linked");
            continue;
        }
        // A real directory (e.g. the old copied layout) — do not delete the user's files.
        if link.exists() && !link.is_symlink() {
            eprintln!(
                "  ! .claude/{name} is a real directory, not a symlink — move it aside and re-run \
                 `cargo xtask setup` to link it to `{name}/`"
            );
            continue;
        }
        // A stale/wrong symlink: safe to replace (a symlink owns no content).
        if link.is_symlink() {
            let _ = std::fs::remove_file(&link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&want, &link).expect("create symlink");
        #[cfg(not(unix))]
        {
            eprintln!(
                "  ! symlinks not supported on this platform — link .claude/{name} → {name}/ by hand"
            );
            continue;
        }
        println!("  + linked .claude/{name} → {}", want.display());
        any_change = true;
    }

    if any_change {
        println!("\nsetup: done — Claude will pick up this checkout's skills and commands.");
    } else {
        println!("\nsetup: nothing to do (already wired up).");
    }
}

fn build(paths: &Paths, store: Option<PathBuf>) {
    let store = store.unwrap_or_else(|| paths.repo.join("target/cadenza-store"));
    std::fs::create_dir_all(&store).expect("create store dir");

    // Build BOTH runtime components (wasm32) and content-address each into the store: the RELEASE
    // runtime (what a shipped program pins + composes) and the DEBUG-COUNTERS runtime (the same code
    // with the `live-objects` leak counter — the Perceus leak-check harness composes it, located by
    // content address, never rebuilt). The debug build is read BEFORE the release build overwrites the
    // shared output path.
    println!("== xtask: building the value-heap runtime component (release + debug-counters) ==");
    let sh = Shell::new().expect("open a shell for the component build");

    let debug_wasm = build_component_with_features(
        &sh,
        &paths.seed,
        "cdz-runtime",
        "cdz_runtime",
        &["debug-counters"],
    );
    // CANONICALIZE (strip the tool-version `producers` sections) before hashing + storing, so the hash
    // is reproducible across machines with the same rustc — the stored artifact IS the stripped bytes,
    // so a composed program's imported hash matches the file on disk.
    let debug_bytes = canonicalize_runtime(&debug_wasm);
    let debug_hash = content_address(&debug_bytes);
    std::fs::write(store.join(format!("{debug_hash}.wasm")), &debug_bytes).expect("store debug rt");
    println!("   debug-counters runtime content address: {debug_hash}");

    let runtime_wasm = build_component(&sh, &paths.seed, "cdz-runtime", "cdz_runtime");
    let runtime_bytes = canonicalize_runtime(&runtime_wasm);
    let runtime_hash = content_address(&runtime_bytes);
    println!("   runtime content address: {runtime_hash}");
    let runtime_stored = store.join(format!("{runtime_hash}.wasm"));
    std::fs::write(&runtime_stored, &runtime_bytes).expect("store runtime");
    println!("   stored → {}", runtime_stored.display());

    // A small manifest recording both stored runtimes, for the host / verifier to consult.
    let manifest = format!(
        "# Cadenza content-addressed store — the value-heap runtime.\n\
         runtime = \"{runtime_hash}\"\n\
         debug_runtime = \"{debug_hash}\"\n"
    );
    std::fs::write(store.join("runtime.toml"), manifest).expect("write runtime.toml");

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
    let mut run = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));

    // Wait on every stage; the first that fails determines the exit code. Waiting on all (rather
    // than short-circuiting) reaps each child and lets its stderr finish flushing to the terminal.
    let statuses = [
        ("cdz-syntax", syntax.wait()),
        ("rcdzc", rcdzc.wait()),
        ("cdz-run", run.wait()),
    ];
    for (stage, status) in statuses {
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("xtask run: {stage} did not complete: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// A stage's binary could not be spawned at all (missing/not-executable) — distinct from it running
/// and exiting non-zero, which is surfaced by its wait status.
fn launch_fail(stage: &str, e: std::io::Error) -> ! {
    eprintln!("xtask run: could not launch {stage}: {e}");
    std::process::exit(1);
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
}

/// Build the three pipeline tools once (under `profile`) and return their binary paths — shared by
/// `run`/`gate`/`roundtrip`/`fmt`/`emit` so none pays a per-invocation `cargo run` build. The
/// interactive commands use `dev` (fast build); the corpus gate uses `release-debug` (optimized), so
/// that the ~900-case run is not dominated by unoptimized tools.
fn build_tools(paths: &Paths, profile: &str) -> Tools {
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
        "cargo build --quiet --profile {profile} -p cdz -p cdz-corpus -p cdz-run -p cdz-rt -p cdz-num"
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
    Tools {
        syntax: cdz.clone(),
        corpus: bin.join("cdz-corpus"),
        rcdzc: cdz,
        run: bin.join("cdz-run"),
        cdz_rt_dir,
        cdz_num_dir,
    }
}

/// The outcome of driving one program (sexpr text) through the pipeline.
enum Ran {
    /// Ran to a value, rendered to canonical text, plus the OBSERVED HOST CALLS (each a dotted `E.op`, in
    /// call order — from cdz-run's `host-call` stderr lines). The observed sequence is verified against a
    /// case's `(host-calls …)`; empty for a program that makes no host call (the common shape).
    Value(String, Vec<String>),
    /// The compiler rejected/declined the program. `code` is the diagnostic CODE the compiler emitted
    /// (`Some("CDZ0210")`) — a TYPED rejection the corpus can match against `(error CODE)` — or `None`
    /// for a codeless DECLINE (an unimplemented construct: `Reject::decline`), which grades as `todo`
    /// (not-yet-built), never a disagreement. This is the "grade by what the compiler DOES" rule applied
    /// to rejections: a coded reject is a decision to check, a codeless decline is a gap to fill.
    Declined { code: Option<String> },
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
    /// The self-hosted Cadenza-in-Cadenza (ML) compiler, invoked via `cdz run-ml` (source in, one-line
    /// verdict out). Its front-end supports only a small subset today, so most programs DECLINE — this
    /// target is graded DIFFERENTIALLY against the Wasm (rcdzc) oracle, NOT the corpus directly (see the
    /// `ml-conformance` reported step): a decline is coverage-not-yet, an agreeing value is progress, a
    /// disagreeing value is the only real failure. It never drives the baseline gate.
    CadenzaMl,
}

/// The `--target` value clap parses for the `gate` command (its own enum so clap validates the
/// spelling and `--help` lists the choices), mapped to [`GateTarget`] at dispatch.
#[derive(Clone, Copy, clap::ValueEnum)]
enum GateTargetArg {
    Wasm,
    Rust,
    RustAsync,
}

/// Drive one program's s-expression `text` through cdz-syntax → rcdzc → cdz-run, returning the
/// outcome. Uses a real pipe with the program fed on cdz-syntax's stdin (no temp files). When `call`
/// is given, the export is invoked with those runtime arguments (`--call <export> --arg <v>…`) — how a
/// case exercises a parameterized entrypoint rather than a nullary one; `None` runs the sole export
/// with no arguments (the common case).
fn run_program(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    modules: &[(String, String)],
    call: Option<&Call>,
    host_responses: &[(String, String)],
    target: GateTarget,
) -> Ran {
    match target {
        GateTarget::Wasm => {
            run_program_wasm(tools, store, program, modules, call, host_responses, None)
        }
        // The Rust backend has no host-boundary path — a host-delegating case declines there (Todo).
        GateTarget::Rust if !host_responses.is_empty() => Ran::Declined { code: None },
        GateTarget::RustAsync if !host_responses.is_empty() => Ran::Declined { code: None },
        GateTarget::Rust => run_program_rust(tools, program, modules, call, false),
        GateTarget::RustAsync => run_program_rust(tools, program, modules, call, true),
        // The ML compiler has no package/host path today — a multi-file or host-delegating case is
        // simply not-yet-supported there, which is a decline (coverage-not-yet), never a disagreement.
        GateTarget::CadenzaMl if !modules.is_empty() || !host_responses.is_empty() => {
            Ran::Declined { code: None }
        }
        GateTarget::CadenzaMl => run_program_ml(tools, program, call),
    }
}

/// Drive one program through the self-hosted ML compiler via `cdz run-ml` — source on stdin, one line
/// of verdict on stdout: `value <sexpr>` | `declined` | `error <msg>` (exit 0 always; a non-zero exit
/// is a harness read failure). Maps that verdict to [`Ran`]: `value` → `Ran::Value` (no host calls —
/// the ML path has no host boundary), `declined` → a codeless `Ran::Declined` (not-yet-supported, so it
/// grades as `todo`/coverage-not-yet, never a disagreement), `error <msg>` → `Ran::Trap` (the program
/// reached the compiler and was rejected with a message — distinct from a clean decline). The ML
/// front-end is subset-only today, so most programs decline; this target is graded differentially
/// against the Wasm oracle by the `ml-conformance` step, not against the corpus directly. `call` is
/// currently ignored (the ML subset is nullary); a parameterized case declines above via the subset.
fn run_program_ml(tools: &Tools, program: &str, _call: Option<&Call>) -> Ran {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // `tools.rcdzc` is the unified `cdz` binary; `run-ml` reads the program SOURCE from stdin.
    let mut child = match Command::new(&tools.rcdzc)
        .arg("run-ml")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Ran::BadArtifact(format!("cdz run-ml spawn failed: {e}")),
    };
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(program.as_bytes())
    {
        return Ran::BadArtifact(format!("cdz run-ml: writing program to stdin failed: {e}"));
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return Ran::BadArtifact(format!("cdz run-ml wait failed: {e}")),
    };
    // A non-zero exit is the ONE reserved harness-read-failure path (per the run-ml contract) — a
    // broken feed, not a verdict.
    if !out.status.success() {
        return Ran::BadArtifact(format!(
            "cdz run-ml exited non-zero: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // The verdict is the LAST non-empty stdout line (the contract is one line, but be robust to a
    // trailing newline / an incidental leading line).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let verdict = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if verdict == "declined" {
        Ran::Declined { code: None }
    } else if let Some(v) = verdict.strip_prefix("value ") {
        Ran::Value(v.trim().to_string(), Vec::new())
    } else if let Some(msg) = verdict.strip_prefix("error ") {
        Ran::Trap(msg.trim().to_string())
    } else {
        Ran::BadArtifact(format!("cdz run-ml: unrecognized verdict line {verdict:?}"))
    }
}

/// Drive one program through cdz-syntax → rcdzc (wasm) → cdz-run — the historical path. A multi-file
/// PACKAGE case (`modules` non-empty) instead writes the entry + library files to a temp dir and runs
/// `cdz compile <files> --entry main` (the package path); either way the emitted component is run the
/// same way.
fn run_program_wasm(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    modules: &[(String, String)],
    call: Option<&Call>,
    host_responses: &[(String, String)],
    opt_level: Option<&str>,
) -> Ran {
    use std::io::Write;
    use std::process::Stdio;

    // Emit the component bytes — either the single-file pipe or the multi-file package compile.
    // `opt_level` selects the pass tier (only the opt-sweep passes a level; `None` = compiler default).
    let component = if modules.is_empty() {
        emit_component_single_at(tools, program, opt_level)
    } else {
        emit_component_package(tools, program, modules)
    };
    let component = match component {
        Ok(bytes) => bytes,
        Err(ran) => return ran,
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
    // A `(call …)` case names the export and passes runtime arguments; cdz-run coerces each `--arg` to
    // the export's declared parameter type (its `--arg` allows a leading `-`, so a negative value is
    // taken as the argument, not a flag).
    if let Some(call) = call {
        run.arg("--call").arg(&call.export);
        for arg in &call.args {
            run.arg("--arg").arg(arg);
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
    let run_out = child.wait_with_output().expect("wait cdz-run");
    if run_out.status.success() {
        // cdz-run prints the OBSERVED host calls to stderr as `host-call\t<op>` lines, in call order;
        // parse them so the case's `(host-calls …)` can be verified. Empty for a non-host program.
        let observed = observed_host_calls(&run_out.stderr);
        Ran::Value(
            String::from_utf8_lossy(&run_out.stdout).trim().to_string(),
            observed,
        )
    } else {
        Ran::Trap(first_line(&run_out.stderr))
    }
}

/// The single-file component-emit path: program text (stdin) → binary AST → component (stdout), via
/// the `cdz convert | cdz compile` pipe. `Err(Ran::Declined)` on a rejection/decline (its code
/// recovered from stderr).
/// The single-file component-emit path at an optimization level — `opt_level` is the `cdz compile
/// --opt-level` value (`"O0"`..`"O3"`), or `None` for the compiler default (`O1`). Only the
/// opt-level-equivalence sweep ([`gate_opt_sweep`]) passes a level; the normal gate passes `None` so its
/// behavior is byte-identical to before.
fn emit_component_single_at(
    tools: &Tools,
    program: &str,
    opt_level: Option<&str>,
) -> Result<Vec<u8>, Ran> {
    use std::io::Write;
    use std::process::{Command, Stdio};

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
    let rcdzc = Command::new(&tools.rcdzc)
        .args(&compile_args)
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = rcdzc.wait_with_output().expect("wait rcdzc");
    let _ = syntax.wait();
    if rcdzc_out.status.success() {
        Ok(rcdzc_out.stdout)
    } else {
        // A rejection: recover the diagnostic CODE from the first `error [CODE]` line rcdzc printed to
        // stderr. A TYPED rejection carries a code; a codeless DECLINE (unimplemented construct) none.
        Err(Ran::Declined {
            code: first_error_code(&rcdzc_out.stderr),
        })
    }
}

/// The multi-file PACKAGE component-emit path (`DESIGN-package-linking.md`): write the ENTRY (`program`,
/// as `main.sexp`) and each library `(name, prog)` (as `<name>.sexp`) into a fresh temp dir, then run
/// `cdz compile <lib>.sexp… main.sexp --entry main -o -` — the `cdz` front-end parses each source in
/// process and `compile()` links them. `Err(Ran::Declined)` on a reject/decline (code from stderr).
fn emit_component_package(
    tools: &Tools,
    program: &str,
    modules: &[(String, String)],
) -> Result<Vec<u8>, Ran> {
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
    let out = cmd
        .output()
        .unwrap_or_else(|e| launch_fail("cdz compile", e));
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(Ran::Declined {
            code: first_error_code(&out.stderr),
        })
    }
}

/// Emit Rust source for a SINGLE-file program via the `cdz convert | cdz compile - --target <rust>` pipe.
/// `Ok(source)` or `Err(Ran::Declined { code })` on a shared-front reject/decline (code from stderr).
fn emit_rust_single(tools: &Tools, program: &str, rust_target: &str) -> Result<String, Ran> {
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
    let rcdzc = Command::new(&tools.rcdzc)
        .args(["compile", "-", "-o", "-", "--target", rust_target])
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = rcdzc.wait_with_output().expect("wait rcdzc");
    let _ = syntax.wait();
    if !rcdzc_out.status.success() {
        return Err(Ran::Declined {
            code: first_error_code(&rcdzc_out.stderr),
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
    let out = cmd
        .output()
        .unwrap_or_else(|e| launch_fail("cdz compile", e));
    let _ = std::fs::remove_dir_all(&dir);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(Ran::Declined {
            code: first_error_code(&out.stderr),
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
fn run_program_rust(
    tools: &Tools,
    program: &str,
    modules: &[(String, String)],
    call: Option<&Call>,
    async_mode: bool,
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
        match emit_rust_single(tools, program, rust_target) {
            Ok(m) => m,
            Err(ran) => return ran,
        }
    } else {
        match emit_rust_package(tools, program, modules, rust_target) {
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
            let args: Vec<String> = c.args.iter().map(|a| rust_call_arg(a)).collect();
            (
                rust_ident(&c.export),
                format!("{}({})", rust_ident(&c.export), args.join(", ")),
            )
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
    // The driver's `fn main` calls the export and prints the result the way cdz-run renders it. In ASYNC
    // mode the export is an `async fn` taking `&mut impl CdzEnv` first, so the driver supplies a no-limit
    // gas `Env` + a minimal `block_on` executor and drives `prog::export(&mut env, args)` — the answer
    // must MATCH the sync/wasm oracle (gas metering is invisible to the result), so it grades identically.
    let call_or_await = if async_mode {
        // Rewrite `export(args…)` → `block_on(prog::export(&mut GATE_ENV_INIT, args…))`; a nullary call
        // `export()` becomes `block_on(prog::export(&mut …))`.
        let inner = if call_expr.ends_with("()") {
            format!("prog::{}(&mut env)", export)
        } else {
            // `export(a, b)` → `prog::export(&mut env, a, b)`.
            let arglist = call_expr
                .strip_prefix(&format!("{export}("))
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            format!("prog::{export}(&mut env, {arglist})")
        };
        format!("block_on({inner})")
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
    // …and the QUANTITY result's unit VALUE-FORM (`// cdz-unit[<ident>]: <value-form>`) — the dotted
    // `((. Unit base) …)` / `Unit./`-quotient surface cdz-run prints a quantity value with, which the type
    // note's `render_name` (the bare `(Unit.base …)` / `Unit.*` TYPE surface) does NOT carry. The backend
    // emits it via `Unit::render_value_form` for every Qty result, so the top-level Qty render splices it
    // verbatim rather than reconstructing it from the type string. Keyed by the export's ident.
    let unit_form = cdz_unit_form(&module, &export);
    // …and the NON-scale-1 quantity's `num/den` scale (`// cdz-scale[<ident>]`) — the harness multiplies the
    // boundary magnitude by it so `5 kilometer` displays `5000 meter`. `None` for a scale-1/non-Qty result.
    let unit_scale = cdz_scale(&module, &export);
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
            cdz_render_expr(
                ty,
                &sums,
                &newtypes,
                &sum_params,
                unit_form.as_deref(),
                unit_scale,
            )
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
    // In async mode the driver needs an `Env` impl (a no-limit gas meter — the gate checks ANSWERS, not
    // fuel bounds) and a tiny `block_on` executor, plus `let mut env = …` before the call.
    let full = if async_mode {
        format!("mod prog {{\n{module}\n}}\n{ASYNC_GATE_HARNESS}\n{body}")
    } else {
        format!("mod prog {{\n{module}\n}}\n{body}")
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
    let mut cmd = Command::new("rustc");
    cmd.args(["-O", "--edition", "2021"])
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
        match Command::new(&bin).output() {
            Ok(o) => {
                got = Some(o);
                break;
            }
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
        // The Rust backend has no host-boundary path — a host-delegating case declines before reaching
        // here — so the observed-host-call list is always empty.
        Ran::Value(
            String::from_utf8_lossy(&run.stdout).trim().to_string(),
            Vec::new(),
        )
    } else {
        Ran::Trap(rust_panic_message(&run.stderr))
    }
}

/// The trap REASON from a Rust process's panic stderr. Rust formats a panic as
/// `thread '<name>' panicked at <file>:<line>:<col>:` followed by the panic MESSAGE on the NEXT line
/// (`panic!("unreachable")` → the message line is `unreachable`). The gate's `trap_kind` classifies by
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

/// Make a Cadenza name a Rust identifier the SAME way the Rust backend does (`sanitize_ident`): a `-`
/// (and any non-ident char) becomes `_`. Kept in lockstep so the driver's call names match the emitted
/// `pub fn` names. (A small copy rather than a dependency on the compiler crate, per the xtask/tools
/// process boundary — the tools are separate binaries.)
fn rust_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()) {
            s.push(c);
        } else if c.is_ascii_digit() {
            s.push('_');
            s.push(c);
        } else {
            s.push('_');
        }
    }
    if s.is_empty() {
        s.push('_');
    }
    s
}

/// Translate a corpus CALL ARGUMENT (a canonical sexp VALUE) into the Rust expression that reconstructs
/// it, so a compound argument crosses into the emitted `pub fn` the way the Rust backend represents it.
///
/// The gate passes each arg as its canonical value text. A bare SCALAR (`20`, `-1`, `true`) is already a
/// valid Rust literal whose type the fn signature fixes, so it passes through verbatim. A COMPOUND value
/// must be rebuilt to match the backend's representation (mirroring `cdz_render_at`, the result side):
///  - `(tuple v0 v1 …)` → the Rust tuple `(e0, e1, …)`; a ONE-element tuple gets the trailing-comma form
///    `(e0,)` (Rust would otherwise read `(e0)` as a parenthesized scalar, not a 1-tuple).
///  - `(record (name val) …)` → a Rust tuple of the field values in SORTED-KEY order — the same canonical
///    order the backend lowers a record to (`(Record (x _) (y _))` → `(i64, i64)` with `x` first).
///
/// Anything else (a `list`/sum/`Some`/`Ok` arg, or a value shape not yet needed) passes through verbatim —
/// no regression: those constructs decline at the BACKEND today (list/sum results have no native Rust form),
/// so no trial reaches here relying on them, and a genuinely unhandled shape fails rustc exactly as before.
/// A `cdz_num::Big` constructor expression for an `i128` BigInt entry-arg value — mirrors the backend's
/// `const_big_expr`: in-i64 range → `Big::from_i64`, else `from_sign_magnitude_bytes(&[sign, LE-bytes…])`
/// (the runtime's canonical sign-magnitude leaf, little-endian). Keeps the driver's BigInt arg byte-identical
/// to the value the library body constructs, so the two compare equal.
fn big_arg_expr(n: i128) -> String {
    if let Ok(n64) = i64::try_from(n) {
        return format!("cdz_num::Big::from_i64({n64})");
    }
    let sign = if n < 0 { 1u8 } else { 0u8 };
    let mag = n.unsigned_abs();
    // Little-endian magnitude bytes, trimmed of trailing zeros (a canonical minimal form).
    let mut bytes = vec![sign];
    let le = mag.to_le_bytes();
    let last = le.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
    bytes.extend_from_slice(&le[..last]);
    let elems = bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("cdz_num::Big::from_sign_magnitude_bytes(&[{elems}])")
}

fn rust_call_arg(val: &str) -> String {
    let v = val.trim();
    // A FLOAT SPECIAL-VALUE literal (`nan`/`inf`/`-inf`) is not a Rust value token — map it to the `f64`
    // associated constant so it crosses as the right float. (The corpus writes these as bare words; a
    // finite float literal like `1.5` is already valid Rust, so it passes through below.) Only `f64` forms
    // appear as args today; a `Float32` NaN arg would need `f32::NAN` + a cast, but none occur.
    match v {
        "nan" | "NaN" => return "f64::NAN".to_string(),
        "inf" | "+inf" => return "f64::INFINITY".to_string(),
        "-inf" => return "f64::NEG_INFINITY".to_string(),
        _ => {}
    }
    // A STRING literal (`"abc"`) — the canonical value form of a `String` arg — must cross as an OWNED
    // `String`, because the emitted export's parameter is `s: String` (owned), NOT `&str`. A bare `"abc"`
    // Rust literal is a `&'static str`, so passing it directly is a type error (E0308) — the exported-entry
    // String-arg surface (breaker-found, corpus-bugfix; no corpus case passed a String ENTRY arg before, so
    // it was untested). Wrap it `"abc".to_string()`. The quoted form is unambiguously a String value (a
    // Cadenza string renders `"..."`); the inner escapes are already Rust-valid (cdz-run's string form and
    // Rust's share `\n`/`\t`/`\"`/`\\`), so the literal is emitted verbatim inside the `.to_string()`.
    if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
        return format!("{v}.to_string()");
    }
    // A NON-SCALAR entry arg must cross via the SAME construction form the emitted LIBRARY body uses for
    // that type — the corpus writes the RAW Cadenza literal/expr text (`100N`, `1R`, `((. Bytes of) …)`),
    // none of which is valid Rust, so the rust gate DRIVER's arg-emit must lower it (breaker-found CLUSTER;
    // the exported-entry non-scalar-arg surface was wholly untested — every String/BigInt/… input case built
    // the value INSIDE the program). Each form below mirrors `cdz_num`/the Vec builder the backend emits.
    //
    // A BIGINT literal is `<digits>N` (the `N` suffix). Cross as `cdz_num::Big` — `from_i64` for an in-i64
    // magnitude, else `from_sign_magnitude_bytes(&[sign, LE-bytes…])` (the same two-way split `const_big_expr`
    // uses). The digits are parsed to an `i128` first (covers well beyond i64); a magnitude past i128 would
    // need the byte form from the raw digits — no corpus arg reaches that, so parse-to-i128 suffices.
    if let Some(digits) = v.strip_suffix('N')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && let Ok(n) = digits.parse::<i128>()
    {
        return big_arg_expr(n);
    }
    // A RATIONAL literal is `<int>R` (an integer rational `n/1`) or `<n>/<d>` (a fraction). Cross as
    // `cdz_num::Rational::new(Big::from_i64(n), Big::from_i64(d))` — the same form the body emits for
    // `Rational.of`. `Rational::new` normalizes, so any equivalent spelling is fine.
    if let Some(int) = v.strip_suffix('R')
        && !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && let Ok(n) = int.parse::<i64>()
    {
        return format!(
            "cdz_num::Rational::new(cdz_num::Big::from_i64({n}), cdz_num::Big::from_i64(1))"
        );
    }
    if let Some((ns, ds)) = v.split_once('/')
        && let (Ok(n), Ok(d)) = (ns.trim().parse::<i64>(), ds.trim().parse::<i64>())
    {
        return format!(
            "cdz_num::Rational::new(cdz_num::Big::from_i64({n}), cdz_num::Big::from_i64({d}))"
        );
    }
    // A compound is a parenthesized head form; a bare token is a scalar literal → verbatim.
    let inner = match v.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        Some(inner) => inner.trim(),
        None => return v.to_string(),
    };
    // A BYTES value crosses as `((. Bytes of) (list <byte>…))` — the dotted `Bytes.of` member-access head.
    // Emit the same `Vec<u8>` the library body builds: `vec![<b>u8, …]`. Detect the dotted head before the
    // whitespace split (which would mis-tokenize the `(. Bytes of)` sub-form). ONLY marshal when the inner
    // `(list …)` shape is genuinely present — if it is absent (a malformed `((. Bytes of) …)`), FALL THROUGH
    // to pass-through-verbatim below so rustc/the backend REJECTS it, rather than silently defaulting to an
    // empty `vec![]` (which would compile a program with the WRONG argument value — a silent harness
    // miscompile, Copilot PR#507). `and_then` (not `map(...).unwrap_or_default()`) keeps the `None` path
    // falling through instead of collapsing to empty.
    if let Some(after) = inner.strip_prefix("(. Bytes of)")
        && let Some(elems) = after
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|s| s.trim().strip_prefix("list"))
            .map(|s| {
                split_top_level(s.trim())
                    .iter()
                    .map(|b| format!("{}u8", b.trim()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
    {
        return format!("vec![{elems}]");
    }
    let (head, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    match head {
        // A LIST value `(list e0 e1 …)` → Rust's `vec![<e0>, <e1>, …]` (the `Vec<T>` the library uses). Each
        // element is marshalled recursively (a list of tuples/sums composes). An empty `(list)` → `vec![]`.
        "list" => {
            let elems: Vec<String> = split_top_level(rest)
                .iter()
                .map(|e| rust_call_arg(e))
                .collect();
            format!("vec![{}]", elems.join(", "))
        }
        // A built-in Option/Result variant → Rust's own `Option`/`Result` (what the library emits). A
        // payload is marshalled recursively; `None` is nullary. Matches the backend's `SumNew` rendering.
        "Some" => format!("Some({})", rust_call_arg(rest)),
        "None" => "None".to_string(),
        "Ok" => format!("Ok({})", rust_call_arg(rest)),
        "Err" => format!("Err({})", rust_call_arg(rest)),
        "tuple" => {
            let elems: Vec<String> = split_top_level(rest)
                .iter()
                .map(|e| rust_call_arg(e))
                .collect();
            if elems.len() == 1 {
                format!("({},)", elems[0]) // 1-tuple: trailing comma so it isn't a paren-scalar.
            } else {
                format!("({})", elems.join(", "))
            }
        }
        "record" => {
            // A record value crosses in the SAME positional Rust tuple the backend emits (fields in canonical
            // SORTED-key order). The corpus writes a record arg in one of TWO surface forms:
            //  - NAMED-pair form `(record (a 3) (b 4))` — each element is a `(name value)` pair; sort by NAME
            //    so the positional tuple matches the backend's sorted-key field order.
            //  - POSITIONAL value form `(record 3 4)` — bare values ALREADY in field order (the `record` head
            //    is dropped by cdz-run's tuple-literal parser; several DIRECT-CALL record-arg cases use this).
            // Disambiguate by whether EVERY element is parenthesized: a named-pair element is `(a 3)`, a
            // positional scalar element is a bare `3`. (A positional element that is itself a compound would
            // also be parenthesized; the record-arg corpus is scalar-field, so this split is exact there.)
            let raw = split_top_level(rest);
            let all_pairs = !raw.is_empty() && raw.iter().all(|f| f.trim().starts_with('('));
            let elems: Vec<String> = if all_pairs {
                let mut fields: Vec<(String, String)> = raw
                    .iter()
                    .filter_map(|f| {
                        let f = f.trim();
                        let body = f.strip_prefix('(')?.strip_suffix(')')?.trim();
                        let (name, fval) = body.split_once(char::is_whitespace)?;
                        Some((name.trim().to_string(), rust_call_arg(fval)))
                    })
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                fields.into_iter().map(|(_, v)| v).collect()
            } else {
                // Positional value form — the bare values are already in field order.
                raw.iter().map(|e| rust_call_arg(e)).collect()
            };
            if elems.len() == 1 {
                format!("({},)", elems[0])
            } else {
                format!("({})", elems.join(", "))
            }
        }
        // Not a compound the harness rebuilds — pass through verbatim (declines at the backend if unsupported).
        _ => v.to_string(),
    }
}

/// Make a Cadenza SUM / VARIANT name a Rust identifier the SAME way the Rust backend's `types::sum_ident`
/// does — kept in lockstep so a sum VALUE that escapes renders through the enum the backend actually
/// emitted. A clean ident (valid Rust ident chars, not the mangle marker) passes through, except a Rust
/// PRIMITIVE type name (`i64`, `bool`, …) which is prefixed `cdz_ty_`; any lossy / leading-digit /
/// marker-prefixed name is hex-mangled `cdzsum_<hex-utf8>` (injective, so two distinct sum names never
/// collide into one emitted enum). Mirrors `types::sum_ident` + `sanitize_ident`'s keyword handling.
fn sum_rust_ident(name: &str) -> String {
    const MARKER: &str = "cdzsum_";
    let is_clean = {
        let mut chars = name.chars();
        matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
            && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
    };
    if is_clean && !name.starts_with(MARKER) {
        // A clean ident may be a Rust keyword (→ `r#kw`, or `cdz_kw_…` for the raw-ident exceptions) or a
        // primitive type name (→ `cdz_ty_…`); mirror the backend's escapes.
        let s = rust_ident(name);
        if matches!(s.as_str(), "crate" | "self" | "Self" | "super" | "_") {
            format!("cdz_kw_{s}")
        } else if is_rust_keyword_driver(&s) {
            format!("r#{s}")
        } else if is_rust_primitive_type_driver(&s) {
            format!("cdz_ty_{s}")
        } else {
            s
        }
    } else {
        let mut hex = String::with_capacity(name.len() * 2 + MARKER.len());
        hex.push_str(MARKER);
        for b in name.bytes() {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }
}

/// Rust reserved words — the driver's copy of the backend's `is_rust_keyword`, for `sum_rust_ident`.
fn is_rust_keyword_driver(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "gen"
    )
}

/// Rust primitive type names — the driver's copy of the backend's `is_rust_primitive_type`.
fn is_rust_primitive_type_driver(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

/// The export `name`'s CADENZA result type, read off the `// cdz-return[<ident>]: <type>` note the
/// backend emits before each fn (the type's `render_name`, e.g. `Int64`, `(Tuple Int64 Bool)`, `(Record
/// (a Int64) (b Int64))`). `None` if no matching note is found. The gate renders the result to cdz-run's
/// text form from THIS (the Cadenza type keeps field names + the Tuple/Record distinction the Rust type
/// erases). `name` is the export's SANITIZED ident, matching the note's `[<ident>]` tag.
fn cdz_return_type(module: &str, name: &str) -> Option<String> {
    let prefix = format!("// cdz-return[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The `// cdz-unit[<name>]: <value-form>` note for a QUANTITY export — the unit's canonical VALUE-form
/// s-expr (`Unit::render_value_form`, byte-identical to what cdz-run prints inside `((. Qty of) …)`). Only
/// a Qty-returning fn emits one; `None` for any other result. Spliced verbatim by the top-level Qty render.
fn cdz_unit_form(module: &str, name: &str) -> Option<String> {
    let prefix = format!("// cdz-unit[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The native Rust scalar type for a Cadenza scalar type-NAME (`render_name` form) — `Int64`→`i64`,
/// `UInt32`→`u32`, `Float64`→`f64`, `Float32`→`f32`, etc. Used by the Qty display-scale render to spell the
/// inner width the scale multiply runs in. Falls back to `i64` for an unrecognized name (only the aliased
/// integer/float widths reach a scaled Qty magnitude — the backend declines a non-aliased inner upstream).
fn rust_scalar_type_name(ty: &str) -> &'static str {
    match ty.trim() {
        "Int8" => "i8",
        "Int16" => "i16",
        "Int32" => "i32",
        "Int64" => "i64",
        "UInt8" => "u8",
        "UInt16" => "u16",
        "UInt32" => "u32",
        "UInt64" => "u64",
        "Float32" => "f32",
        "Float64" => "f64",
        _ => "i64",
    }
}

/// The `// cdz-scale[<name>]: <num>/<den>` note for a NON-scale-1 QUANTITY export — the unit's scale to its
/// dimension's reference, which the harness applies to the boundary magnitude so `5 kilometer` displays as
/// `5000 meter`. Only a Qty result at a non-reference unit emits one; `None` for a scale-1 (or non-Qty)
/// result (the magnitude is displayed as stored). Returns the `(num, den)` machine-integer ratio.
fn cdz_scale(module: &str, name: &str) -> Option<(i128, i128)> {
    let prefix = format!("// cdz-scale[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            let (n, d) = rest.trim().split_once('/')?;
            return Some((n.trim().parse().ok()?, d.trim().parse().ok()?));
        }
    }
    None
}

/// A Rust EXPRESSION that renders the driver's result binding `__r` (whose CADENZA type is `ty`, in
/// `render_name` form) to cdz-run's canonical text form — the value the gate grades against. Type-
/// directed and recursive over the Cadenza type:
///  - `Unit` → the token `unit`;
///  - `(Tuple T0 T1 …)` → `(tuple <r.0> <r.1> …)`;
///  - `(Record (a T0) (b T1) …)` → `(record (a <r.0>) (b <r.1>) …)` — the fields are already in sorted
///    order (both the type's render and the emitted Rust tuple order them the same), so element `i`
///    reads `.i`;
///  - any scalar (`Int64`, `Bool`, …) → `{}` (an integer/bool `Display`s exactly as cdz-run prints it).
fn cdz_render_expr(
    ty: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
    newtypes: &std::collections::HashMap<String, String>,
    sum_params: &std::collections::HashMap<String, usize>,
    unit_form: Option<&str>,
    unit_scale: Option<(i128, i128)>,
) -> String {
    let mut helpers = Vec::new();
    let mut on_path = Vec::new();
    let expr = cdz_render_at(
        ty,
        "__r",
        sums,
        newtypes,
        sum_params,
        unit_form,
        unit_scale,
        &mut helpers,
        &mut on_path,
    );
    // The recursive-sum render helpers (if any) are hoisted ahead of the expression, then the expression
    // is a block that defines them and evaluates. Each helper is a `fn`, so mutual/self recursion works.
    if helpers.is_empty() {
        expr
    } else {
        format!("{{ {} {expr} }}", helpers.join(" "))
    }
}

/// The recursive worker for [`cdz_render_expr`]: `path` is the Rust access path to the value being
/// rendered (starts at `__r`, descends `.0`/`.1`… into tuple/record elements — a record IS a positional
/// tuple in sorted-field order, so its `i`th field is `.i`).
///
/// `helpers` collects generated recursive render `fn`s (for a RECURSIVE user sum, whose TYPE unfolds
/// infinitely — `IntList = Cons(Tuple Int64 IntList) | Nil` — so it CANNOT be inlined without the codegen
/// itself never terminating). `on_path` is the set of user-sum idents currently being unfolded on the
/// recursion path: re-entering one means a cycle, so emit a CALL to its (runtime-recursive) helper fn
/// instead of inlining, moving the recursion from gate-codegen-time (over the infinite type) to Rust
/// runtime (over the FINITE value — a `Nil` leaf terminates it). Mirrors the wasm value-encode, which walks
/// the value, not the type.
#[allow(clippy::too_many_arguments)]
fn cdz_render_at(
    ty: &str,
    path: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
    newtypes: &std::collections::HashMap<String, String>,
    sum_params: &std::collections::HashMap<String, usize>,
    // The QUANTITY unit VALUE-form (`Unit::render_value_form`), present only when THIS `path` is a Qty and
    // its unit note reached here. Threaded ONLY to the TOP-LEVEL result (`__r`): the corpus has no Qty
    // nested inside a compound result, so a descent into a tuple/record/sum element passes `None`, and a
    // Qty reached with no form falls back to converting the type-string unit. Consumed by the Qty arm.
    unit_form: Option<&str>,
    // The NON-scale-1 quantity's `num/den` scale — present only for the TOP-LEVEL Qty result at a
    // non-reference unit; the Qty arm multiplies the boundary magnitude by it (Float `× num/den`, Int
    // `× num / den`). `None` for a scale-1 Qty (display = stored) and every non-top-level descent.
    unit_scale: Option<(i128, i128)>,
    helpers: &mut Vec<String>,
    on_path: &mut Vec<String>,
) -> String {
    let ty = ty.trim();
    if ty == "Unit" {
        return "\"unit\".to_string()".to_string();
    }
    // An erased NEWTYPE (`// cdz-newtype[Pt]: (Tuple Int64 Int64)`) — its runtime value IS the inner type
    // (the tag erased, `type-system.md §156`), and `Ty::Nominal`'s render_name is the bare name `Pt`. Render
    // by its INNER type so a `Pt`-typed boundary value renders structurally as `(tuple 5 5)` — NOT falling
    // to the scalar `Display` of the erased Rust tuple `(i64, i64)` (rustc E0277). Checked before the user-
    // sum arm (a newtype has no `cdz-sum` descriptor) and the scalar fallthrough.
    if let Some(inner) = newtypes.get(ty) {
        // Same value/path — a newtype-over-Qty forwards the unit form (the tag is erased, so a `Pt = Mk Qty`
        // renders as its inner quantity).
        return cdz_render_at(
            inner, path, sums, newtypes, sum_params, unit_form, unit_scale, helpers, on_path,
        );
    }
    // `(Tuple T0 T1 …)` → `(tuple …)`. The EMPTY tuple `(Tuple)` (a variant's explicit empty-tuple payload,
    // distinct from `Unit`) renders the literal `(tuple)` — no elements, no `path` read, and NO trailing
    // space (a `format!("(tuple {})", "")` would render `(tuple )`).
    if let Some(elems) = parse_head_type(ty, "Tuple") {
        if elems.is_empty() {
            return "\"(tuple)\".to_string()".to_string();
        }
        let placeholders = vec!["{}"; elems.len()].join(" ");
        let args: Vec<String> = elems
            .iter()
            .enumerate()
            .map(|(i, e)| {
                cdz_render_at(
                    e,
                    &format!("({path}).{i}"),
                    sums,
                    newtypes,
                    sum_params,
                    None,
                    None,
                    helpers,
                    on_path,
                )
            })
            .collect();
        return format!("format!(\"(tuple {placeholders})\", {})", args.join(", "));
    }
    // A record TYPE `(Record (a T0) (b T1) …)` → the VALUE form `(record (a …) (b …) …)`. Each element is
    // a `(name Type)` pair; the fields are in sorted order (matching the emitted tuple), so field `i` reads
    // `.i`. The head is matched CAPITALIZED — `Ty::render_name` writes a record type `(Record …)` (a1c9bc09,
    // matching its annotation spelling, distinct from the lowercase value constructor `(record …)`); the
    // EMITTED value form stays lowercase `(record …)`, cdz-run's canonical value spelling. (Was matched
    // lowercase `record`, which stopped matching after a1c9bc09 → a record return type fell through to the
    // scalar `Display` path and the emitted `(i64, i64)` tuple failed rustc E0277 "doesn't implement
    // Display", failing every record-escape case on the rust gate.)
    if let Some(fields) = parse_head_type(ty, "Record") {
        // Each field renders as `(<name> <value>)` — the name is a literal, the value is `.i` rendered
        // as its own type. The `format!` gets one `({} {})` group per field, args = name, value, ….
        let mut args = Vec::with_capacity(fields.len() * 2);
        for (i, field) in fields.iter().enumerate() {
            // `field` is `(name Type)` — strip its OUTER parens (exactly one each; `trim_end_matches(')')`
            // would wrongly eat a nested type's close paren, e.g. `(y (Tuple Int64 Int64))`), then split
            // the leading name from the rest.
            let f = field.trim();
            let inner = f
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(f)
                .trim();
            let (fname, fty) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
            args.push(format!("\"{}\"", fname.trim()));
            args.push(cdz_render_at(
                fty.trim(),
                &format!("({path}).{i}"),
                sums,
                newtypes,
                sum_params,
                None,
                None,
                helpers,
                on_path,
            ));
        }
        let groups = vec!["({} {})"; fields.len()].join(" ");
        return format!("format!(\"(record {groups})\", {})", args.join(", "));
    }
    // A `(List T)` value is the Rust `Vec<T>` the backend emits — render it as cdz-run's canonical
    // `(list e0 e1 …)` (empty → `(list)`), each element rendered as its own type `T`. Emit a Rust block
    // that folds the elements into a `String`: iterate `&<path>` (borrow — the value may be read only
    // here), render each element via a FRESH binder, and join under the `(list …)` wrapper. The element
    // binder `__e{depth}` (keyed on path length) avoids a shadow-capture when the element is itself a
    // list/sum. `.iter()` yields `&T`; the recursive render reads the binder, and default ref binding
    // makes a `&i64`/`&(…)` `Display`/index fine, exactly as the Option/Result payload render relies on.
    let ebind = format!("__e{}", path.len());
    if let Some(args) = parse_head_type(ty, "List") {
        let elem_ty = args.first().map(String::as_str).unwrap_or("");
        let inner = cdz_render_at(
            elem_ty, &ebind, sums, newtypes, sum_params, None, None, helpers, on_path,
        );
        // Build `(list <e0> <e1> …)`: seed with "(list", push a space + each element's render, close ")".
        return format!(
            "{{ let mut __s = String::from(\"(list\"); for {ebind} in ({path}).iter() {{ __s.push(' '); __s.push_str(&({inner})); }} __s.push(')'); __s }}"
        );
    }
    // A `(Set E)` value is the Rust `BTreeSet<E>` the backend emits — render it as cdz-run's canonical
    // `((. Set of) (list e0 e1 …))` (empty → `((. Set of) (list))`), each element rendered as its type.
    // A `BTreeSet` iterates in SORTED order, which IS the canonical element-value order the runtime uses.
    if let Some(args) = parse_head_type(ty, "Set") {
        let elem_ty = args.first().map(String::as_str).unwrap_or("");
        let inner = cdz_render_at(
            elem_ty, &ebind, sums, newtypes, sum_params, None, None, helpers, on_path,
        );
        return format!(
            "{{ let mut __s = String::from(\"((. Set of) (list\"); for {ebind} in ({path}).iter() {{ __s.push(' '); __s.push_str(&({inner})); }} __s.push_str(\"))\"); __s }}"
        );
    }
    // A `(Map K V)` value is the Rust `BTreeMap<K, V>` the backend emits — render it as cdz-run's canonical
    // `(map (k0 v0) (k1 v1) …)` (empty → `(map)`), each entry a `(<key> <value>)` group with key and value
    // rendered as their own types. A `BTreeMap` iterates in SORTED KEY order — the canonical key order.
    if let Some(args) = parse_head_type(ty, "Map") {
        let key_ty = args.first().map(String::as_str).unwrap_or("");
        let val_ty = args.get(1).map(String::as_str).unwrap_or("");
        let kbind = format!("__mk{}", path.len());
        let vbind = format!("__mv{}", path.len());
        let kr = cdz_render_at(
            key_ty, &kbind, sums, newtypes, sum_params, None, None, helpers, on_path,
        );
        let vr = cdz_render_at(
            val_ty, &vbind, sums, newtypes, sum_params, None, None, helpers, on_path,
        );
        return format!(
            "{{ let mut __s = String::from(\"(map\"); for ({kbind}, {vbind}) in ({path}).iter() {{ __s.push_str(&format!(\" ({{}} {{}})\", {kr}, {vr})); }} __s.push(')'); __s }}"
        );
    }
    // A QUANTITY result `(Qty <inner> <unit>)` — the rust backend maps a `Ty::Qty { inner }` at a scale-1
    // unit to its INNER magnitude's type (the wrapper erases), so `{path}` is the magnitude. cdz-run renders
    // it `((. Qty of) <magnitude> <unit-value-form>)`: render the magnitude by its inner type, then splice
    // the unit's VALUE-form s-expr. The canonical value form (`((. Unit base) …)`, a `Unit./` quotient for a
    // derived unit) comes from the backend's `// cdz-unit` note (`unit_form`) — byte-identical to cdz-run.
    // Scale-1 only reaches here, so the stored magnitude IS the displayed one (no scaling in the render).
    if let Some(args) = parse_head_type(ty, "Qty") {
        let inner_ty = args.first().map(String::as_str).unwrap_or("");
        // Prefer the backend's value-form note (handles EVERY unit shape — base, power, product, quotient).
        // Fall back to converting the type-string unit (a Qty reached WITHOUT a note — only the simple
        // base/dimensionless/single-positive-power shapes the type-string convert covers; the corpus has no
        // note-less Qty result, so the fallback is belt-and-suspenders for a nested Qty a future case adds).
        let unit = match unit_form {
            Some(v) => v.to_string(),
            None => match args.get(1).map(String::as_str).unwrap_or("") {
                "Unit.one" => "(. Unit one)".to_string(),
                u if u.starts_with("(Unit.base ") => {
                    format!("((. Unit base) {}", &u["(Unit.base ".len()..])
                }
                u if u.starts_with("(Unit.^ (Unit.base ") => {
                    u.replacen("(Unit.base ", "((. Unit base) ", 1)
                }
                other => other.to_string(),
            },
        };
        let unit_lit = unit.replace('\\', "\\\\").replace('"', "\\\"");
        // A NON-scale-1 unit DISPLAY-SCALES the stored magnitude to its reference (`5 km` → `5000 m`): the
        // backend crosses the RAW magnitude + a `// cdz-scale` note (`unit_scale`), and the display multiply
        // happens HERE, in the inner numeric type, mirroring the wasm boundary value-encode
        // (`const_value_ast_scaled`): a Float multiplies as f64 (`v * num as f64 / den as f64`, IEEE rounds);
        // an Int multiplies then integer-divides (`v * num / den`, truncates toward zero). Only Float/Int
        // inners reach here with a scale (the backend's `qty_scale_supported` gate declines a Rational/BigInt
        // non-scale-1, which needs exact rational scaling). A scale-1 Qty has `unit_scale == None` → the raw
        // path renders unchanged. The scaled value binds to a fresh local `__q` of the inner type so the
        // recursive render reads a plain scalar (not the arithmetic expression). Wrap the scaled expr in its
        // inner-typed `let` so the `* / ` runs in the right width (an i64 magnitude scales as i64, etc.).
        let scaled_path = match unit_scale {
            Some((num, den)) => {
                let bind = format!("__q{}", path.len());
                let (letbind_ty, scaled_expr) = if inner_ty.trim() == "Rational" {
                    // A RATIONAL magnitude scales EXACTLY (no rounding): multiply by the scale as a Rational
                    // `num/den` — `Rational::mul` normalizes to lowest terms, so `5 mile` = `5/1 · 201168/125`
                    // = `201168/25 meter` exactly. `Big::from_i64` builds the ratio's limbs (a real
                    // prefix/family scale fits i64). The exact twin of the Float(rounds)/Int(truncates) cases.
                    (
                        "cdz_num::Rational".to_string(),
                        format!(
                            "({path}).mul(&cdz_num::Rational::new(cdz_num::Big::from_i64({num}i64), cdz_num::Big::from_i64({den}i64)))"
                        ),
                    )
                } else {
                    // The magnitude's Rust scalar type (`Float64`→`f64`, `Int64`→`i64`, `UInt32`→`u32`, …).
                    let inner_rust = rust_scalar_type_name(inner_ty);
                    // Float multiplies in f64/f32 (num/den cast to the float type, IEEE rounds); an integer
                    // multiplies then truncating-divides in its own width (a `{n}{ty}` literal fixes width).
                    let expr = if inner_rust == "f64" || inner_rust == "f32" {
                        format!("(({path}) * ({num} as {inner_rust}) / ({den} as {inner_rust}))")
                    } else {
                        format!("(({path}) * ({num}{inner_rust}) / ({den}{inner_rust}))")
                    };
                    (inner_rust.to_string(), expr)
                };
                let letbind = format!("let {bind}: {letbind_ty} = {scaled_expr};");
                Some((bind, letbind))
            }
            None => None,
        };
        // The magnitude renders by its inner type; a Qty nested in the magnitude position is impossible (an
        // inner is a numeric scalar), so pass `None` for the descent's unit form + scale.
        let render_path = scaled_path
            .as_ref()
            .map(|(b, _)| b.as_str())
            .unwrap_or(path);
        let inner = cdz_render_at(
            inner_ty,
            render_path,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            helpers,
            on_path,
        );
        let body = format!("format!(\"((. Qty of) {{}} {unit_lit})\", {inner})");
        return match scaled_path {
            Some((_, letbind)) => format!("{{ {letbind} {body} }}"),
            None => body,
        };
    }
    // A `BigInt` value is the Rust `cdz_num::Big` the backend emits — render it as its exact decimal, the
    // BARE integer text cdz-run prints for a BigInt (`42`, `-58`), via `Big::to_decimal_string`. `{path}`
    // is a `Big`/`&Big`; the method takes `&self`, so a reference works. Matches the runtime's BigInt
    // value-encode (a sign-magnitude leaf rendered as its decimal).
    if ty == "BigInt" {
        return format!("({path}).to_decimal_string()");
    }
    // A `Rational` value is the Rust `cdz_num::Rational` the backend emits — render it as cdz-run's `n/d`
    // form (`1/2`, `5/1`, `-3/4`), via `Rational::to_display_string`. It is kept in lowest terms with the
    // sign on the numerator + a positive denominator, so the string matches the oracle (an integer-valued
    // rational still shows the explicit `/1`).
    if ty == "Rational" {
        return format!("({path}).to_display_string()");
    }
    // A `String` value is the Rust `String` the backend emits — render it as cdz-run's canonical
    // `"<content>"` form: the RAW UTF-8 content wrapped in double quotes, with NO escaping (matching the
    // runtime's `Shape::Str => format!("\"{}\"", …)` — a raw passthrough). `{path}` is a `String`/`&String`;
    // `format!` displays it verbatim between the quotes.
    if ty == "String" {
        return format!("format!(\"\\\"{{}}\\\"\", {path})");
    }
    // A `Symbol` value is the Rust `String` the backend emits (a Symbol IS its canonical text) — render it
    // as cdz-run's canonical CONSTRUCTION form `((. Symbol of) "<content>")` (`lower::const_value_ast`'s
    // Symbol surface; the gate accepts the bare value or the `(: value type)` form, so the value form
    // suffices). The content is the raw UTF-8 between quotes, no escaping (matching the `String` render's
    // raw passthrough). `{path}` is a `String`/`&String`.
    if ty == "Symbol" {
        return format!("format!(\"((. Symbol of) \\\"{{}}\\\")\", {path})");
    }
    // A `Char` value is the Rust `char` the backend emits — render it as cdz-run's canonical `#\<…>` form,
    // matching `cadenza-syntax`'s `literal::render_char`: the named specials (`#\space`/`#\newline`/`#\tab`/
    // `#\return`/`#\null`), a control scalar → `#\u+HHHH` (uppercase hex, ≥4 digits), else `#\<char>`. The
    // emitted Rust block matches `char` against those cases. `{path}` is a `char`/`&char`; the block binds
    // an owned `char` via `.clone()` (a `&char` clones to `char`; `char` is Copy+Clone).
    if ty == "Char" {
        // `.clone()` (not a bare bind): `{path}` may be a `char` value OR a `&char` payload binder; `char`
        // is Copy+Clone, so `.clone()` yields an owned `char` from either (a bare `let __c: char = &char`
        // would fail).
        return format!(
            "{{ let __c: char = ({path}).clone(); match __c {{ \
             ' ' => \"#\\\\space\".to_string(), \
             '\\n' => \"#\\\\newline\".to_string(), \
             '\\t' => \"#\\\\tab\".to_string(), \
             '\\r' => \"#\\\\return\".to_string(), \
             '\\0' => \"#\\\\null\".to_string(), \
             __c if __c.is_control() => format!(\"#\\\\u+{{:04X}}\", __c as u32), \
             __c => format!(\"#\\\\{{}}\", __c), \
             }} }}"
        );
    }
    // A `Bytes` value is the Rust `Vec<u8>` the backend emits — render it as cdz-run's canonical `b"…"`
    // form, escaping each byte with the SAME rules as the runtime's `escape_byte`: `\n`/`\r`/`\t`/`\\`/`\"`
    // named, `\0`, printable ASCII `0x20..=0x7e` passthrough, else `\xHH` (lowercase hex). The emitted Rust
    // block folds the bytes into the `b"…"` string. (`{path}` is a `Vec<u8>`/`&Vec<u8>`; `.iter()` yields
    // `&u8`.)
    if ty == "Bytes" {
        return format!(
            "{{ let mut __s = String::from(\"b\\\"\"); for &__byte in ({path}).iter() {{ match __byte {{ \
             b'\\n' => __s.push_str(\"\\\\n\"), \
             b'\\r' => __s.push_str(\"\\\\r\"), \
             b'\\t' => __s.push_str(\"\\\\t\"), \
             b'\\\\' => __s.push_str(\"\\\\\\\\\"), \
             b'\\\"' => __s.push_str(\"\\\\\\\"\"), \
             0 => __s.push_str(\"\\\\0\"), \
             0x20..=0x7e => __s.push(__byte as char), \
             b => __s.push_str(&format!(\"\\\\x{{:02x}}\", b)), \
             }} }} __s.push('\\\"'); __s }}"
        );
    }
    // The BUILT-IN `Option`/`Result` map to Rust's OWN `Option`/`Result`, so a value of one is rendered by
    // MATCHING it — the driver knows both variant shapes (`Some`/`None`, `Ok`/`Err`) and cdz-run's canonical
    // BARE form for a built-in variant (`(Some <p>)`, `(None unit)`, `(Ok <p>)`, `(Err <p>)`). The payload
    // types come from the head's type ARGS (`(Option A)` → the `Some` payload is `A`; `(Result A B)` → `Ok`
    // is `A`, `Err` is `B`), each rendered recursively, so a nested `(Option (Option Int64))` or an
    // `(Option (Tuple …))` composes. Matching `&<path>` borrows (the value may be used only here) and relies
    // on default binding modes (the payload binder is a reference, which `Display`s / indexes fine). A
    // FRESH binder per match depth (`__v{depth}`, derived from the path length) avoids a shadow-capture when
    // a payload is itself a sum.
    let vbind = format!("__v{}", path.len());
    if let Some(args) = parse_head_type(ty, "Option") {
        let payload = args.first().map(String::as_str).unwrap_or("");
        let inner = cdz_render_at(
            payload, &vbind, sums, newtypes, sum_params, None, None, helpers, on_path,
        );
        return format!(
            "match &{path} {{ Some({vbind}) => format!(\"(Some {{}})\", {inner}), None => \"(None unit)\".to_string() }}"
        );
    }
    if let Some(args) = parse_head_type(ty, "Result") {
        let ok = cdz_render_at(
            args.first().map(String::as_str).unwrap_or(""),
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            helpers,
            on_path,
        );
        let err = cdz_render_at(
            args.get(1).map(String::as_str).unwrap_or(""),
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            helpers,
            on_path,
        );
        return format!(
            "match &{path} {{ Ok({vbind}) => format!(\"(Ok {{}})\", {ok}), Err({vbind}) => format!(\"(Err {{}})\", {err}) }}"
        );
    }
    // A GENERIC user sum at a concrete instantiation — `(Box Int64)`, a HEAD-APPLIED type whose head names
    // a sum with a `// cdz-sum-params[Head]: N` note. Its descriptor's payload tokens carry `T{k}`
    // placeholders (`(W T0) (E)`); substitute the instantiation's args (`[Int64]`) for the placeholders,
    // then render INLINE via a `match` (no helper `fn`, so no Rust generic type signature to spell — Rust
    // infers `Box<i64>` from the matched value). This is what lets a generic-sum boundary value render on
    // the rust gate the way it does on wasm; without it a generic-sum escape fell to the scalar path and
    // failed rustc E0277 ("doesn't implement Display").
    if let Some((head, args)) = parse_applied_type(ty)
        && let Some(variants) = sums.get(&head)
        && sum_params.get(&head).copied().unwrap_or(0) == args.len()
        && !args.is_empty()
    {
        let vbind = format!("__g{}", path.len());
        // The emitted enum ident (escaped/mangled the SAME way the backend's `sum_ident` does), used in the
        // `prog::<Enum>::` path; the PRINTED name stays the Cadenza `head`/`vname`.
        let head_ident = sum_rust_ident(&head);
        let mut arms = Vec::with_capacity(variants.len());
        for (vname, payloads) in variants {
            let vident = sum_rust_ident(vname);
            // Substitute the instantiation args for the `T{k}` placeholders in each payload token.
            let subst: Vec<String> = payloads
                .iter()
                .map(|p| subst_type_params(p, &args))
                .collect();
            match subst.len() {
                0 => arms.push(format!(
                    "prog::{head_ident}::{vident} => \"({vname} unit)\".to_string()"
                )),
                1 => {
                    let inner = cdz_render_at(
                        &subst[0], &vbind, sums, newtypes, sum_params, None, None, helpers, on_path,
                    );
                    arms.push(format!(
                        "prog::{head_ident}::{vident}({vbind}) => format!(\"({vname} {{}})\", {inner})"
                    ));
                }
                n => {
                    let placeholders = vec!["{}"; n].join(" ");
                    let parts: Vec<String> = subst
                        .iter()
                        .enumerate()
                        .map(|(i, pty)| {
                            cdz_render_at(
                                pty,
                                &format!("({vbind}).{i}"),
                                sums,
                                newtypes,
                                sum_params,
                                None,
                                None,
                                helpers,
                                on_path,
                            )
                        })
                        .collect();
                    arms.push(format!(
                        "prog::{head_ident}::{vident}({vbind}) => format!(\"({vname} {placeholders})\", {})",
                        parts.join(", ")
                    ));
                }
            }
        }
        return format!("match &{path} {{ {} }}", arms.join(", "));
    }
    // A USER sum — a bare type name (`Opt`, `P`, `E`) with an emitted `// cdz-sum[…]` descriptor giving its
    // variants (name + payload type) in discriminant order. Render by MATCHING into cdz-run's BARE form,
    // uniform with a built-in sum: a payload variant → `(<Variant> <payload>)` (payload rendered
    // recursively from its type); a nullary variant → `(<Variant> unit)`. The Rust variant identifier is
    // the SANITIZED name (matching the emitted enum); the printed name is the CADENZA variant name (the
    // descriptor's first token). A MONOMORPHIC user sum is a bare name here; a GENERIC one is handled by the
    // head-applied arm above.
    if let Some(variants) = sums.get(ty) {
        // The enum is defined INSIDE `mod prog { … }` (the driver wraps the emitted module), so the
        // driver's `fn main` names it qualified: `prog::<Enum>::<Variant>`. (A built-in Option/Result is
        // std's, unqualified — handled above.)
        //
        // A user sum is rendered through a generated recursive helper `fn __render_<Ident>(v: &prog::Ident)
        // -> String`, NOT inlined. This is what makes a RECURSIVE sum terminate: `IntList = Cons(Tuple Int64
        // IntList) | Nil` unfolds infinitely as a TYPE, so inlining `cdz_render_at` for each payload never
        // returns (the codegen itself diverges → stack overflow building the render expression). Routing
        // through a helper moves the recursion to Rust RUNTIME over the finite value: the helper matches the
        // variants, and a self-referential payload position emits a CALL to the same helper (because the sum
        // is on `on_path` when its payloads are rendered), so a `Nil` leaf terminates the runtime walk.
        // The emitted enum ident (escaped/mangled the SAME way the backend's `sum_ident` does), used in the
        // `prog::<Enum>` path + the helper name; the PRINTED name stays the Cadenza `ty`/`vname`.
        let ty_ident = sum_rust_ident(ty);
        let fn_name = format!("__render_{ty_ident}");
        if !on_path.iter().any(|s| s == ty) {
            // First time this sum is unfolded on the path — generate its helper (once; a later occurrence
            // reuses it). Push the name onto the path so a self-reference inside a payload emits a call.
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                on_path.push(ty.to_string());
                // A variant's DISPLAY HEAD (including the opening paren) in the canonical value form. Most
                // user/prelude sums render a variant BARE — `(Cons …`, `(Pos …`. But the prelude reflection
                // sum `Ast` renders QUALIFIED — `((. Ast Int) …`, `((. Ast Name) …` — because its variant
                // names (`Int`, `List`, `Bool`, `Float`) collide with prelude/type names, so the canonical
                // form disambiguates via member access (matching cdz-run + the wasm gate for `Ast`;
                // `Sign`/`Ordering`, whose variants don't collide, stay bare). Keyed on the sum name `Ast` —
                // the one reflection value-type whose escape form is qualified. Every arm then emits
                // `{head} <payload…>)` uniformly (nullary → `{head} unit)`).
                let disp_head = |vname: &str| -> String {
                    if ty == "Ast" {
                        format!("((. {ty} {vname})")
                    } else {
                        format!("({vname}")
                    }
                };
                let mut arms = Vec::with_capacity(variants.len());
                for (vname, payloads) in variants {
                    let vident = sum_rust_ident(vname);
                    let head = disp_head(vname);
                    match payloads.len() {
                        // A nullary variant → `{head} unit)`.
                        0 => arms.push(format!(
                            "prog::{ty_ident}::{vident} => \"{head} unit)\".to_string()"
                        )),
                        // A single-payload variant → `(Name <payload>)`, the payload rendered from `__p`
                        // (its own type — a scalar, tuple, record, or nested sum; kept nested if a tuple).
                        1 => {
                            let inner = cdz_render_at(
                                &payloads[0],
                                "__p",
                                sums,
                                newtypes,
                                sum_params,
                                None,
                                None,
                                helpers,
                                on_path,
                            );
                            arms.push(format!(
                                "prog::{ty_ident}::{vident}(__p) => format!(\"{head} {{}})\", {inner})"
                            ));
                        }
                        // A MULTI-payload variant → `(Name e0 e1 …)` SPREAD FLAT. Its N payloads box as ONE
                        // Rust tuple field (`P((i64, Option<i64>))`), so bind that tuple `__p` and render
                        // each element `(__p).i` by its own payload type — the flat form the wasm value-
                        // encode produces (`(P 5 (Some 5))`), NOT the nested `(P (tuple 5 (Some 5)))`.
                        n => {
                            let placeholders = vec!["{}"; n].join(" ");
                            let parts: Vec<String> = payloads
                                .iter()
                                .enumerate()
                                .map(|(i, pty)| {
                                    cdz_render_at(
                                        pty,
                                        &format!("(__p).{i}"),
                                        sums,
                                        newtypes,
                                        sum_params,
                                        None,
                                        None,
                                        helpers,
                                        on_path,
                                    )
                                })
                                .collect();
                            arms.push(format!(
                                "prog::{ty_ident}::{vident}(__p) => format!(\"{head} {placeholders})\", {})",
                                parts.join(", ")
                            ));
                        }
                    }
                }
                on_path.pop();
                // `#[allow(unused)]` — a mutually-referenced helper may be defined but only reached via
                // another; the block-hoisting emits every generated fn, some unused on a given path.
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(__v: &prog::{ty_ident}) -> String {{ match __v {{ {} }} }}",
                    arms.join(", ")
                ));
            }
        }
        // A borrow — the value may be used only here; the helper takes `&prog::Ident`.
        return format!("{fn_name}(&{path})");
    }
    // A FLOAT (`Float32`/`Float64`) renders via cdz-run's canonical `display_float`, NOT Rust's `{}`:
    // a whole float is `N.0` (Rust's `{}` prints `42`, the corpus wants `42.0`), `-0.0` and `NaN` are
    // named. Inline the exact `display_float` logic (widening a Float32 to f64 first) so the Rust-gate
    // render matches the value form the wasm gate + cdz-run produce.
    if ty == "Float64" || ty == "Float32" {
        // `.clone() as f64` (not a bare `as f64`): the path may be a VALUE (`.0`, top-level `__r`) OR a
        // `&f64` reference (a payload binder in a sum-render helper `match &v { Enum::Float(__p) => … }`
        // binds `__p: &f64`), and `(&f64) as f64` is an invalid reference cast (E0606). `f64: Clone` +
        // autoref makes `.clone()` yield an owned `f64` from either, then `as f64` is a no-op / a
        // Float32→f64 widen. (Surfaced when `Ast` — a sum with a `Float` payload — became renderable once
        // its `String` payload got a rep; the helper then hit the `&f64` cast.)
        return format!(
            "{{ let __f = ({path}).clone() as f64; \
             if __f == 0.0 && __f.is_sign_negative() {{ \"-0.0\".to_string() }} \
             else if __f.is_nan() {{ \"NaN\".to_string() }} \
             else if __f.fract() == 0.0 && __f.is_finite() {{ format!(\"{{:.0}}.0\", __f) }} \
             else {{ format!(\"{{}}\", __f) }} }}"
        );
    }
    // A scalar: Display it.
    format!("format!(\"{{}}\", {path})")
}

/// Parse the `// cdz-sum[<Ident>]: (<Variant> <payload-render>) (<Nullary>) …` descriptor notes the Rust
/// backend emits into a map `Ident → [(variant-name, Some(payload-type) | None)]`, variants in
/// discriminant order. The gate driver reads this to `match` a USER-sum boundary value into its canonical
/// bare form (the enum decl gives rustc the type; this gives the driver the variant structure the plain
/// return-type name erases). Only monomorphic user sums have a descriptor (see `emit_sum_descriptors`).
fn cdz_sum_descriptors(
    module: &str,
) -> std::collections::HashMap<String, Vec<(String, Vec<String>)>> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-sum[") else {
            continue;
        };
        let Some((ident, groups)) = rest.split_once("]:") else {
            continue;
        };
        // Each top-level `(…)` group is one variant: its first token is the Cadenza variant name, and the
        // remaining top-level tokens are its payload type render_names — ZERO (nullary), ONE (single), or N
        // (a MULTI-payload variant, whose N payloads the harness renders SPREAD FLAT). `split_top_level`
        // respects nesting, so a payload that is itself a `(Tuple …)`/`(record …)`/`(Option …)` stays one
        // token; the token COUNT is the variant's arity (a single `(Tuple …)` token = one tuple payload,
        // kept nested; N tokens = a multi-payload variant, spread).
        let variants: Vec<(String, Vec<String>)> = split_top_level(groups.trim())
            .iter()
            .filter_map(|g| {
                let inner = g.strip_prefix('(')?.strip_suffix(')')?.trim();
                let toks = split_top_level(inner);
                let (name, payloads) = toks.split_first()?;
                Some((name.trim().to_string(), payloads.to_vec()))
            })
            .collect();
        map.insert(ident.trim().to_string(), variants);
    }
    map
}

/// Parse the `// cdz-sum-params[<Ident>]: N` notes into a map `Ident → parameter count`. A GENERIC user
/// sum emits one so the driver knows how many `T{k}` placeholders its descriptor's payloads carry (hence
/// how many concrete args to bind from the result type). A monomorphic sum emits none (count 0, absent).
fn cdz_sum_params(module: &str) -> std::collections::HashMap<String, usize> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-sum-params[") else {
            continue;
        };
        if let Some((ident, n)) = rest.split_once("]:")
            && let Ok(count) = n.trim().parse::<usize>()
        {
            map.insert(ident.trim().to_string(), count);
        }
    }
    map
}

/// Split a HEAD-APPLIED type `(<Head> <Arg>…)` into `(head, args)` — `"(Box Int64)"` → `("Box", ["Int64"])`,
/// `"(M Int64 Bool)"` → `("M", ["Int64", "Bool"])`. `None` if `ty` is not a parenthesized head-applied form
/// (a bare name like `Box`, or a scalar). Respects nesting via `split_top_level` (an arg that is itself a
/// `(Option …)` stays one arg). Used to recognize a generic-sum instantiation at the render site.
fn parse_applied_type(ty: &str) -> Option<(String, Vec<String>)> {
    let inner = ty.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let toks = split_top_level(inner);
    let (head, args) = toks.split_first()?;
    Some((head.trim().to_string(), args.to_vec()))
}

/// Substitute the type-parameter placeholders `T0`, `T1`, … in a descriptor payload token with the concrete
/// instantiation `args` — `"T0"` with `["Int64"]` → `"Int64"`; `"(Option T0)"` → `"(Option Int64)"`. A
/// placeholder is a WHOLE token `T{k}` (a nested one inside `(Option T0)` is replaced by a bounded scan
/// over `Tk` word-boundaries). Only `k < args.len()` is substituted; a `T{k}` out of range is left as-is
/// (should not occur — the param count matches).
fn subst_type_params(payload: &str, args: &[String]) -> String {
    let mut s = payload.to_string();
    // Replace longest index first (T10 before T1) so a prefix match doesn't corrupt a two-digit index.
    for k in (0..args.len()).rev() {
        let placeholder = format!("T{k}");
        // Replace `Tk` only at token boundaries — surrounded by start/end, whitespace, or parens — so a
        // type name that merely CONTAINS "T0" is not mangled. Rebuild by scanning tokens split on the
        // structural chars; simplest robust form: replace `(Tk)`, ` Tk`, `Tk `, and a bare whole `Tk`.
        if s == placeholder {
            s = args[k].clone();
            continue;
        }
        s = s
            .replace(&format!("({placeholder} "), &format!("({} ", args[k]))
            .replace(&format!(" {placeholder})"), &format!(" {})", args[k]))
            .replace(&format!(" {placeholder} "), &format!(" {} ", args[k]))
            .replace(&format!("({placeholder})"), &format!("({})", args[k]));
    }
    s
}

/// Parse the `// cdz-newtype[<Ident>]: <inner-render-name>` descriptor notes into a map `Ident → inner
/// type`. An erased newtype's runtime value IS its inner type (the tag adds nothing), so the gate renders a
/// newtype-typed boundary value by its inner type — see [`cdz_render_at`]'s newtype arm.
fn cdz_newtype_descriptors(module: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-newtype[") else {
            continue;
        };
        if let Some((ident, inner)) = rest.split_once("]:") {
            map.insert(ident.trim().to_string(), inner.trim().to_string());
        }
    }
    map
}

/// Split a `render_name` head-applied type `(<Head> A B …)` into its argument strings, or `None` if `ty`
/// is not `(<Head> …)`. Respects nesting — a space or paren inside a nested `(…)` group does not split.
/// Used to destructure `(Tuple T0 T1)` and `(Record (a T0) (b T1))`.
fn parse_head_type(ty: &str, head: &str) -> Option<Vec<String>> {
    let inner = ty.strip_prefix('(')?.strip_suffix(')')?.trim();
    let rest = inner.strip_prefix(head)?;
    // `head` must be a WHOLE token: either it is the entire content — `(Tuple)`, the empty tuple, zero args
    // — or it is followed by whitespace before its args (`(Tuple T0 …)`). The whitespace check alone would
    // reject the exact-match empty case `(Tuple)` (rest is ""), so a `(Tuple)` return type fell through to a
    // scalar `Display` of the erased Rust `()` → E0277; and it must still reject a hypothetical `(TupleX …)`
    // (rest starts with `X`, neither empty nor whitespace-led).
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(split_top_level(rest.trim()))
}

/// Split a string into top-level whitespace-separated groups, treating a balanced `(…)` as one group.
/// `"a (Tuple Int64 Bool) c"` → `["a", "(Tuple Int64 Bool)", "c"]`.
fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => {
                if depth == 0 && start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b')' => depth -= 1,
            _ if b.is_ascii_whitespace() && depth == 0 => {
                if let Some(st) = start.take() {
                    out.push(s[st..i].to_string());
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        out.push(s[st..].to_string());
    }
    out
}

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

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
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
fn first_error_code(stderr: &[u8]) -> Option<String> {
    for line in String::from_utf8_lossy(stderr).lines() {
        // Match `… error [CODE]…` — a typed ERROR (not a warning). The code is between `[` and `]`.
        if let Some(rest) = line.split_once("error [").map(|(_, r)| r)
            && let Some(code) = rest.split(']').next()
            && !code.is_empty()
        {
            return Some(code.trim().to_string());
        }
    }
    None
}

/// Options for `gate` (grows without re-threading a widening arg list).
struct GateOpts {
    files: Vec<PathBuf>,
    store: Option<PathBuf>,
    case: Option<String>,
    save: bool,
    check: bool,
    target: GateTarget,
}

/// Run one or more corpus files through the pipeline and grade each case against its recorded
/// outcome. Delegates case parsing + normalization to `cdz-syntax corpus`, then drives each program.
fn gate(paths: &Paths, profile: &str, opts: GateOpts) {
    let tools = build_tools(paths, profile);
    let files = if opts.files.is_empty() {
        default_corpus_files(paths)
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
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools, file))
        .collect();
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
        std::process::exit(check_baseline(paths, &verdicts, opts.target));
    }
    if fail > 0 {
        std::process::exit(1);
    }
}

/// The observable-outcome KEY the opt-sweep compares across optimization levels — the projection every
/// level must preserve. Two runs are level-EQUIVALENT iff their keys are equal; a divergence in this key
/// across O0..O3 is a candidate miscompile. A value carries its host-call trace; a trap keys on its
/// classified KIND ([`trap_kind`]) or — for an UNCLASSIFIED reason — the RAW first line (NOT a single
/// "other" bucket, else two genuinely-different unclassified traps would compare EQUAL and a real
/// cross-level trap divergence among them would be missed). Extracted as a free fn so the comparison the
/// blocking gate rests on is unit-tested (see `opt_sweep_outcome_key_*` tests).
fn sweep_outcome_key(ran: &Ran) -> String {
    match ran {
        Ran::Value(v, calls) if calls.is_empty() => format!("value {v}"),
        Ran::Value(v, calls) => format!("value {v} [host: {}]", calls.join(",")),
        Ran::Trap(msg) => match trap_kind(msg) {
            Some(kind) => format!("trap {kind}"),
            None => format!("trap raw:{}", first_line(msg.as_bytes())),
        },
        Ran::Declined { code } => format!("declined {}", code.as_deref().unwrap_or("-")),
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
/// cases are skipped in this first cut (the single-file pipe covers the arithmetic/control programs where
/// the pass tiers act). Wired into `cargo xtask check` (operator directive 2026-07-17) → a HARD BLOCKING
/// merge gate: because pr-sync re-gates every MR via `check`, a cross-level divergence rejects the MR. It
/// guards nothing while the `PassManager` pipeline is empty (every level runs identically), but stands
/// ready to catch fleet-wide any future Core pass that mis-optimizes at `O2`/`O3`.
fn gate_opt_sweep(paths: &Paths, profile: &str, opts: &GateOpts) {
    // The sweep drives the WASM run path only (`run_program_wasm`). Honoring `--target rust`/`rust-async`
    // would need `--opt-level` threaded through the separate rust run path (a follow-up); silently running
    // wasm under `--target rust` would give a FALSE sense of cross-level coverage for that backend. So
    // REJECT the combo with a clear error rather than fake it.
    if !matches!(opts.target, GateTarget::Wasm) {
        eprintln!(
            "error: --opt-sweep supports only the wasm target for now (got --target {}). \
             Run `gate --opt-sweep` without --target; rust/rust-async opt-level sweep is a follow-up.",
            match opts.target {
                GateTarget::Rust => "rust",
                GateTarget::RustAsync => "rust-async",
                GateTarget::Wasm => "wasm",
                GateTarget::CadenzaMl => "cadenza-ml",
            }
        );
        std::process::exit(2);
    }
    let tools = build_tools(paths, profile);
    let files = if opts.files.is_empty() {
        default_corpus_files(paths)
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
        .flat_map(|file| read_corpus(&tools, file))
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
                let (cursor, checked, skipped, divergences, records, tools, store) = (
                    &cursor,
                    &checked,
                    &skipped,
                    &divergences,
                    &records,
                    &tools,
                    &opts.store,
                );
                scope.spawn(move || {
                    loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= records.len() {
                            break;
                        }
                        match sweep_one_case(tools, store, &records[i], &LEVELS) {
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
        "\ngate --opt-sweep: {checked} checked ({skipped} skipped: package/decline), {} divergence(s) across O0..O3",
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
) -> SweepOutcome {
    // Multi-file package cases are out of scope for this first cut.
    if !rec.modules.is_empty() {
        return SweepOutcome::Skipped;
    }
    // Each trial (a `(call …)`, or the single no-call trial) is run at every level and compared.
    let calls: Vec<Option<&Call>> = if rec.trials.is_empty() {
        vec![None]
    } else {
        rec.trials.iter().map(|t| t.call.as_ref()).collect()
    };
    let mut diffs = Vec::new();
    let mut checked_any = false;
    for call in calls {
        let runs: Vec<Ran> = levels
            .iter()
            .map(|lvl| {
                run_program_wasm(
                    tools,
                    store,
                    &rec.program,
                    &rec.modules,
                    call,
                    &rec.host_responses,
                    Some(lvl),
                )
            })
            .collect();
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
    // the pipeline stages are separate processes the OS already schedules across cores.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

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
        for rec in read_corpus(tools, file) {
            if !rec.description.contains(needle) {
                continue;
            }
            found += 1;
            // Run each trial (re-driving the program per its `(call …)`) so the debug view shows every
            // call/expect/actual line — a multi-trial case lists them in order.
            let rans: Vec<Ran> = rec
                .trials
                .iter()
                .map(|t| {
                    run_program(
                        tools,
                        store,
                        &rec.program,
                        &rec.modules,
                        t.call.as_ref(),
                        &rec.host_responses,
                        target,
                    )
                })
                .collect();
            let verdict = match grade_ran(&rec, &rans) {
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
                    Ran::Value(v, calls) if calls.is_empty() => format!("value {v}"),
                    Ran::Value(v, calls) => format!("value {v} [host-calls: {}]", calls.join(", ")),
                    Ran::Declined { code: Some(c) } => format!("rejected [{c}]"),
                    Ran::Declined { code: None } => {
                        "declined (compiler can't compile it yet)".to_string()
                    }
                    Ran::Trap(t) => format!("trap: {t}"),
                    Ran::BadArtifact(e) => format!("artifact did not build: {e}"),
                };
                println!("expect:   {}", trial.expect);
                println!("actual:   {actual}");
            }
            println!("verdict:  {verdict}\n");
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

/// A parsed corpus record (the flat stream `cdz-syntax corpus` emits).
struct CorpusRecord {
    description: String,
    program: String,
    /// Sibling LIBRARY modules of a multi-file PACKAGE case (`DESIGN-package-linking.md`), each a
    /// `(name, program)` from a `module` record line. Empty for a single-file case (then `program` is
    /// compiled alone). When non-empty, the wasm gate driver writes every module + the entry (`program`,
    /// named `main`) to a temp dir and runs `cdz compile <files> --entry main` instead of the stdin pipe.
    modules: Vec<(String, String)>,
    /// One or more TRIALS — each an optional `(call …)` paired with the `expect` payload it must
    /// produce. The program is compiled ONCE; each trial runs its call and grades against its expect,
    /// and the case's verdict COMBINES them (see `grade_ran`). A single-result case is one trial with
    /// `call: None`; a case interleaving several `(call …) (output …)` pairs has one trial each.
    trials: Vec<Trial>,
    /// The `(needs …)` capabilities a case documents. NO LONGER gates grading — every case is graded by
    /// what the compiler ACTUALLY does (a construct it can't compile DECLINES → `Todo`, not skipped), so
    /// `(needs)` is documentation only now, kept so a corpus `(needs …)` clause still parses. (Was a
    /// blunt whole-feature skip that hid a case running to a WRONG value as a benign skip.)
    #[allow(dead_code)]
    needs: Vec<String>,
    /// The HOST-CALL RESPONSES (E2h) — `(op, value)` pairs from the record stream's `host-response`
    /// lines, in call order. A host-delegating case's program consumes these; the wasm gate driver
    /// forwards each to `cdz-run --host-response`. Empty for a non-host case.
    host_responses: Vec<(String, String)>,
    /// The recorded HOST-CALL sequence (E2h) — the dotted `E.op` names from the record stream's
    /// `host-call` lines, in call order. The gate verifies the run's observed host calls against this
    /// (`grade_ran`); empty for a case with no `(host-calls …)`.
    host_calls: Vec<String>,
}

/// One (call, expected-payload) trial of a case — a single run of the compiled program.
struct Trial {
    /// The `(call …)` for this trial, or `None` to invoke the sole export with no arguments.
    call: Option<Call>,
    /// The `expect` payload, e.g. `output (: 42 Int64)`, `error CDZ0201`, `trap "…"`.
    expect: String,
}

/// A corpus case's `(call <export> <arg>…)` clause, parsed from the record stream. The export is run
/// with these arguments (already reduced to bare value text by `cdz-syntax corpus`), which cdz-run
/// coerces to the export's declared parameter types — the path that exercises a parameterized entry.
struct Call {
    export: String,
    args: Vec<String>,
}

/// Run `cdz-corpus records <file>` and parse its record stream.
fn read_corpus(tools: &Tools, file: &Path) -> Vec<CorpusRecord> {
    use std::process::Command;
    let out = Command::new(&tools.corpus)
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

/// Parse the flat record stream: `key\tvalue` lines, records separated by a `---` line. Each TRIAL is a
/// `call` line (the export) + its following `arg` lines + the `expect` line that CLOSES it — so an
/// `expect` flushes the pending call/args into a trial. A single-trial case is the historical shape.
fn parse_records(text: &str) -> Vec<CorpusRecord> {
    let mut records = Vec::new();
    let (mut desc, mut prog, mut needs) = (String::new(), String::new(), Vec::new());
    let mut modules: Vec<(String, String)> = Vec::new();
    let mut trials: Vec<Trial> = Vec::new();
    let mut host_responses: Vec<(String, String)> = Vec::new();
    let mut host_calls: Vec<String> = Vec::new();
    let (mut call_export, mut call_args): (Option<String>, Vec<String>) = (None, Vec::new());
    for line in text.lines() {
        if line == "---" {
            records.push(CorpusRecord {
                description: std::mem::take(&mut desc),
                program: std::mem::take(&mut prog),
                modules: std::mem::take(&mut modules),
                trials: std::mem::take(&mut trials),
                needs: std::mem::take(&mut needs),
                host_responses: std::mem::take(&mut host_responses),
                host_calls: std::mem::take(&mut host_calls),
            });
            // Defensive: a well-formed record ends every trial with an `expect`, so nothing is pending.
            call_export = None;
            call_args.clear();
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
                "call" => call_export = Some(val.to_string()),
                "arg" => call_args.push(val.to_string()),
                "expect" => {
                    // The `expect` closes a trial: pair the pending call (if any) with this payload.
                    let call = call_export.take().map(|export| Call {
                        export,
                        args: std::mem::take(&mut call_args),
                    });
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
                _ => {}
            }
        }
    }
    records
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
        .map(|t| {
            run_program(
                tools,
                store,
                &rec.program,
                &rec.modules,
                t.call.as_ref(),
                &rec.host_responses,
                target,
            )
        })
        .collect();
    grade_ran(rec, &rans)
}

/// Combine per-trial outcomes into the case's verdict. `rans[i]` is the outcome of `rec.trials[i]`.
/// Shared by the tally path and the single-case debug view.
fn grade_ran(rec: &CorpusRecord, rans: &[Ran]) -> Grade {
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
        && let Some(Ran::Value(_, observed)) = rans.iter().find(|r| matches!(r, Ran::Value(..)))
        && *observed != rec.host_calls
    {
        return Grade::Fail(format!(
            "host-call mismatch: expected [{}], observed [{}]",
            rec.host_calls.join(", "),
            observed.join(", ")
        ));
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
            let expected_val = expected_value(payload);
            let expected_full = payload.trim().to_string();
            match ran {
                Ran::Value(v, _) if *v == expected_val || *v == expected_full => Grade::Pass,
                Ran::Value(v, _) => Grade::Fail(format!("expected {expected_full}, ran → {v}")),
                Ran::Declined { .. } => Grade::Todo, // compiler can't compile it yet
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
            let want = payload.trim();
            match ran {
                Ran::Value(v, _) => Grade::Fail(format!("expected rejection {want}, ran → {v}")),
                Ran::Declined { code: Some(got) } if got == want => Grade::Pass,
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
        // Trap-reason matching normalizes both sides to a canonical trap KIND (`trap_kind`), so the
        // corpus's `divide by zero` matches wasmtime's `integer divide by zero`, `overflow` matches
        // `integer overflow`, `index out of bounds` matches `out of bounds memory access`, etc.
        "trap" => match ran {
            Ran::Value(v, _) => Grade::Fail(format!("expected a trap, ran → {v}")),
            // A broken artifact for a case that should TRAP is still a miscompile (the backend was asked
            // for a runnable artifact that traps and emitted un-compilable source instead).
            Ran::BadArtifact(e) => {
                Grade::Fail(format!("expected a trap, artifact did not build: {e}"))
            }
            Ran::Trap(actual) => match (trap_kind(payload), trap_kind(actual)) {
                // Both classify AND agree → the expected trap fired.
                (Some(want), Some(got)) if want == got => Grade::Pass,
                // Trapped, but the reason doesn't classify or doesn't match — a real trap, unconfirmed.
                _ => Grade::Todo,
            },
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
        "declines" => match ran {
            Ran::Declined { .. } => Grade::Pass,
            Ran::Value(v, _) => Grade::Fail(format!("expected a decline, ran → {v}")),
            Ran::Trap(t) => Grade::Fail(format!("expected a decline, trapped: {t}")),
            Ran::BadArtifact(e) => {
                Grade::Fail(format!("expected a decline, artifact did not build: {e}"))
            }
        },
        _ => Grade::Todo,
    }
}

/// Classify a trap-reason string (from EITHER the corpus's `(trap "<reason>")` or cdz-run's actual
/// wasmtime trap message) into a canonical trap KIND, so the two vocabularies compare equal. Returns
/// `None` for a reason that doesn't classify (so the grader stays conservative — an unclassifiable
/// actual never Passes against a classifiable expectation, and vice versa).
///
/// The corpus writes human reasons (`divide by zero`, `integer overflow`, `index out of bounds`,
/// `unreachable`, `out of range`); wasmtime writes its own (`integer divide by zero`, `integer
/// overflow`, `out of bounds memory access`, `wasm 'unreachable' instruction executed`). Both are
/// lowercased and matched by the distinguishing SUBSTRING, mapping to one token per underlying trap.
fn trap_kind(reason: &str) -> Option<&'static str> {
    let r = reason.to_ascii_lowercase();
    // Order matters: check the most specific substrings first. "divide by zero" / "division by zero"
    // both contain "zero"; "integer overflow" / "overflow" share "overflow".
    if r.contains("divide by zero") || r.contains("division by zero") {
        Some("div-by-zero")
    } else if r.contains("out of bounds") || r.contains("out-of-bounds") {
        // wasmtime "out of bounds memory access" (a guest bounds trap) and the corpus "index out of
        // bounds" are the same underlying fault — a list/segment index past the end.
        Some("out-of-bounds")
    } else if r.contains("overflow") {
        // "integer overflow" / bare "overflow" — an arithmetic result outside the type width.
        Some("overflow")
    } else if r.contains("unreachable") || r.contains("shift count out of range") {
        // wasmtime "wasm 'unreachable' instruction executed" / the corpus bare "unreachable" — the
        // compiler lowers an explicit non-arithmetic trap (a `trap`/uninhabited-match) to `unreachable`.
        // The rust backend's shift-count guard panics "shift count out of range" for the SAME
        // non-arithmetic `Core::Trap` the wasm backend lowers to bare `unreachable` (an out-of-range
        // shift count) — map it to the same canonical kind so a `(trap "unreachable")` shift-count case
        // grades pass on BOTH backends. (Rust's second shift panic, "integer overflow in left shift",
        // already classifies via the "overflow" arm above.)
        Some("unreachable")
    } else {
        None
    }
}

/// The value out of an `output` payload `(: <value> <Type>)` — the text of `<value>`. Falls back to
/// the whole payload if it is not the `(: value Type)` shape.
fn expected_value(payload: &str) -> String {
    // payload is `(: <value> <Type>)`. Take the FIRST whitespace-separated token after `(:` as the
    // value, respecting nesting — a COMPOUND value/type is itself parenthesized (`(: (tuple 0 7) (Tuple
    // Int64 Int64))`), so a naive "everything up to the last space" split cuts the value wrong. Scan the
    // first balanced token: a `(…)` group, or a bare atom up to the next top-level space.
    let inner = payload.trim();
    let Some(rest) = inner.strip_prefix("(:") else {
        return inner.to_string();
    };
    let rest = rest.trim();
    let bytes = rest.as_bytes();
    if bytes.first() == Some(&b'(') {
        // A parenthesized value — take the balanced `(…)` group.
        let mut depth = 0i32;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return rest[..=i].to_string();
                    }
                }
                _ => {}
            }
        }
        rest.to_string()
    } else if bytes.first() == Some(&b'"') {
        // A QUOTED STRING value (`(: "parse error" String)`) — take up to and INCLUDING the closing `"`.
        // A String value contains INTERNAL SPACES, so the bare-atom "up to next space" split would cut it
        // wrong (`"parse` — breaking every multi-word string result). Scan for the matching close quote,
        // honoring a `\"` escape so an embedded quote does not end the token early.
        let mut escaped = false;
        for (i, &b) in bytes.iter().enumerate().skip(1) {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                return rest[..=i].to_string();
            }
        }
        rest.to_string()
    } else {
        // A bare atom — up to the next space.
        match rest.find(char::is_whitespace) {
            Some(idx) => rest[..idx].to_string(),
            None => rest.trim_end_matches(')').to_string(),
        }
    }
}

/// The default corpus: every `spec/semantics/NN-*.{sexp,md}`, sorted for stable order. Corpus files
/// follow the `NN-feature` naming convention (a numeric prefix), which distinguishes a migrated
/// corpus `.md` from an ordinary docs `.md` like `README.md` — only digit-led stems are corpus files.
/// A stem may exist as `.sexp` (source) and/or `.md` (migrated); during the migration both may
/// coexist, so a `.sexp` whose stem also has a `.md` is dropped — the `.md` wins. That way a file
/// cuts over to markdown the moment it is migrated, and its `.sexp` can be deleted afterward with
/// zero change to the gate.
fn default_corpus_files(paths: &Paths) -> Vec<PathBuf> {
    let dir = paths.repo.join("spec/semantics");
    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("xtask gate: reading {}: {e}", dir.display());
            std::process::exit(1);
        })
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sexp" || x == "md"))
        // Only the `NN-feature` corpus files, never an ordinary docs `.md` (e.g. README.md).
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()))
        })
        .collect();
    let migrated: std::collections::HashSet<_> = entries
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| p.file_stem().map(|s| s.to_os_string()))
        .collect();
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .filter(|p| {
            // Keep every `.md`; keep a `.sexp` only if its stem has no migrated `.md`.
            !(p.extension().is_some_and(|x| x == "sexp")
                && p.file_stem().is_some_and(|s| migrated.contains(s)))
        })
        .collect();
    files.sort();
    files
}

// ============================================================================================
// gate baseline — a committed per-case verdict snapshot, so a REGRESSION (a case that used to pass
// and now doesn't) fails `gate --check` even while the pass/todo/fail totals drift.
// ============================================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Todo,
    Fail,
}

impl Verdict {
    fn tag(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Todo => "todo",
            Verdict::Fail => "fail",
        }
    }
    fn parse(s: &str) -> Option<Verdict> {
        match s {
            "pass" => Some(Verdict::Pass),
            "todo" => Some(Verdict::Todo),
            "fail" => Some(Verdict::Fail),
            _ => None,
        }
    }
}

/// The committed baseline file for a target: `<repo>/spec/semantics/.gate-baseline` for the default
/// wasm gate, and a target-suffixed sibling (`.gate-baseline-rust`) for another backend — so each
/// backend has its OWN regression baseline and one does not clobber the other's.
fn baseline_path(paths: &Paths, target: GateTarget) -> PathBuf {
    let name = match target {
        GateTarget::Wasm => ".gate-baseline".to_string(),
        GateTarget::Rust => ".gate-baseline-rust".to_string(),
        GateTarget::RustAsync => ".gate-baseline-rust-async".to_string(),
        // CadenzaMl is graded DIFFERENTIALLY against the Wasm oracle (the `ml-conformance` reported
        // step), never against a saved baseline — it has no baseline file and this is never reached.
        GateTarget::CadenzaMl => {
            unreachable!("cadenza-ml is differential-only; it has no gate baseline")
        }
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
    let mut by_desc: std::collections::BTreeMap<&str, Verdict> = std::collections::BTreeMap::new();
    for (d, v) in verdicts {
        by_desc.insert(d.as_str(), *v);
    }
    let mut lines: Vec<String> = by_desc
        .iter()
        .map(|(d, v)| format!("{}\t{d}", v.tag()))
        .collect();
    lines.sort();
    let body = format!(
        "# gate baseline — per-case verdicts (verdict\\tdescription). Regenerate with `cargo xtask gate --save`.\n{}\n",
        lines.join("\n")
    );
    std::fs::write(baseline_path(paths, target), body).expect("write baseline");
}

/// Compare current verdicts to the baseline. Returns the process exit code: non-zero if any case
/// REGRESSED (baseline pass → now not pass) or a baseline case vanished. Newly-passing cases and
/// new cases are reported but do not fail the check.
fn check_baseline(paths: &Paths, verdicts: &[(String, Verdict)], target: GateTarget) -> i32 {
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
    // Parse into a description→verdict map, but DETECT duplicate descriptions as we go. The baseline
    // is keyed by description (a map), so two lines with the same description silently collapse —
    // last-parsed wins — which can MASK a real verdict (e.g. a `todo` line hiding a `pass` line for the
    // same case, or vice-versa). Duplicates are easy to introduce now that these files are `merge=union`
    // (both sides of a merge append their copy). Fail loudly on any duplicate rather than let it hide a
    // verdict silently. (`gate --save` re-sorts + de-dupes, so the fix is to regenerate the baseline.)
    // Classify duplicate descriptions. `merge=union` on this file re-injects a duplicate LINE whenever
    // two branches append near the same region, so a BENIGN same-verdict dup (both copies agree) is a
    // routine merge artifact — NOT a reason to hard-fail every agent's `gate --check`. Only a
    // CONFLICTING dup (same description, DIFFERENT verdicts) is dangerous: the map-load's last-wins
    // would silently mask a verdict. So: auto-dedup benign dups (rewrite the file clean + continue),
    // and HARD-FAIL only on a conflicting dup.
    let mut base: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    let mut seen: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    let mut benign_dups = 0usize;
    let mut conflicting: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            base.insert(d.to_string(), verdict);
            match seen.insert(d.to_string(), verdict) {
                None => {}
                Some(prev) if prev == verdict => benign_dups += 1,
                Some(_) => conflicting.push(d.to_string()),
            }
        }
    }
    if !conflicting.is_empty() {
        conflicting.sort();
        conflicting.dedup();
        eprintln!(
            "xtask gate --check: {} CONFLICTING duplicate case description(s) in {} — the same case \
             appears with DIFFERENT verdicts, so the map-keyed baseline silently masks one (last wins). \
             This is a real integrity error; regenerate with `cargo xtask gate --save` and check which \
             verdict is correct. Conflicting:",
            conflicting.len(),
            path.display()
        );
        for d in &conflicting {
            eprintln!("  •  {d}");
        }
        return 3;
    }
    if benign_dups > 0 {
        // Benign same-verdict dups (a union-merge artifact) — auto-dedup the file in place and carry on,
        // rather than hard-failing the fleet's gate over a harmless duplicate line. `save_baseline`'s
        // canonical (sorted, unique) writer does the dedup; the verdict map is unchanged.
        eprintln!(
            "xtask gate --check: auto-deduped {benign_dups} benign (same-verdict) duplicate line(s) in \
             {} — a merge=union artifact, harmless; rewrote the baseline clean.",
            path.display()
        );
        let verdicts_now: Vec<(String, Verdict)> =
            base.iter().map(|(d, v)| (d.clone(), *v)).collect();
        save_baseline(paths, &verdicts_now, target);
    }

    let now: std::collections::HashMap<&str, Verdict> =
        verdicts.iter().map(|(d, v)| (d.as_str(), *v)).collect();

    let mut regressed: Vec<String> = Vec::new();
    let mut gained: Vec<String> = Vec::new();
    let mut vanished: Vec<String> = Vec::new();

    for (desc, &was) in &base {
        match now.get(desc.as_str()) {
            None => vanished.push(desc.clone()),
            Some(&is) => {
                if was == Verdict::Pass && is != Verdict::Pass {
                    regressed.push(format!("{desc} ({} → {})", was.tag(), is.tag()));
                } else if was != Verdict::Pass && is == Verdict::Pass {
                    gained.push(desc.clone());
                }
            }
        }
    }

    if !gained.is_empty() {
        println!("\nnewly passing ({}):", gained.len());
        for g in &gained {
            println!("  +  {g}");
        }
    }
    if !regressed.is_empty() {
        println!("\nREGRESSED ({}):", regressed.len());
        for r in &regressed {
            println!("  -  {r}");
        }
    }
    if !vanished.is_empty() {
        println!("\nvanished from the corpus ({}):", vanished.len());
        for v in &vanished {
            println!("  ?  {v}");
        }
    }

    if regressed.is_empty() && vanished.is_empty() {
        println!(
            "\ngate --check: OK (no regressions vs baseline; {} newly passing)",
            gained.len()
        );
        0
    } else {
        println!(
            "\ngate --check: FAIL ({} regressed, {} vanished)",
            regressed.len(),
            vanished.len()
        );
        1
    }
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
fn check(paths: &Paths, profile: &str) {
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
    log.step("test", "cargo test --workspace", repo);
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
    // (an unsound pass caught before it lands — the §9 both-backend-miscompile guard). Wasm-target only
    // (the sweep rejects a non-wasm target); ~parallelized so the full corpus completes in a couple min.
    log.step(
        "opt-sweep",
        &format!("{xtask} --profile {profile} gate --opt-sweep"),
        repo,
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
        "implementation/agent-harness",
    ] {
        log.step(
            &format!("cdz-test {suite}"),
            &format!("{cdz} test {suite}"),
            repo,
        );
    }

    // Citation-coverage regression gate: fail if a `//=` / `//#` duvet citation was deleted/stranded
    // (live cited < the committed floor). Skips only when `duvet` isn't installed; a present-but-
    // erroring duvet (a stranded citation) FAILS loudly. Thread `--profile` through like the gate step
    // so `cargo xtask --profile <p> check` runs every nested self-invocation under the SAME profile
    // (no cross-profile rebuild of this xtask binary between steps).
    log.step_show(
        "duvet-check",
        &format!("{xtask} --profile {profile} duvet-check"),
        repo,
    );

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

    // cadenza-ml conformance — REPORTED, never a baseline gate. Drive each shared-corpus program
    // through BOTH the ML compiler (`cdz run-ml`) and the rcdzc/wasm ORACLE, and diff the verdicts:
    // an ML decline is coverage-not-yet (the front-end is subset-only), an ML value that MATCHES the
    // oracle is progress, and an ML value that DISAGREES with the oracle is a differential miscompile.
    // Report-only for now (a D>0 loud-warns but does not red the fleet, since the ML front-end is under
    // active development); promote to blocking once it's trusted. Green against today's declines-all
    // stub (0 agree / 0 disagree). This is the differential form the operator directed (C + diff).
    report_ml_conformance(paths, profile);

    println!("\ncheck: all green ✓  (full log: {})", log.path.display());
}

/// Report cadenza-ml conformance DIFFERENTIALLY against the rcdzc/wasm oracle (never fails `check`).
/// For each corpus case: run it through the ML compiler (`cdz run-ml`) and through the Wasm oracle, and
/// classify — ML declined → coverage-not-yet; ML value == oracle value → agree; ML value != oracle, or
/// ML ran where the oracle declined / vice-versa → DISAGREE (a differential miscompile). Prints
/// `cadenza-ml: X agree / D disagree / N total`; a `D>0` prints a loud warning naming the disagreeing
/// cases. Report-only: it warns but never reds the gate (the ML front-end is subset-only + evolving).
fn report_ml_conformance(paths: &Paths, profile: &str) {
    let tools = build_tools(paths, profile);
    // The oracle (Wasm) resolves the value-heap runtime from the content-addressed store; the ML path
    // needs no store. Default store location, same as the gate.
    let store = Some(paths.repo.join("target/cadenza-store"));
    let files = default_corpus_files(paths);
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools, file))
        .collect();

    let (mut agree, mut disagree, mut not_yet) = (0usize, 0usize, 0usize);
    let mut disagreements: Vec<String> = Vec::new();
    for rec in &records {
        // Compare the FIRST trial only (the ML subset is nullary; a `(call …)` case declines in
        // run_program_ml's subset guard, so the first trial is representative of ML support today).
        let call = rec.trials.first().and_then(|t| t.call.as_ref());
        let ml = run_program(
            &tools,
            &store,
            &rec.program,
            &rec.modules,
            call,
            &rec.host_responses,
            GateTarget::CadenzaMl,
        );
        match &ml {
            // Not-yet-supported: the ML front-end declined. Coverage-not-yet, never a disagreement.
            Ran::Declined { .. } => not_yet += 1,
            // The ML compiler produced a value (or an error/bad artifact) — compare to the oracle.
            _ => {
                let oracle = run_program(
                    &tools,
                    &store,
                    &rec.program,
                    &rec.modules,
                    call,
                    &rec.host_responses,
                    GateTarget::Wasm,
                );
                if ml_agrees_with_oracle(&ml, &oracle) {
                    agree += 1;
                } else {
                    disagree += 1;
                    disagreements.push(format!(
                        "{}: ml={} oracle={}",
                        rec.description,
                        ran_summary(&ml),
                        ran_summary(&oracle)
                    ));
                }
            }
        }
    }
    let total = agree + disagree + not_yet;
    println!(
        "cadenza-ml: {agree} agree / {disagree} disagree / {total} total ({not_yet} coverage-not-yet)"
    );
    if disagree > 0 {
        eprintln!(
            "  ⚠ cadenza-ml DIFFERENTIAL DISAGREEMENT(S) — the ML compiler's verdict differs from the \
             rcdzc oracle (a differential miscompile to investigate; report-only, not yet blocking):"
        );
        for d in &disagreements {
            eprintln!("    • {d}");
        }
    }
}

/// Whether the ML compiler's outcome AGREES with the rcdzc oracle on the same program. A shared VALUE
/// must match; a shared trap agrees (both aborted); a coded reject agrees if the oracle also rejected.
/// A value-vs-non-value, or ML-ran-where-oracle-declined (and vice-versa), is a DISAGREEMENT. (An ML
/// decline is filtered out by the caller before this — it's coverage-not-yet, never compared.)
fn ml_agrees_with_oracle(ml: &Ran, oracle: &Ran) -> bool {
    match (ml, oracle) {
        (Ran::Value(a, _), Ran::Value(b, _)) => a == b,
        (Ran::Trap(_), Ran::Trap(_)) => true,
        // The oracle declined but ML produced a value/trap — a disagreement (ML ran where rcdzc didn't).
        _ => false,
    }
}

/// A one-line summary of a `Ran` for the disagreement report.
fn ran_summary(r: &Ran) -> String {
    match r {
        Ran::Value(v, _) => format!("value {v}"),
        Ran::Declined { code: Some(c) } => format!("reject {c}"),
        Ran::Declined { code: None } => "declined".to_string(),
        Ran::Trap(m) => format!("trap {m}"),
        Ran::BadArtifact(m) => format!("bad-artifact {m}"),
    }
}

/// The set of case DESCRIPTIONS in a gate-baseline file (the `verdict\tdescription` lines, skipping
/// the `#` header/blank lines). Pure, so the agreement lint is unit-tested without the filesystem.
fn baseline_titles(text: &str) -> std::collections::BTreeSet<String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_v, d)| d.to_string()))
        .collect()
}

/// Assert the three gate baselines (wasm / rust / rust-async) cover the SAME set of case descriptions.
/// A title present in one baseline but absent from another is a stale-baseline error (typically a
/// wasm-only `gate --save` that left the rust baselines behind). Returns an actionable error naming the
/// diverging titles + the heal command; `Ok(())` when all three agree (or a baseline is absent —
/// nothing to compare, and `gate --check` already errors on a missing baseline).
fn baseline_titles_agree_lint(paths: &Paths) -> Result<(), String> {
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

// ============================================================================================
// roundtrip — the syntax surfaces round-trip on every corpus program.
// ============================================================================================

/// For every corpus program, confirm the syntax surfaces round-trip. `sexpr` is STRICT: re-encoding
/// the round-tripped text to binary yields the SAME bytes as the original. `ml` is IDEMPOTENT: it may
/// canonicalize on the first round-trip (e.g. name-alias compound ctors → their literal/string-
/// primitive form), so it must reach a FIXED POINT (`ml(ml(x)) == ml(x)`) rather than reproduce the
/// original byte-for-byte. Guards `cadenza-syntax` (reader/printer/codec) independently of the compiler.
fn roundtrip(paths: &Paths, profile: &str, files: Vec<PathBuf>) {
    let tools = build_tools(paths, profile);
    let files = if files.is_empty() {
        default_corpus_files(paths)
    } else {
        files
    };

    // Gather every case, then round-trip them in PARALLEL. Each case is independent — it only READS
    // `tools` and spawns its own `cdz-syntax` conversions — so the ~1025 cases × 2 surfaces (each a
    // subprocess) are embarrassingly parallel; the serial loop was process-wait-bound at ~10s. Failure
    // messages are collected per case (in the SAME order as `records`) so the reported list is stable.
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools, file))
        .collect();
    let per_case = roundtrip_all_parallel(&tools, records);

    let (mut ok, mut fail) = (0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();
    for case in per_case {
        if case.counted_ok {
            ok += 1;
        }
        fail += case.failures.len() as u32;
        failures.extend(case.failures);
    }

    println!("\nroundtrip: {ok} programs ok, {fail} failures");
    if !failures.is_empty() {
        println!();
        for f in failures.iter().take(40) {
            println!("  FAIL  {f}");
        }
        if failures.len() > 40 {
            println!("  … and {} more", failures.len() - 40);
        }
        std::process::exit(1);
    }
}

/// One case's round-trip outcome: whether it counted as an `ok` program, and any failure messages.
/// `counted_ok` mirrors the serial loop's `ok += 1` — reached iff the reference sexpr→binary
/// succeeded (a program whose reference conversion fails is NOT counted ok).
struct RoundtripCase {
    counted_ok: bool,
    failures: Vec<String>,
}

/// Round-trip every record in PARALLEL, returning one [`RoundtripCase`] per record in the SAME order
/// as `records`. Same shape as `grade_all_parallel`: a `std::thread::scope` worker pool (no new
/// dependency) pulling from a shared atomic cursor, each result written to its own index slot so the
/// reported failure list is order-stable. Each case only READS `tools` and spawns its own `cdz-syntax`
/// conversions, so there is no shared mutable state.
fn roundtrip_all_parallel(tools: &Tools, records: Vec<CorpusRecord>) -> Vec<RoundtripCase> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let n = records.len();
    let slots: Vec<Mutex<Option<RoundtripCase>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let cursor = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

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
                    let mut failures = Vec::new();
                    // The reference: the program's canonical binary AST. If it fails, the case is not
                    // counted ok (mirrors the serial loop's early `continue`).
                    let counted_ok = match to_binary(tools, &rec.program) {
                        None => {
                            failures.push(format!("{}: sexpr→binary failed", rec.description));
                            false
                        }
                        Some(bin0) => {
                            // Round-trip through each intermediate surface, back to binary, compare bytes.
                            //
                            // `sexpr` is STRICT: it must reproduce the exact binary (byte-identical).
                            //
                            // `ml` is IDEMPOTENT rather than strict: the ML surface is the long-term
                            // syntax and is allowed to CANONICALIZE on the first round-trip (e.g. the
                            // name-alias compound ctors `(tuple a b)`/`(list …)` render to the literal
                            // `(a, b)`/`[…]`, which parses back to the string-primitive `("tuple" …)` —
                            // a deliberate, semantics-preserving normalization, not information loss).
                            // So we require the ML round-trip to reach a FIXED POINT: a second ML
                            // round-trip must reproduce the first (`ml(ml(x)) == ml(x)`). That still
                            // catches any real ML round-trip bug (non-idempotence = instability or lost
                            // information) while tolerating the one-time ctor canonicalization.
                            match roundtrip_via(tools, &bin0, "sexpr") {
                                Some(bin1) if bin1 == bin0 => {}
                                Some(_) => failures
                                    .push(format!("{}: binary≠binary via sexpr", rec.description)),
                                None => failures.push(format!(
                                    "{}: round-trip via sexpr errored",
                                    rec.description
                                )),
                            }
                            match roundtrip_via(tools, &bin0, "ml") {
                                Some(bin1) => match roundtrip_via(tools, &bin1, "ml") {
                                    Some(bin2) if bin2 == bin1 => {}
                                    Some(_) => failures.push(format!(
                                        "{}: ml round-trip not idempotent (ml(ml(x)) != ml(x))",
                                        rec.description
                                    )),
                                    None => failures.push(format!(
                                        "{}: second ml round-trip errored",
                                        rec.description
                                    )),
                                },
                                None => failures.push(format!(
                                    "{}: round-trip via ml errored",
                                    rec.description
                                )),
                            }
                            true
                        }
                    };
                    *slots[i].lock().unwrap() = Some(RoundtripCase {
                        counted_ok,
                        failures,
                    });
                }
            });
        }
    });

    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap()
                .expect("every case round-tripped")
        })
        .collect()
}

/// A program's sexpr text → its canonical binary AST bytes (via `cdz-syntax`).
fn to_binary(tools: &Tools, program: &str) -> Option<Vec<u8>> {
    convert_bytes(tools, program.as_bytes(), "sexpr", "binary")
}

/// binary → <surface> text → binary, returning the re-encoded bytes (to compare to the original).
fn roundtrip_via(tools: &Tools, bin0: &[u8], surface: &str) -> Option<Vec<u8>> {
    let text = convert_bytes(tools, bin0, "binary", surface)?;
    convert_bytes(tools, &text, surface, "binary")
}

/// Run `cdz-syntax --from <from> --to <to>` over `input` bytes (stdin) and return its stdout.
fn convert_bytes(tools: &Tools, input: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(&tools.syntax)
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

// ============================================================================================
// fmt — format program files through the printer.
// ============================================================================================

/// Format each file through the printer (round-trip its own surface to canonical form). `--check`
/// writes nothing and exits non-zero if any file is not already canonical.
fn fmt(paths: &Paths, profile: &str, files: Vec<PathBuf>, to: &str, check: bool) {
    if files.is_empty() {
        eprintln!("xtask fmt: name at least one file");
        std::process::exit(1);
    }
    let tools = build_tools(paths, profile);
    let mut unformatted: Vec<String> = Vec::new();

    for file in &files {
        let original = match std::fs::read(file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("xtask fmt: {}: {e}", file.display());
                std::process::exit(1);
            }
        };
        // Format = parse the surface and re-print it canonically (same surface in and out).
        let formatted = match convert_bytes(&tools, &original, to, to) {
            Some(b) => b,
            None => {
                eprintln!("xtask fmt: {}: does not parse as {to}", file.display());
                std::process::exit(1);
            }
        };
        // The printer emits no trailing newline; keep files newline-terminated.
        let mut formatted = formatted;
        if !formatted.ends_with(b"\n") {
            formatted.push(b'\n');
        }
        if formatted == original {
            continue;
        }
        if check {
            unformatted.push(file.display().to_string());
        } else if let Err(e) = std::fs::write(file, &formatted) {
            eprintln!("xtask fmt: writing {}: {e}", file.display());
            std::process::exit(1);
        } else {
            println!("formatted {}", file.display());
        }
    }

    if check && !unformatted.is_empty() {
        println!("not formatted ({}):", unformatted.len());
        for f in &unformatted {
            println!("  {f}");
        }
        std::process::exit(1);
    }
}

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

/// SHA-256 of the bytes, lowercase hex (the recorded hashing choice).
pub(crate) fn content_address(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
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
    fn ml_agrees_with_oracle_only_on_matching_value_or_shared_trap() {
        // Same value → agree; different value → disagree (the differential miscompile).
        assert!(ml_agrees_with_oracle(
            &Ran::Value("42".into(), vec![]),
            &Ran::Value("42".into(), vec![])
        ));
        assert!(!ml_agrees_with_oracle(
            &Ran::Value("42".into(), vec![]),
            &Ran::Value("43".into(), vec![])
        ));
        // Both trapped → agree (both aborted).
        assert!(ml_agrees_with_oracle(
            &Ran::Trap("overflow".into()),
            &Ran::Trap("divide by zero".into())
        ));
        // ML ran to a value where the oracle DECLINED (or trapped) → disagree (ML ran where rcdzc
        // didn't — a real differential, the case the diff exists to catch).
        assert!(!ml_agrees_with_oracle(
            &Ran::Value("1".into(), vec![]),
            &Ran::Declined { code: None }
        ));
        assert!(!ml_agrees_with_oracle(
            &Ran::Value("1".into(), vec![]),
            &Ran::Trap("overflow".into())
        ));
    }

    #[test]
    fn trap_kind_maps_corpus_and_wasmtime_vocabularies_to_one_token() {
        // The corpus's human reasons and wasmtime's actual trap messages must classify to the SAME token
        // per underlying trap, so `grade_trial`'s `trap` arm recognizes an expected trap that fired.
        // Division by zero — corpus writes both spellings; wasmtime prepends "integer".
        assert_eq!(trap_kind("divide by zero"), Some("div-by-zero"));
        assert_eq!(trap_kind("division by zero"), Some("div-by-zero"));
        assert_eq!(
            trap_kind("cdz-run: trap: wasm trap: integer divide by zero: error while executing"),
            Some("div-by-zero")
        );
        // Overflow — bare and "integer" both, corpus + wasmtime.
        assert_eq!(trap_kind("overflow"), Some("overflow"));
        assert_eq!(trap_kind("integer overflow"), Some("overflow"));
        assert_eq!(
            trap_kind("cdz-run: trap: wasm trap: integer overflow: error"),
            Some("overflow")
        );
        // Out of bounds — corpus "index out of bounds" vs wasmtime "out of bounds memory access".
        assert_eq!(trap_kind("index out of bounds"), Some("out-of-bounds"));
        assert_eq!(
            trap_kind("wasm trap: out of bounds memory access"),
            Some("out-of-bounds")
        );
        // Unreachable — corpus bare word vs wasmtime's full phrasing.
        assert_eq!(trap_kind("unreachable"), Some("unreachable"));
        assert_eq!(
            trap_kind("wasm `unreachable` instruction executed"),
            Some("unreachable")
        );
        // An unclassifiable reason yields None (grader stays conservative — never a false Pass).
        assert_eq!(trap_kind("some novel host failure"), None);
    }

    #[test]
    fn opt_sweep_outcome_key_distinguishes_observably_different_runs() {
        // The opt-sweep's blocking guarantee rests on `sweep_outcome_key`: two runs are level-equivalent
        // iff their keys are EQUAL. So the key MUST separate every observably-different outcome — a value
        // change, a value-vs-trap, a trap-KIND change, a decline-code change — else a real cross-level
        // divergence (an unsound optimization) would compare equal and slip past the gate.
        let val = |v: &str| Ran::Value(v.to_string(), vec![]);
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
                code: Some("CDZ0302".into())
            }),
            sweep_outcome_key(&Ran::Declined {
                code: Some("CDZ0304".into())
            })
        );
    }

    #[test]
    fn opt_sweep_outcome_key_separates_distinct_unclassified_traps() {
        // The review-fix-3 invariant (PR #529 Copilot): an UNCLASSIFIED trap reason must NOT collapse to a
        // single "other" bucket, or two genuinely-different unclassified traps would compare EQUAL and a
        // real cross-level trap divergence among them would be MISSED. Both reasons are `trap_kind` = None,
        // so the key falls back to the raw first line and they must differ.
        assert_eq!(trap_kind("novel host failure A"), None);
        assert_eq!(trap_kind("novel host failure B"), None);
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
        // Trapped, but the reason does not match (or classify) → Todo, never a false Pass.
        assert!(matches!(
            grade_trial(
                "trap index out of bounds",
                &Ran::Trap("cdz-run: trap: wasm trap: integer overflow: bt".to_string())
            ),
            Grade::Todo
        ));
        // A program the corpus says traps that instead ran to a value → Fail (the miscompile signal).
        assert!(matches!(
            grade_trial("trap divide by zero", &Ran::Value("5".to_string(), vec![])),
            Grade::Fail(_)
        ));
        // A compile-time rejection (the overflow caught as CDZ0302 before running) → Todo, not Fail.
        assert!(matches!(
            grade_trial(
                "trap integer overflow",
                &Ran::Declined {
                    code: Some("CDZ0302".to_string())
                }
            ),
            Grade::Todo
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
        );
        assert!(
            expr.contains("(record (") && expr.contains("(__r).0") && expr.contains("(__r).1"),
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
        );
        assert!(
            nested.contains("(record (") && nested.contains("(tuple "),
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
        );
        assert!(
            expr.contains("(tuple ") && expr.contains("(__r).0") && expr.contains("(__r).1"),
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
        );
        assert_eq!(
            expr, "\"(tuple)\".to_string()",
            "an empty tuple renders the literal `(tuple)`, no path read, no trailing space: {expr}"
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
        let expr = cdz_render_expr("W", &ds, &nt, &std::collections::HashMap::new(), None, None);
        assert!(
            expr.contains("(P {} {})") && expr.contains("(__p).0") && expr.contains("(__p).1"),
            "a multi-payload variant spreads its payloads flat under the name: {expr}"
        );
        assert!(
            !expr.contains("(P {})"),
            "a multi-payload variant must NOT render as one nested tuple payload: {expr}"
        );
        // A single-tuple-payload variant keeps the nested tuple — `(Q {})` where `{}` is `(tuple …)`.
        let vexpr = cdz_render_expr("V", &ds, &nt, &std::collections::HashMap::new(), None, None);
        assert!(
            vexpr.contains("(Q {})") && vexpr.contains("(tuple "),
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
}

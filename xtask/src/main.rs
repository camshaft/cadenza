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
    },
    /// The omnibus health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
    /// the wasm runtime build, and the behavior gate. Each step's output is captured to a log file
    /// (`target/xtask-logs/`); the console shows one ✓ per step, and the first failing step prints
    /// the whole log + its path.
    Check,
    /// Round-trip every corpus program through the syntax surfaces (sexpr→binary→sexpr and
    /// sexpr→ml→sexpr) and confirm each reproduces a structurally-equal AST — guards `cadenza-syntax`
    /// independently of whether the compiler can compile anything.
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
        } => gate(
            &paths,
            profile,
            GateOpts {
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
            },
        ),
        Cmd::Check => check(&paths, profile),
        Cmd::Roundtrip { files } => roundtrip(&paths, profile, files),
        Cmd::Fmt { files, to, check } => fmt(&paths, profile, files, &to, check),
        Cmd::Emit { file, from, out } => emit(&paths, profile, &file, &from, out),
        Cmd::Codegen { check } => codegen::run(&paths, check),
        Cmd::Bench { save } => bench::run(&paths, save),
        Cmd::Miri { filter } => miri(&paths, &filter),
        Cmd::GuideWasm { store } => guide_wasm(&paths, store),
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

/// The workspace directory anchors, resolved once from this crate's manifest location. xtask lives
/// at `<repo>/xtask`, so the repo root is the manifest's parent and the seed workspace is the fixed
/// `<repo>/implementation/seed` beneath it. Every path derives from these two — no fragile
/// `.parent().parent()` chains, and correct inside a git worktree (each worktree's manifest dir
/// resolves to that worktree's own root).
struct Paths {
    /// `<repo>` — the workspace root (parent of `<repo>/xtask`).
    repo: PathBuf,
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
        if let Err(e) = cmd!(sh, "wasm-pack build --target web --release").run() {
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
    if let Err(e) = cmd!(
        sh,
        "cargo build --quiet --profile {profile} -p cdz -p cdz-corpus -p cdz-run"
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
    Tools {
        syntax: cdz.clone(),
        corpus: bin.join("cdz-corpus"),
        rcdzc: cdz,
        run: bin.join("cdz-run"),
    }
}

/// The outcome of driving one program (sexpr text) through the pipeline.
enum Ran {
    /// Ran to a value, rendered to canonical text.
    Value(String),
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
    call: Option<&Call>,
    target: GateTarget,
) -> Ran {
    match target {
        GateTarget::Wasm => run_program_wasm(tools, store, program, call),
        GateTarget::Rust => run_program_rust(tools, program, call, false),
        GateTarget::RustAsync => run_program_rust(tools, program, call, true),
    }
}

/// Drive one program through cdz-syntax → rcdzc (wasm) → cdz-run — the historical path.
fn run_program_wasm(
    tools: &Tools,
    store: &Option<PathBuf>,
    program: &str,
    call: Option<&Call>,
) -> Ran {
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

    // Stage 2: AST → component; capture stderr so a decline carries its diagnostic.
    let rcdzc = Command::new(&tools.rcdzc)
        .args(["compile", "-", "-o", "-"])
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = rcdzc.wait_with_output().expect("wait rcdzc");
    let _ = syntax.wait();
    if !rcdzc_out.status.success() {
        // A rejection: recover the diagnostic CODE from the first `error [CODE]` line rcdzc printed to
        // stderr (`rcdzc: error [CDZ0210] (node …): …`). A TYPED rejection carries a code; a codeless
        // DECLINE (unimplemented construct) carries none. `grade_ran` uses this to match `(error CODE)`.
        return Ran::Declined {
            code: first_error_code(&rcdzc_out.stderr),
        };
    }

    // Stage 3: run the component (its stdout is the value; a trap goes to stderr with exit 1).
    let mut run = Command::new(&tools.run);
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
    let mut child = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&rcdzc_out.stdout)
        .ok();
    let run_out = child.wait_with_output().expect("wait cdz-run");
    if run_out.status.success() {
        Ran::Value(String::from_utf8_lossy(&run_out.stdout).trim().to_string())
    } else {
        Ran::Trap(first_line(&run_out.stderr))
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
fn run_program_rust(tools: &Tools, program: &str, call: Option<&Call>, async_mode: bool) -> Ran {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Stage 1+2: program text → binary AST → Rust source (rcdzc `--target rust[-async] -o -`, on stdout).
    let rust_target = if async_mode { "rust-async" } else { "rust" };
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
        // A shared-front rejection/decline — identical outcome to the wasm path.
        return Ran::Declined {
            code: first_error_code(&rcdzc_out.stderr),
        };
    }
    let module = String::from_utf8_lossy(&rcdzc_out.stdout).to_string();

    // The export to invoke, and the call expression. The gate passes bare value text (`20`, `-1`,
    // `true`); written verbatim they are valid Rust literals whose type the fn signature fixes. A
    // negative arg is a valid Rust expression too. With no `(call …)`, invoke the sole export nullary.
    let (export, call_expr) = match call {
        Some(c) => (
            rust_ident(&c.export),
            format!("{}({})", rust_ident(&c.export), c.args.join(", ")),
        ),
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

    // A per-TRIAL temp dir, keyed by a content hash of the program AND its call expression. The key
    // MUST include the call: one program is driven by several trials (a `(call …)` case runs the same
    // export with different args) and the gate grades trials IN PARALLEL — keying on the program alone
    // would point every trial at the SAME `prog.rs`/`prog`, so one worker recompiles `prog` while
    // another execs it (a write-vs-exec race → "text file busy" / permission-denied, the flake this
    // fixes). Distinct call expressions get distinct dirs, so parallel trials never touch one path.
    let key = fnv1a(&format!("{program}\u{0}{call_expr}"));
    let dir = std::env::temp_dir().join(format!("rcdzc-gate-rust-{key:016x}"));
    let _ = std::fs::create_dir_all(&dir);
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
    let body = match ret_ty.as_deref().map(|ty| cdz_render_expr(ty, &sums)) {
        Some(render) => {
            format!("fn main() {{ let __r = {call_or_await}; println!(\"{{}}\", {render}); }}\n")
        }
        // Unknown return type (no emitted signature parsed) — fall back to `{}` (a scalar).
        None => format!("fn main() {{ println!(\"{{}}\", {call_or_await}); }}\n"),
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
    // that does not build) — the exact miscompile class this gate catches.
    let compiled = Command::new("rustc")
        .args(["-O", "--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output();
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
        Ran::Value(String::from_utf8_lossy(&run.stdout).trim().to_string())
    } else {
        Ran::Trap(first_line(&run.stderr))
    }
}

/// The async gate driver's harness: a no-limit `GateEnv` implementing the emitted `CdzEnv` (the gate
/// checks ANSWERS, not fuel bounds, so `consume` never blocks/panics) + a minimal `block_on` executor
/// (a real Waker is unneeded — the emitted futures never register one; they only `.await` `consume`,
/// which is `Ready` immediately, so a busy-poll loop drives them to completion). Spliced into the async
/// driver before `fn main`.
const ASYNC_GATE_HARNESS: &str = r#"
struct GateEnv;
impl prog::CdzEnv for GateEnv {
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

/// A Rust EXPRESSION that renders the driver's result binding `__r` (whose CADENZA type is `ty`, in
/// `render_name` form) to cdz-run's canonical text form — the value the gate grades against. Type-
/// directed and recursive over the Cadenza type:
///  - `Unit` → the token `unit`;
///  - `(Tuple T0 T1 …)` → `(tuple <r.0> <r.1> …)`;
///  - `(Record (a T0) (b T1) …)` → `(record (a <r.0>) (b <r.1>) …)` — the fields are already in sorted
///    order (both the type's render and the emitted Rust tuple order them the same), so element `i`
///    reads `.i`;
///  - any scalar (`Int64`, `Bool`, …) → `{}` (an integer/bool `Display`s exactly as cdz-run prints it).
fn cdz_render_expr(ty: &str, sums: &std::collections::HashMap<String, Vec<(String, Option<String>)>>) -> String {
    cdz_render_at(ty, "__r", sums)
}

/// The recursive worker for [`cdz_render_expr`]: `path` is the Rust access path to the value being
/// rendered (starts at `__r`, descends `.0`/`.1`… into tuple/record elements — a record IS a positional
/// tuple in sorted-field order, so its `i`th field is `.i`).
fn cdz_render_at(
    ty: &str,
    path: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Option<String>)>>,
) -> String {
    let ty = ty.trim();
    if ty == "Unit" {
        return "\"unit\".to_string()".to_string();
    }
    // `(Tuple T0 T1 …)` → `(tuple …)`.
    if let Some(elems) = parse_head_type(ty, "Tuple") {
        let placeholders = vec!["{}"; elems.len()].join(" ");
        let args: Vec<String> = elems
            .iter()
            .enumerate()
            .map(|(i, e)| cdz_render_at(e, &format!("({path}).{i}"), sums))
            .collect();
        return format!("format!(\"(tuple {placeholders})\", {})", args.join(", "));
    }
    // `(record (a T0) (b T1) …)` → `(record (a …) (b …) …)`. Each element is a `(name Type)` pair; the
    // fields are in sorted order (matching the emitted tuple), so field `i` reads `.i`. (`Ty::render_name`
    // writes the record head LOWERCASE — `(record …)` — vs the tuple's capital `(Tuple …)`.)
    if let Some(fields) = parse_head_type(ty, "record") {
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
            args.push(cdz_render_at(fty.trim(), &format!("({path}).{i}"), sums));
        }
        let groups = vec!["({} {})"; fields.len()].join(" ");
        return format!("format!(\"(record {groups})\", {})", args.join(", "));
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
        let inner = cdz_render_at(payload, &vbind, sums);
        return format!(
            "match &{path} {{ Some({vbind}) => format!(\"(Some {{}})\", {inner}), None => \"(None unit)\".to_string() }}"
        );
    }
    if let Some(args) = parse_head_type(ty, "Result") {
        let ok = cdz_render_at(args.first().map(String::as_str).unwrap_or(""), &vbind, sums);
        let err = cdz_render_at(args.get(1).map(String::as_str).unwrap_or(""), &vbind, sums);
        return format!(
            "match &{path} {{ Ok({vbind}) => format!(\"(Ok {{}})\", {ok}), Err({vbind}) => format!(\"(Err {{}})\", {err}) }}"
        );
    }
    // A USER sum — a bare type name (`Opt`, `P`, `E`) with an emitted `// cdz-sum[…]` descriptor giving its
    // variants (name + payload type) in discriminant order. Render by MATCHING into cdz-run's BARE form,
    // uniform with a built-in sum: a payload variant → `(<Variant> <payload>)` (payload rendered
    // recursively from its type); a nullary variant → `(<Variant> unit)`. The Rust variant identifier is
    // the SANITIZED name (matching the emitted enum); the printed name is the CADENZA variant name (the
    // descriptor's first token). A generic user sum has no descriptor (its payload is a type parameter) —
    // it falls through to the scalar path, which does not `Display` an enum, so the compile error stays a
    // clear signal rather than a silent wrong render (no corpus case escapes a generic user sum).
    if let Some(variants) = sums.get(ty) {
        // The enum is defined INSIDE `mod prog { … }` (the driver wraps the emitted module), so the
        // driver's `fn main` names it qualified: `prog::<Enum>::<Variant>`. (A built-in Option/Result is
        // std's, unqualified — handled above.)
        let mut arms = Vec::with_capacity(variants.len());
        for (vname, payload) in variants {
            let vident = rust_ident(vname);
            match payload {
                Some(pty) => {
                    let inner = cdz_render_at(pty, &vbind, sums);
                    arms.push(format!(
                        "prog::{ty}::{vident}({vbind}) => format!(\"({vname} {{}})\", {inner})"
                    ));
                }
                None => arms.push(format!(
                    "prog::{ty}::{vident} => \"({vname} unit)\".to_string()"
                )),
            }
        }
        return format!("match &{path} {{ {} }}", arms.join(", "));
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
) -> std::collections::HashMap<String, Vec<(String, Option<String>)>> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-sum[") else {
            continue;
        };
        let Some((ident, groups)) = rest.split_once("]:") else {
            continue;
        };
        // Each top-level `(…)` group is one variant: its first token is the Cadenza variant name, the
        // remainder (if any) is the payload type's render_name. `split_top_level` respects nesting, so a
        // record/tuple payload `(Pt (record (x Int64) (y Int64)))` stays one group.
        let variants: Vec<(String, Option<String>)> = split_top_level(groups.trim())
            .iter()
            .filter_map(|g| {
                let inner = g.strip_prefix('(')?.strip_suffix(')')?.trim();
                match inner.split_once(char::is_whitespace) {
                    Some((name, payload)) => {
                        Some((name.trim().to_string(), Some(payload.trim().to_string())))
                    }
                    None => Some((inner.to_string(), None)),
                }
            })
            .collect();
        map.insert(ident.trim().to_string(), variants);
    }
    map
}

/// Split a `render_name` head-applied type `(<Head> A B …)` into its argument strings, or `None` if `ty`
/// is not `(<Head> …)`. Respects nesting — a space or paren inside a nested `(…)` group does not split.
/// Used to destructure `(Tuple T0 T1)` and `(Record (a T0) (b T1))`.
fn parse_head_type(ty: &str, head: &str) -> Option<Vec<String>> {
    let inner = ty.strip_prefix('(')?.strip_suffix(')')?.trim();
    let rest = inner.strip_prefix(head)?;
    // `head` must be a whole token (so `(Tuple …)` doesn't match a hypothetical `(TupleX …)`).
    if !rest.starts_with(char::is_whitespace) {
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
                .map(|t| run_program(tools, store, &rec.program, t.call.as_ref(), target))
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
                    Ran::Value(v) => format!("value {v}"),
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
    let mut trials: Vec<Trial> = Vec::new();
    let (mut call_export, mut call_args): (Option<String>, Vec<String>) = (None, Vec::new());
    for line in text.lines() {
        if line == "---" {
            records.push(CorpusRecord {
                description: std::mem::take(&mut desc),
                program: std::mem::take(&mut prog),
                trials: std::mem::take(&mut trials),
                needs: std::mem::take(&mut needs),
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
        .map(|t| run_program(tools, store, &rec.program, t.call.as_ref(), target))
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
                Ran::Value(v) if *v == expected_val || *v == expected_full => Grade::Pass,
                Ran::Value(v) => Grade::Fail(format!("expected {expected_full}, ran → {v}")),
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
                Ran::Value(v) => Grade::Fail(format!("expected rejection {want}, ran → {v}")),
                Ran::Declined { code: Some(got) } if got == want => Grade::Pass,
                // Rejected (a different code), a codeless decline, or a trap — refused, not miscompiled.
                Ran::Declined { .. } | Ran::Trap(_) => Grade::Todo,
                // A broken artifact cannot validate a rejection CODE (the front-end never rejected — the
                // BACK end failed to build a value it accepted), so it is not a clean signal here: Todo.
                Ran::BadArtifact(_) => Grade::Todo,
            }
        }
        // `trap …`: matching a trap reason needs machinery not yet wired (the runtime message). Count as
        // todo unless a clear disagreement — a program the corpus says TRAPS that instead ran to a value.
        "trap" => match ran {
            Ran::Value(v) => Grade::Fail(format!("expected a trap, ran → {v}")),
            // A broken artifact for a case that should TRAP is still a miscompile (the backend was asked
            // for a runnable artifact that traps and emitted un-compilable source instead).
            Ran::BadArtifact(e) => {
                Grade::Fail(format!("expected a trap, artifact did not build: {e}"))
            }
            _ => Grade::Todo,
        },
        _ => Grade::Todo,
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
    };
    paths.repo.join("spec/semantics").join(name)
}

/// Write the current verdicts as the baseline: one `verdict\tdescription` line per case, sorted by
/// description so the file is stable and a diff is meaningful.
fn save_baseline(paths: &Paths, verdicts: &[(String, Verdict)], target: GateTarget) {
    let mut lines: Vec<String> = verdicts
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
    let mut base: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            base.insert(d.to_string(), verdict);
        }
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
/// the wasm runtime build, and the behavior gate. Each step's full output is CAPTURED to a log file
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
    let repo = &paths.repo;
    log.step("fmt", "cargo fmt --all --check", repo);
    log.step("build", "cargo build --workspace", repo);
    log.step("test", "cargo test --workspace", repo);
    log.step("clippy", "cargo clippy --workspace -- -D warnings", repo);

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

    println!("\ncheck: all green ✓  (full log: {})", log.path.display());
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

/// For every corpus program, confirm sexpr→binary→sexpr and sexpr→ml→sexpr reproduce a
/// structurally-equal AST — i.e. re-encoding the round-tripped text to binary yields the SAME bytes
/// as the original. Guards `cadenza-syntax` (reader/printer/codec) independently of the compiler.
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
                            for surface in ["sexpr", "ml"] {
                                match roundtrip_via(tools, &bin0, surface) {
                                    Some(bin1) if bin1 == bin0 => {}
                                    Some(_) => failures.push(format!(
                                        "{}: binary≠binary via {surface}",
                                        rec.description
                                    )),
                                    None => failures.push(format!(
                                        "{}: round-trip via {surface} errored",
                                        rec.description
                                    )),
                                }
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

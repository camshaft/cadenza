//! `xtask-mandates` — the standalone mandate-lint binary (v-xtask-decompose). Runs the mechanizable
//! STANDING-MANDATE checks over `implementation/**/*.rs` and exits non-zero on any violation. Invoked
//! by the nix app (`nix run .#lint-mandates`) with `CDZ_REPO_ROOT` set to the invoking worktree; falls
//! back to the current directory. The monolith `cargo xtask lint-mandates` dispatches to the same
//! `xtask_mandates::lint_mandates` library fn, so the two can't drift.

use std::path::PathBuf;

fn main() {
    // The repo root: `CDZ_REPO_ROOT` (set by the nix app, since a relocated nix binary can't self-locate
    // the source tree) else the current directory (the `cargo run -p xtask-mandates` / bare-invocation path).
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));

    // `xtask-mandates declines` runs ONLY the seq-280 decline-professionalism scan and nothing else — a
    // SEPARATE check surface so v-fleet-tooling can wire it (a `.#lint-declines` app + `decline-
    // professionalism` gate check) and fold it into localGate INDEPENDENTLY of the default mandate set,
    // AFTER v-deferral's 3 residue sites land (ratchet-at-zero, no grandfather). The default (no arg) run
    // is UNCHANGED — it must not carry this check, else it would red the already-folded mandate gate early.
    if std::env::args().nth(1).as_deref() == Some("declines") {
        match xtask_mandates::lint_decline_professionalism(&repo) {
            Ok(v) if v.is_empty() => {
                println!("lint-declines: ok — no deferral wording in decline messages");
            }
            Ok(violations) => {
                for x in &violations {
                    eprintln!("lint-declines: {}:{}", x.file.display(), x.reason);
                }
                eprintln!(
                    "lint-declines: {} decline-professionalism violation(s)",
                    violations.len()
                );
                std::process::exit(1);
            }
            Err(msg) => {
                eprintln!("lint-declines: {msg}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mut failed = false;

    // (1) The syn-based STANDING MANDATES (no-integration-tests, no-hard-coded-runtime-hash, …).
    match xtask_mandates::lint_mandates(&repo) {
        Ok(v) if v.is_empty() => println!("lint-mandates: ok — no mandate violations"),
        Ok(violations) => {
            for x in &violations {
                eprintln!("lint-mandates: {}: {}", x.file.display(), x.reason);
            }
            eprintln!("lint-mandates: {} mandate violation(s)", violations.len());
            failed = true;
        }
        Err(msg) => {
            eprintln!("lint-mandates: {msg}");
            failed = true;
        }
    }

    // (2) The FILE-SIZE mandate (operator shrink-initiative seq-274): fail on an oversized `implementation/
    // **/*.rs` not on the self-expiring FILE_SIZE_ALLOWLIST, and on a STALE allowlist entry. Its SINGLE
    // source of truth is `xtask_support::file_size_lint` — the monolith `cargo xtask check` runs the SAME
    // fn, so the two can't drift. Folding it into this binary (which `mandateLintCheck` runs, folded into
    // localGate) gives the shrink initiative TEETH under self-merge: a red makes gate-local HOLD. A GitHub
    // required-status can't, because `gh pr merge --admin` bypasses required checks. Run REGARDLESS of (1)'s
    // result so both lints report in one pass.
    match xtask_support::file_size_lint(&repo) {
        Ok(()) => println!("lint-mandates: ok — no oversized source files"),
        Err(msg) => {
            eprintln!("lint-mandates(file-size): {msg}");
            failed = true;
        }
    }

    if failed {
        std::process::exit(1);
    }
}

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
    match xtask_mandates::lint_mandates(&repo) {
        Ok(v) if v.is_empty() => println!("lint-mandates: ok — no mandate violations"),
        Ok(violations) => {
            for x in &violations {
                eprintln!("lint-mandates: {}: {}", x.file.display(), x.reason);
            }
            eprintln!("lint-mandates: {} mandate violation(s)", violations.len());
            std::process::exit(1);
        }
        Err(msg) => {
            eprintln!("lint-mandates: {msg}");
            std::process::exit(1);
        }
    }
}

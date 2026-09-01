//! `xtask-fmt` — format Cadenza program file(s) through the printer, rewriting them in place (or, with
//! `--check`, exiting non-zero if any file is not already formatted). Formatting = parse the surface and
//! re-print it canonically (same surface in and out), via `cdz convert --from <to> --to <to>`.
//!
//! Carved out of the xtask monolith into its own crate (v-xtask-decompose). The conversion is
//! `xtask_support::convert_bytes` (the shared cdz-driver helper, also used by roundtrip) — one source of
//! truth. The `cdz` binary comes from `CDZ_SEED_BIN_DIR` (the nix `apps.fmt` wrapper injects the warm
//! nix-built cdz) — no cargo build; falling back to `<CDZ_REPO_ROOT|cwd>/target/debug` for a bare
//! `cargo run -p xtask-fmt` (dev), where a prior `cargo build` left the bin.
//!
//! Usage: `xtask-fmt [--to <surface>] [--check] <file>…` — `--to` defaults to `sexpr`.

use std::path::PathBuf;
use xtask_support::convert_bytes;

fn main() {
    // Hand-rolled arg parse (no clap — minimal deps): `--to <surface>`, `--check`, else a positional file.
    let mut files: Vec<PathBuf> = Vec::new();
    let mut to = String::from("sexpr");
    let mut check = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--to") => {
                to = args
                    .next()
                    .and_then(|v| v.to_str().map(str::to_string))
                    .unwrap_or_else(|| {
                        eprintln!("xtask fmt: --to needs a surface argument");
                        std::process::exit(1);
                    });
            }
            Some("--check") => check = true,
            _ => files.push(PathBuf::from(arg)),
        }
    }

    if files.is_empty() {
        eprintln!("xtask fmt: name at least one file");
        std::process::exit(1);
    }

    // The nix-built cdz (surface conversions), from the dir the `apps.fmt` wrapper points CDZ_SEED_BIN_DIR
    // at. Falls back to `<repo>/target/debug` for a bare `cargo run -p xtask-fmt` (dev).
    let repo = xtask_support::repo_root();
    let bin_dir = xtask_support::seed_bin_dir(&repo);
    let cdz = bin_dir.join("cdz");

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
        let formatted = match convert_bytes(&cdz, &original, &to, &to) {
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

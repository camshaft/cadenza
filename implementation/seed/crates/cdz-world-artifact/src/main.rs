//! `cdz-world-artifact` — the CLI over [`cdz_world_artifact::Worlds`].
//!
//! Parse a WIT world declaration and emit each world's `KIND_WIT_WORLD` binary-AST artifact. A build step
//! (the nix reducer-guest derivation, and `cargo xtask world-artifact`) SHELLS OUT to this binary rather
//! than linking the library, so the WIT→artifact logic stays fully isolated from the build tool (operator
//! directive 2026-08-24 — decompose xtask into small utility programs, same pattern as `cdz-component-rewrite`).
//!
//! ```text
//! cdz-world-artifact <world.wit> <out-dir> [<world> ...]
//! ```
//!
//! Parses `<world.wit>`, then writes `<out-dir>/<world>.bin` for each named world — or, if none are named,
//! for EVERY world the document declares (no world name is baked into the tool). Prints one line per artifact
//! to stdout. Exits non-zero on an I/O / parse / build error, or if a named world is not declared.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!(
                "cdz-world-artifact: {msg}\n\nusage: cdz-world-artifact <world.wit> <out-dir> [<world> ...]"
            );
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let [wit_path, out_dir, worlds @ ..] = args else {
        return Err(
            "expected <world.wit> <out-dir> and optionally one or more <world> names".into(),
        );
    };
    let wit_path = Path::new(wit_path);
    let out_dir = Path::new(out_dir);

    let wit_src = std::fs::read_to_string(wit_path)
        .map_err(|e| format!("reading {}: {e}", wit_path.display()))?;
    let doc = cdz_world_artifact::Worlds::parse(&wit_path.display().to_string(), &wit_src)?;

    // No world named on the command line → emit every world the document declares.
    let selected: Vec<String> = if worlds.is_empty() {
        let all = doc.names();
        if all.is_empty() {
            return Err(format!("{} declares no world", wit_path.display()));
        }
        all
    } else {
        worlds.to_vec()
    };

    std::fs::create_dir_all(out_dir).map_err(|e| format!("creating {}: {e}", out_dir.display()))?;

    for world in &selected {
        let bytes = doc.artifact(world)?;
        let out = out_dir.join(format!("{world}.bin"));
        std::fs::write(&out, &bytes).map_err(|e| format!("writing {}: {e}", out.display()))?;
        println!(
            "cdz-world-artifact: wrote {} ({} bytes) from {}",
            out.display(),
            bytes.len(),
            wit_path.display()
        );
    }
    Ok(())
}

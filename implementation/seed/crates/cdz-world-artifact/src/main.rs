//! `cdz-world-artifact` — the CLI over [`cdz_world_artifact::Worlds`].
//!
//! Parse a WIT world declaration and emit each world's `KIND_WIT_WORLD` binary-AST artifact. A build step
//! (the nix reducer-guest derivation, and `cargo xtask world-artifact`) SHELLS OUT to this binary rather
//! than linking the library, so the WIT→artifact logic stays fully isolated from the build tool (operator
//! directive 2026-08-24 — decompose xtask into small utility programs, same pattern as `cdz-component-rewrite`).
//!
//! ```text
//! cdz-world-artifact <world.wit> <out-dir> [<world> ...] [--dep <dep.wit>]...
//! ```
//!
//! Parses `<world.wit>`, then writes `<out-dir>/<world>.bin` for each named world — or, if none are named,
//! for EVERY world the document declares (no world name is baked into the tool). Prints one line per artifact
//! to stdout. Exits non-zero on an I/O / parse / build error, or if a named world is not declared.
//!
//! `--dep <dep.wit>` (repeatable) names a WIT file in ANOTHER package that `<world.wit>` imports (e.g. a
//! test world importing `cadenza:platform/reducer`, whose package lives in the sibling `world.wit`). Each
//! dep is parsed into the same resolve BEFORE the main file so the cross-package import binds. Name the
//! world(s) to emit so only the main document's world is written, not the deps'.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!(
                "cdz-world-artifact: {msg}\n\nusage: cdz-world-artifact <world.wit> <out-dir> [<world> ...] [--dep <dep.wit>]..."
            );
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    // Split off `--dep <path>` pairs (repeatable) from the positional args, in one pass.
    let mut positional: Vec<String> = Vec::new();
    let mut dep_paths: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--dep" {
            let p = it
                .next()
                .ok_or_else(|| "--dep expects a <dep.wit> path".to_string())?;
            dep_paths.push(p.clone());
        } else {
            positional.push(a.clone());
        }
    }
    let [wit_path, out_dir, worlds @ ..] = positional.as_slice() else {
        return Err(
            "expected <world.wit> <out-dir> and optionally one or more <world> names".into(),
        );
    };
    let wit_path = Path::new(wit_path);
    let out_dir = Path::new(out_dir);

    // Read each dependency package's source (pushed into the resolve before the main file).
    let deps: Vec<(String, String)> = dep_paths
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .map(|src| (p.clone(), src))
                .map_err(|e| format!("reading dep {p}: {e}"))
        })
        .collect::<Result<_, _>>()?;

    let wit_src = std::fs::read_to_string(wit_path)
        .map_err(|e| format!("reading {}: {e}", wit_path.display()))?;
    let doc = cdz_world_artifact::Worlds::parse_with_deps(
        &deps,
        &wit_path.display().to_string(),
        &wit_src,
    )?;

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

#[cfg(test)]
mod tests {
    use super::run;
    use std::path::{Path, PathBuf};

    /// A self-contained two-world WIT fixture (no read of any sibling crate's file). `two` declared worlds
    /// so the "emit every world" default is distinguishable from "emit the first".
    const FIXTURE: &str = r#"
        package cadenza:fixture;
        interface a { foo: func() -> u32; }
        interface b { bar: func(x: list<u8>) -> option<list<u8>>; }
        world one { export a; }
        world two { import b; export a; }
    "#;

    /// A fresh, uniquely-named temp dir (test names + pid keep parallel tests from colliding).
    fn tmp(sub: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cdz-wa-test-{}-{sub}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_fixture(dir: &Path) -> String {
        let p = dir.join("world.wit");
        std::fs::write(&p, FIXTURE).unwrap();
        p.display().to_string()
    }

    /// The load-bearing default the `worldArtifacts` nix derivation relies on: with NO world names, emit a
    /// `<world>.bin` for EVERY world the document declares (a regression to "first world only" would drop
    /// `event-reducer-world.bin` and break the privileged guests).
    #[test]
    fn no_world_names_emits_every_declared_world() {
        let dir = tmp("all");
        let wit = write_fixture(&dir);
        let out = dir.join("out");
        run(&[wit, out.display().to_string()]).expect("run succeeds");
        assert!(out.join("one.bin").exists(), "one.bin was emitted");
        assert!(out.join("two.bin").exists(), "two.bin was emitted");
    }

    /// A named world emits only that one (the other declared world is not written).
    #[test]
    fn a_named_world_emits_only_that_world() {
        let dir = tmp("named");
        let wit = write_fixture(&dir);
        let out = dir.join("out");
        run(&[wit, out.display().to_string(), "two".into()]).expect("run succeeds");
        assert!(out.join("two.bin").exists(), "the named world is emitted");
        assert!(
            !out.join("one.bin").exists(),
            "an un-named world is NOT emitted"
        );
    }

    /// An unreadable `.wit` path is a clean error (never a silent no-op).
    #[test]
    fn an_unreadable_wit_is_an_error() {
        let dir = tmp("noread");
        let missing = dir.join("nope.wit").display().to_string();
        let out = dir.join("out").display().to_string();
        let err = run(&[missing, out]).expect_err("a missing wit file errors");
        assert!(err.contains("reading"), "error names the read: {err}");
    }

    /// A named world the document does not declare is an error (surfaces from the `Worlds::artifact` call).
    #[test]
    fn an_undeclared_named_world_is_an_error() {
        let dir = tmp("noworld");
        let wit = write_fixture(&dir);
        let out = dir.join("out").display().to_string();
        let err = run(&[wit, out, "nope".into()]).expect_err("an undeclared world errors");
        assert!(err.contains("no world `nope`"), "got: {err}");
    }

    /// Too few positional args is a usage error, not a panic.
    #[test]
    fn too_few_args_is_an_error() {
        let err = run(&["only-one-arg".into()]).expect_err("needs <wit> and <out-dir>");
        assert!(err.contains("expected"), "got: {err}");
    }

    /// A main WIT that IMPORTS another package resolves via `--dep`: the dep package is pushed first so the
    /// cross-package import binds. Without `--dep` the same main file is a parse error (unknown package).
    /// This is the arg-probe-world shape (a test world importing the platform reducer interface).
    #[test]
    fn a_cross_package_import_resolves_via_dep() {
        let dir = tmp("dep");
        let dep = dir.join("dep.wit");
        std::fs::write(
            &dep,
            "package cadenza:platform;\ninterface reducer { step: func() -> u32; }\n",
        )
        .unwrap();
        let main = dir.join("main.wit");
        std::fs::write(
            &main,
            "package cadenza:test-x;\nworld probe-world { import cadenza:platform/reducer; }\n",
        )
        .unwrap();
        let out = dir.join("out");

        // Without --dep: the cross-package import is unresolved → error.
        let err = run(&[main.display().to_string(), out.display().to_string()])
            .expect_err("cross-package import without its dep errors");
        assert!(
            err.contains("parse") || err.contains("package") || err.contains("reducer"),
            "got: {err}"
        );

        // With --dep dep.wit: resolves + emits probe-world.bin.
        run(&[
            main.display().to_string(),
            out.display().to_string(),
            "probe-world".into(),
            "--dep".into(),
            dep.display().to_string(),
        ])
        .expect("--dep resolves the cross-package import");
        assert!(
            out.join("probe-world.bin").exists(),
            "probe-world.bin emitted"
        );
    }
}

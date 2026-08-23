//! `cdz-component-rewrite` — the CLI over [`cdz_component_rewrite::add_import_versions`].
//!
//! Re-address a component's BARE external imports to content-addressed `name@<version>`. A build step
//! (`cargo xtask build`) SHELLS OUT to this binary rather than linking the library, so the rewrite logic
//! stays fully isolated from the build tool (operator directive 2026-08-23).
//!
//! ```text
//! cdz-component-rewrite <input.wasm> <output.wasm> <name>=<version> [<name>=<version> ...]
//! ```
//!
//! Reads `<input.wasm>`, rewrites each listed bare import `<name>` to `<name>@<version>`, writes the
//! result to `<output.wasm>`, and prints the number of imports rewritten to stderr. Exits non-zero on an
//! I/O/decode error, a malformed `name=version` argument, or if NO import was rewritten while mappings were
//! given (a silently-unmatched name — e.g. a renamed interface — is an error, not a no-op).

use std::collections::BTreeMap;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!(
                "cdz-component-rewrite: {msg}\n\nusage: cdz-component-rewrite <input.wasm> <output.wasm> \
                 <name>=<version> [<name>=<version> ...]"
            );
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let [input, output, mappings @ ..] = args else {
        return Err("expected <input.wasm> <output.wasm> and at least one <name>=<version>".into());
    };
    if mappings.is_empty() {
        return Err("no <name>=<version> mappings given".into());
    }

    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    for m in mappings {
        let (name, version) = m
            .split_once('=')
            .ok_or_else(|| format!("mapping `{m}` is not `<name>=<version>`"))?;
        if name.is_empty() || version.is_empty() {
            return Err(format!("mapping `{m}` has an empty name or version"));
        }
        versions.insert(name.to_string(), version.to_string());
    }

    let component = std::fs::read(input).map_err(|e| format!("reading {input}: {e}"))?;
    let (rewritten, n) = cdz_component_rewrite::add_import_versions(&component, &versions)?;
    if n == 0 {
        return Err(format!(
            "no import matched any of the {} requested name(s) — is a name stale/renamed? ({})",
            versions.len(),
            versions.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    std::fs::write(output, rewritten).map_err(|e| format!("writing {output}: {e}"))?;
    eprintln!("cdz-component-rewrite: rewrote {n} import(s) → {output}");
    Ok(())
}

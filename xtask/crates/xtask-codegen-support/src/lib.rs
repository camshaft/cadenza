//! Shared helpers for the `xtask-codegen-*` bins: the sexpr READER (input) + the rust EMIT tail (output),
//! each formerly copy-pasted identically across the bins and single-sourced here so a fix can't drift.
//!
//! READER (`cdz_bin` + `sexpr_to_arenas`): resolve the seed `cdz` and run `cdz convert --from sexpr --to
//! binary` on an authored `.sexp`, decoding the cadenza-ast BINARY to `Arenas` — the operator's SEXPR →
//! binary-AST codegen IR (no-json). declines + wasm-abi shared this verbatim.
//!
//! EMIT (`format_tokens` + `rustfmt_stdin`): every bin builds a `proc_macro2::TokenStream` (via `quote!`),
//! then must render it to BYTE-IDENTICAL committed Rust: parse to a `syn::File`, pretty-print with
//! `prettyplease`, then run `rustfmt` (prettyplease alone diverges from the committed cargo-fmt'd
//! line-wrapping). All three bins shared this (incl. the rustfmt-required hard-error for cdzPlatformContracts).

use std::path::{Path, PathBuf};

/// The seed `cdz` that converts an authored `.sexp` → cadenza-ast binary. From `CDZ_SEED_BIN_DIR` (the
/// nix-built cdz a codegen derivation injects), else `<repo>/target/debug` for dev.
pub fn cdz_bin(repo: &Path) -> PathBuf {
    std::env::var_os("CDZ_SEED_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/debug"))
        .join("cdz")
}

/// Convert `sexpr` to its cadenza-ast BINARY via `cdz convert` and decode it (the codegen IR; no-json).
/// Hard-errors (exit 1) if `cdz` can't run, the convert fails, or the bytes don't decode.
pub fn sexpr_to_arenas(cdz: &Path, sexpr: &Path) -> cadenza_ast::ast::Arenas {
    let out = std::process::Command::new(cdz)
        .args(["convert", "--from", "sexpr", "--to", "binary"])
        .arg(sexpr)
        .output()
        .unwrap_or_else(|e| {
            eprintln!(
                "xtask codegen: could not run `cdz convert` on {}: {e}",
                sexpr.display()
            );
            std::process::exit(1);
        });
    if !out.status.success() {
        eprintln!(
            "xtask codegen: `cdz convert --to binary {}` failed:\n{}",
            sexpr.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        std::process::exit(1);
    }
    cadenza_ast::codec::decode(&out.stdout).unwrap_or_else(|| {
        eprintln!(
            "xtask codegen: `cdz convert {} --to binary` did not produce a decodable cadenza-ast",
            sexpr.display()
        );
        std::process::exit(1);
    })
}

/// Parse a generated token tree into a `syn::File`, pretty-print it (`prettyplease`), then run it through
/// `rustfmt` for byte-identical committed output. HARD-ERRORS (exit 1) if `rustfmt` is unavailable:
/// prettyplease alone diverges from the committed cargo-fmt'd form, so a rustfmt-less run would silently
/// emit MIS-FORMATTED source that a caller could commit/overlay.
pub fn format_tokens(tokens: proc_macro2::TokenStream) -> String {
    let file = syn::parse2::<syn::File>(tokens)
        .unwrap_or_else(|e| panic!("xtask codegen: generated tokens did not parse (a bug): {e}"));
    let pretty = prettyplease::unparse(&file);
    rustfmt_stdin(&pretty).unwrap_or_else(|| {
        eprintln!(
            "xtask codegen: `rustfmt` is required on PATH (prettyplease alone diverges from the committed \
             cargo-fmt'd form → mis-formatted output). Install the pinned toolchain's rustfmt."
        );
        std::process::exit(1);
    })
}

/// Run `src` through the `rustfmt` binary (stdin→stdout). `None` if rustfmt is unavailable or errors.
pub fn rustfmt_stdin(src: &str) -> Option<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(src.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn format_tokens_pretty_prints_then_rustfmts() {
        // rustfmt is on PATH in the pinned dev/nix env; format_tokens hard-errors otherwise (by design),
        // so a green run here also proves the rustfmt-required contract holds in the test environment.
        let out = format_tokens(quote! { pub fn f()->u8{1} });
        assert!(out.contains("pub fn f() -> u8"), "got: {out}");
    }

    #[test]
    fn rustfmt_stdin_reformats_valid_rust() {
        let out = rustfmt_stdin("fn  f( )  ->u8{1}").expect("rustfmt on PATH in the dev/nix env");
        assert!(out.contains("fn f() -> u8"), "got: {out}");
    }
}

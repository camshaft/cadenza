//! Shared rust-emit tail for the `xtask-codegen-*` bins.
//!
//! Every codegen bin builds a `proc_macro2::TokenStream` (via `quote!`), then must render it to
//! BYTE-IDENTICAL committed Rust: parse to a `syn::File`, pretty-print with `prettyplease`, then run
//! `rustfmt` (prettyplease alone diverges from the committed cargo-fmt'd line-wrapping). This tail was
//! copy-pasted identically into `xtask-codegen-{declines,wasm-abi,contracts}`; single-sourced here so a
//! fix (e.g. the rustfmt-required hard-error v-nix added for the cdzPlatformContracts wiring) can't drift.

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

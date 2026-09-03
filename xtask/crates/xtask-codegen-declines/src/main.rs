//! `xtask-codegen-declines` — generate `rcdzc/src/diag/declines_generated.rs` (the `DeclineId` catalog)
//! FROM the hand-authored `data/unsupported.sexp` source of truth. Increment 2 of the unsupported-error
//! tracker (`implementation/design/DESIGN-unsupported-tracker.md`, operator seq-286-broad + seq-106).
//!
//! Operator seq-106 flow: sexpr (SOURCE) → xtask codegen → rust module → rcdzc CONSUMES it (via
//! `mod declines_generated;`). This bin reads `data/unsupported.sexp` as cadenza-ast BINARY (`cdz convert
//! --from sexpr --to binary`, the codegen IR — dogfoods cadenza-ast, no-json), walks it, and emits the
//! `DeclineId` enum + `impl` (`ALL` / `key` / `code` / `reason`) as rust tokens. It deps ONLY cadenza-ast
//! (+ the rust-emit stack) — NOT rcdzc — so the fleet hot path never rebuilds the compiler (seq-102). No
//! `--oracle-check`: the sexpr IS the sole source of truth for decline codes (no external oracle).
//!
//! Repo root from `CDZ_REPO_ROOT` (else cwd); first non-flag arg = output path (default the committed
//! module); `cdz` from `CDZ_SEED_BIN_DIR` (else `<repo>/target/debug`). A `cdzDeclines` nix drift-check
//! (v-nix) regenerates + diffs the committed module (the seed-closure cycle rules out a build-time overlay).

use proc_macro2::TokenStream;
use quote::quote;
use std::path::{Path, PathBuf};
use xtask_codegen_support::format_tokens;

/// One decline catalog entry, read from a `(decline Variant (code Sym) (reason "…") (doc "…")
/// (blocked-on …))` form. `blocked-on` is sexpr-only tracking metadata — NOT read here.
struct Decline {
    variant: String,
    code: String, // a `Code` variant symbol, or "none" for a still-codeless decline
    reason: String,
    doc: String,
}

fn main() {
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let out = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo.join("implementation/seed/crates/rcdzc/src/diag/declines_generated.rs")
        });
    let sexpr = repo.join("data/unsupported.sexp");

    let declines = read_declines(&sexpr_to_arenas(&cdz_bin(&repo), &sexpr));
    let source = format_tokens(render(&declines));

    if let Err(e) = std::fs::write(&out, &source) {
        eprintln!("xtask-codegen-declines: writing {}: {e}", out.display());
        std::process::exit(1);
    }
    println!(
        "xtask-codegen-declines: wrote {} ({} declines)",
        out.display(),
        declines.len()
    );
}

/// Walk the cadenza-ast `(do (decline …) …)` and collect the catalog entries.
fn read_declines(a: &cadenza_ast::ast::Arenas) -> Vec<Decline> {
    use cadenza_ast::ast::Struct;
    let name = |id| {
        a.as_name(id)
            .expect("decline entry: expected a NAME symbol")
            .to_owned()
    };
    let text = |id| {
        a.as_str(id)
            .expect("decline entry: expected a string literal")
            .to_owned()
    };
    // A `(field …)` sub-form's first argument (e.g. `(code Sym)` → Sym, `(reason "…")` → "…").
    let field_arg = |id| {
        let Struct::List(f) = a.get(id) else {
            panic!("decline field is not a list");
        };
        f[1]
    };

    let Struct::List(items) = a.get(a.root) else {
        panic!("data/unsupported.sexp root is not a `(do …)` list");
    };
    let mut out = Vec::new();
    for &child in items.iter().skip(1) {
        let head = a.head_name(child).expect("decline entry has no head name");
        assert_eq!(
            head, "decline",
            "unknown entry head `{head}` (expected `decline`)"
        );
        let Struct::List(f) = a.get(child) else {
            panic!("decline entry is not a list");
        };
        // f[0]=head `decline`, f[1]=VariantSym, then named sub-forms in order: code, reason, doc, blocked-on.
        out.push(Decline {
            variant: name(f[1]),
            code: name(field_arg(f[2])),
            reason: text(field_arg(f[3])),
            doc: text(field_arg(f[4])),
            // f[5] = (blocked-on …) — sexpr-only tracking metadata, intentionally not read.
        });
    }
    out
}

/// Derive the stable kebab-case registry key from a PascalCase variant (split on each uppercase boundary,
/// lowercase, join with '-'). Deterministic + matches the hand-authored keys (Wasm/Prim… are single words).
fn kebab(variant: &str) -> String {
    let mut s = String::new();
    for (i, c) in variant.char_indices() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                s.push('-');
            }
            s.push(c.to_ascii_lowercase());
        } else {
            s.push(c);
        }
    }
    s
}

/// Build the `declines_generated.rs` token tree: the `DeclineId` enum (with per-variant doc comments) +
/// the `ALL` / `key` / `code` / `reason` impl. `code`/`reason`/doc are DERIVED from the sexpr; `key` is
/// derived from the variant. The `Some(Code::#sym)` tokens resolve when rcdzc compiles this module (Code
/// is in scope there — this crate never links `Code`).
fn render(declines: &[Decline]) -> TokenStream {
    let variants = declines.iter().map(|d| {
        let v = ident(&d.variant);
        let doc = &d.doc;
        quote! { #[doc = #doc] #v }
    });
    let all = declines.iter().map(|d| {
        let v = ident(&d.variant);
        quote! { DeclineId::#v }
    });
    let key_arms = declines.iter().map(|d| {
        let v = ident(&d.variant);
        let k = kebab(&d.variant);
        quote! { DeclineId::#v => #k }
    });
    let code_arms = declines.iter().map(|d| {
        let v = ident(&d.variant);
        if d.code == "none" {
            quote! { DeclineId::#v => None }
        } else {
            let c = ident(&d.code);
            quote! { DeclineId::#v => Some(Code::#c) }
        }
    });
    let reason_arms = declines.iter().map(|d| {
        let v = ident(&d.variant);
        let r = &d.reason;
        quote! { DeclineId::#v => #r }
    });

    quote! {
        //! GENERATED by `xtask-codegen-declines` from `data/unsupported.sexp` — DO NOT EDIT.
        //! Edit the sexpr source + run `cargo run -p xtask-codegen-declines` (a `cdzDeclines` drift-check
        //! reds if this committed file is stale). The unsupported-error tracker's `DeclineId` catalog:
        //! a stable, enumerable referent for every construct rcdzc declines to compile (operator seq-286).
        use crate::diag::Code;

        /// The stable, enumerable catalog of every construct rcdzc declines to compile. A `DeclineId` names
        /// a REASON (a capability the compiler does not realize), not a call site; `DeclineId::ALL` is a
        /// complete, by-construction list. Generated from `data/unsupported.sexp` (the source of truth).
        #[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
        pub enum DeclineId {
            #(#variants),*
        }

        impl DeclineId {
            /// The complete catalog (declared order — byte-deterministic).
            pub const ALL: &'static [DeclineId] = &[#(#all),*];

            /// The stable kebab-case registry key (the durable referent `data/unsupported.sexp` pins).
            pub fn key(self) -> &'static str {
                match self { #(#key_arms),* }
            }

            /// The umbrella code this decline carries (`Some(CDZ0900)` = coded; `None` = still codeless).
            pub fn code(self) -> Option<Code> {
                match self { #(#code_arms),* }
            }

            /// A canonical one-line reason, independent of the runtime `format!` message's specifics.
            pub fn reason(self) -> &'static str {
                match self { #(#reason_arms),* }
            }
        }
    }
}

/// A syntactic identifier token from a name string.
fn ident(s: &str) -> proc_macro2::Ident {
    proc_macro2::Ident::new(s, proc_macro2::Span::call_site())
}

// ── reader/format helpers, copied verbatim from xtask-codegen-wasm-abi (generic) ──────────────────────

/// The `cdz` that converts the sexpr → cadenza-ast binary. From `CDZ_SEED_BIN_DIR` (the nix-built cdz the
/// derivation injects), else `<repo>/target/debug` for dev.
fn cdz_bin(repo: &Path) -> PathBuf {
    std::env::var_os("CDZ_SEED_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/debug"))
        .join("cdz")
}

/// Convert `sexpr` to its cadenza-ast BINARY via `cdz convert` and decode it (the codegen IR; no-json).
fn sexpr_to_arenas(cdz: &Path, sexpr: &Path) -> cadenza_ast::ast::Arenas {
    let out = std::process::Command::new(cdz)
        .args(["convert", "--from", "sexpr", "--to", "binary"])
        .arg(sexpr)
        .output()
        .unwrap_or_else(|e| {
            eprintln!(
                "xtask-codegen-declines: could not run `cdz convert` on {}: {e}",
                sexpr.display()
            );
            std::process::exit(1);
        });
    if !out.status.success() {
        eprintln!(
            "xtask-codegen-declines: `cdz convert --to binary {}` failed:\n{}",
            sexpr.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        std::process::exit(1);
    }
    cadenza_ast::codec::decode(&out.stdout).unwrap_or_else(|| {
        eprintln!(
            "xtask-codegen-declines: `cdz convert {} --to binary` did not produce a decodable cadenza-ast",
            sexpr.display()
        );
        std::process::exit(1);
    })
}

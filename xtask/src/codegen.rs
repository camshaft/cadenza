//! `codegen` — generate the value-heap runtime-ABI table the wasm backend consumes.
//!
//! The value-heap runtime interface is declared, once, in the runtime crate's `wit/runtime.wit` — the
//! ABI's source of truth. The compiler emits programs that IMPORT that interface, so it needs each
//! op's name and core signature to build the import section. Rather than hand-transcribe that (a big
//! hard-coded list that could drift from the WIT), this reads the WIT with `wit-parser` and GENERATES
//! a structured Rust table: `crates/rcdzc/src/backend/wasm/runtime_abi.rs`, one `RtOp { name, params,
//! result }` per declared op.
//!
//! The generated file is PLAIN DATA (no external dep), so it ships in the portable compiler; the
//! `wit-parser` dependency lives ONLY here in xtask (a dev-desk oracle). The backend builds a
//! program's per-program import section from this table (importing only the ops the program uses),
//! rather than pasting opaque envelope blobs — the operator's "structured info the compiler builds up
//! the wasm from, not massive opaque binaries." Re-run (`cargo xtask codegen`) after changing the WIT.

use crate::{Paths, build_component, content_address};
use proc_macro2::TokenStream;
use quote::quote;
use wit_parser::{Resolve, Type as WitType};
use xshell::Shell;

/// The core wasm value type a WIT primitive lowers to at the canonical-ABI core boundary. The runtime
/// interface uses only these four; a richer WIT type (string, list, record) is NOT lowerable as a bare
/// core scalar and is reported so the ABI cannot silently admit one.
#[derive(Clone, Copy)]
enum CoreTy {
    I32, // u32 / bool — a handle or a small scalar
    I64, // s64
    F64, // f64
}

impl CoreTy {
    /// Map a WIT type to its core valtype, or `None` if it is not a bare core scalar (e.g. `string`,
    /// `tuple` — the runtime's `str-*` and `vec-split` results, which the envelope does not lower).
    fn from_wit(t: WitType) -> Option<CoreTy> {
        match t {
            WitType::U32 | WitType::Bool => Some(CoreTy::I32),
            WitType::S64 => Some(CoreTy::I64),
            WitType::F64 => Some(CoreTy::F64),
            _ => None,
        }
    }

    /// The generated `CoreValType::<variant>` path as tokens (the backend's own enum, defined alongside
    /// the table), for splicing into a `quote!`.
    fn variant_tokens(self) -> TokenStream {
        match self {
            CoreTy::I32 => quote!(CoreValType::I32),
            CoreTy::I64 => quote!(CoreValType::I64),
            CoreTy::F64 => quote!(CoreValType::F64),
        }
    }
}

/// One runtime op resolved from the WIT: its name and core signature. A param or result the envelope
/// cannot lower (a non-scalar WIT type) marks the op UNLOWERABLE — it is still listed (so the table
/// mirrors the full interface) but flagged, and the backend never selects it.
struct Op {
    name: String,
    params: Vec<CoreTy>,
    result: Option<CoreTy>,
    /// `false` when a param or result is a non-core-scalar WIT type (`string`, `tuple`) — the op
    /// exists in the interface but the bare-core-signature envelope cannot import it.
    lowerable: bool,
}

/// Generate `runtime_abi.rs` from the runtime WIT. In `check` mode, regenerate in memory and compare
/// to the committed file WITHOUT writing — the STALENESS GATE: exit non-zero if the file is out of
/// date, so a forgotten regeneration fails `xtask check` rather than silently drifting from the WIT.
pub fn run(paths: &Paths, check: bool) {
    let wit = paths.seed.join("crates/cdz-runtime/wit/runtime.wit");
    let out = paths
        .seed
        .join("crates/rcdzc/src/backend/wasm/runtime_abi.rs");

    let ops = match resolve_ops(&wit) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("xtask codegen: {e}");
            std::process::exit(1);
        }
    };
    let iface = "cadenza:runtime/heap";
    // The runtime's CONTENT HASH — built + content-addressed the same way `xtask build` does, so the
    // generated table changes whenever the runtime BINARY changes (not only its WIT). This is what
    // makes staleness automatic across a runtime-code change: rebuild → new hash → `runtime_abi.rs`
    // differs → `codegen --check` fails until regenerated. (`build_component` runs `cargo component`;
    // `xtask check` already builds the runtime, so the cost is shared.)
    let runtime_hash = build_runtime_hash(paths);
    // Build the body as tokens (`render`), pretty-print + rustfmt it (`format_tokens`), then prepend the
    // `//!` module banner as text (a module doc is awkward as a token attribute). prettyplease-then-
    // rustfmt makes the committed file agree with BOTH `fmt --check` and `codegen --check`.
    let body = format_tokens(render(&ops, iface, &runtime_hash));
    let source = format!("{}{body}", module_banner());

    if check {
        // Compare, don't write. A mismatch (or a missing file) means the committed table is behind the
        // WIT — a hard failure with the fix spelled out, so no one has to REMEMBER to regenerate.
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current != source {
            eprintln!(
                "xtask codegen --check: {} is OUT OF DATE with the runtime WIT.\n  \
                 The runtime interface changed but the generated ABI table was not regenerated.\n  \
                 Fix: run `cargo xtask codegen` and commit {}.",
                out.display(),
                out.display()
            );
            std::process::exit(1);
        }
        println!("xtask codegen --check: {} is up to date.", out.display());
        return;
    }

    if let Err(e) = std::fs::write(&out, &source) {
        eprintln!("xtask codegen: writing {}: {e}", out.display());
        std::process::exit(1);
    }
    println!(
        "xtask codegen: wrote {} ({} ops, {} lowerable) from {}",
        out.display(),
        ops.len(),
        ops.iter().filter(|o| o.lowerable).count(),
        wit.display()
    );
}

/// Build the value-heap runtime component and return its content address (SHA-256 hex) — the SAME
/// derivation `xtask build` uses to key the runtime in the content-addressed store. Embedding this in
/// the generated table is what ties the compiler's recorded `REQUIRED_RUNTIME_HASH` to the ACTUAL
/// runtime bytes: a runtime-code change (even with an unchanged WIT) changes the hash, hence the
/// generated file, hence `codegen --check` fails until regenerated. Exits on a build failure (the
/// hash cannot be honestly recorded without the built artifact).
fn build_runtime_hash(paths: &Paths) -> String {
    let sh = Shell::new().expect("open a shell");
    let wasm = build_component(&sh, &paths.seed, "cdz-runtime", "cdz_runtime");
    let bytes = std::fs::read(&wasm)
        .unwrap_or_else(|e| panic!("read built runtime {}: {e}", wasm.display()));
    content_address(&bytes)
}

/// Format a generated `TokenStream` to the committed file's text: `prettyplease` FIRST (deterministic
/// pretty-print from the AST — quote! emits everything on effectively one line, and prettyplease breaks
/// it into readable structure without choking on length), THEN `rustfmt` (the SAME formatter the
/// `fmt --check` gate runs, so its output is stable under that gate — this is what makes `fmt --check`
/// and `codegen --check` agree by construction). rustfmt is best-effort: if it is not on PATH the
/// prettyplease output ships (valid Rust; only `fmt --check` might then re-wrap a line).
fn format_tokens(tokens: proc_macro2::TokenStream) -> String {
    let file = syn::parse2::<syn::File>(tokens)
        .unwrap_or_else(|e| panic!("xtask codegen: generated tokens did not parse (a bug): {e}"));
    let pretty = prettyplease::unparse(&file);
    rustfmt_stdin(&pretty).unwrap_or(pretty)
}

/// Run `src` through the `rustfmt` binary (stdin→stdout). `None` if rustfmt is unavailable or errors.
fn rustfmt_stdin(src: &str) -> Option<String> {
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
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Resolve every function of the runtime's `heap` interface from the WIT, IN DECLARATION ORDER — the
/// full op vocabulary as structured data. (The compiler resolves imports by name, so this order is
/// informational; it is the WIT's own order, kept stable for readability.)
fn resolve_ops(wit_path: &std::path::Path) -> Result<Vec<Op>, String> {
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_file(wit_path)
        .map_err(|e| format!("parse {}: {e}", wit_path.display()))?;
    let iface_id = resolve.packages[pkg]
        .interfaces
        .iter()
        .find(|(name, _)| name.as_str() == "heap")
        .map(|(_, id)| *id)
        .ok_or_else(|| "runtime WIT has no `heap` interface".to_string())?;
    let iface = &resolve.interfaces[iface_id];

    let mut ops = Vec::with_capacity(iface.functions.len());
    for f in iface.functions.values() {
        let mut lowerable = true;
        let mut params = Vec::with_capacity(f.params.len());
        for (_pname, ty) in &f.params {
            match CoreTy::from_wit(*ty) {
                Some(c) => params.push(c),
                None => lowerable = false,
            }
        }
        let result = match f.result {
            Some(ty) => match CoreTy::from_wit(ty) {
                Some(c) => Some(c),
                None => {
                    lowerable = false;
                    None
                }
            },
            None => None,
        };
        ops.push(Op {
            name: f.name.clone(),
            params,
            result,
            lowerable,
        });
    }
    // Sort by name — a deterministic, readable table order (the runtime resolves by name, so any
    // stable order is fine; alphabetical is the most diff-friendly).
    ops.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ops)
}

/// Build the generated Rust source as a `TokenStream` with `quote!` — the `CoreValType` enum, the
/// `RtOp` struct, the `RUNTIME_OPS` table, the typed `OPS` accessor (a named field per op → its
/// `&RUNTIME_OPS[i]`), the interface name, and the runtime content hash. Building tokens (not strings)
/// reads like the emitted Rust and needs no manual escaping; `format_tokens` pretty-prints it. Doc
/// comments are `#[doc = …]` attributes (which render as `///`). A leading `//!`-style module banner
/// can't be a token attribute cleanly, so it is prepended as text by the caller.
fn render(ops: &[Op], iface: &str, runtime_hash: &str) -> TokenStream {
    // The RUNTIME_OPS rows.
    let rows = ops.iter().map(|op| {
        let name = &op.name;
        let params = op.params.iter().map(|c| c.variant_tokens());
        let result = match op.result {
            Some(c) => {
                let v = c.variant_tokens();
                quote!(Some(#v))
            }
            None => quote!(None),
        };
        let lowerable = op.lowerable;
        quote! {
            RtOp { name: #name, params: &[#(#params),*], result: #result, lowerable: #lowerable },
        }
    });

    // The typed `OPS` accessor: `field: &RUNTIME_OPS[i]`, and the struct field decls.
    let field_idents: Vec<syn::Ident> = ops.iter().map(|op| field_ident(&op.name)).collect();
    let field_decls = field_idents.iter().map(|f| quote!(pub #f: &'static RtOp,));
    let field_inits = field_idents.iter().enumerate().map(|(i, f)| {
        let idx = proc_macro2::Literal::usize_unsuffixed(i);
        quote!(#f: &RUNTIME_OPS[#idx],)
    });

    quote! {
        #[doc = " A core wasm value type in a runtime op's signature (the canonical-ABI core boundary"]
        #[doc = " uses only these three). Maps to the backend's `ValType`."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum CoreValType { I32, I64, F64 }

        #[doc = " One value-heap runtime op the compiler may import: its WIT name and core signature. A"]
        #[doc = " non-`lowerable` op carries a non-scalar WIT type (string/tuple) the bare-core envelope"]
        #[doc = " cannot import; it is listed for completeness but never selected."]
        pub struct RtOp {
            pub name: &'static str,
            pub params: &'static [CoreValType],
            pub result: Option<CoreValType>,
            pub lowerable: bool,
        }

        #[doc = " The runtime interface a program imports — the fixed ABI identity, prefix of the"]
        #[doc = " versioned `<iface>@…+<hash>` import name (`component-abi.md` §The Value-Heap Runtime"]
        #[doc = " Crosses By A Well-Known Import). The content-address suffix is `REQUIRED_RUNTIME_HASH`."]
        pub const RUNTIME_IFACE: &str = #iface;

        #[doc = " The SHA-256 content address of the value-heap runtime component this ABI was generated"]
        #[doc = " against — the runtime a program built with this compiler requires. Regenerated from the"]
        #[doc = " built runtime bytes, so it tracks a runtime-code change automatically."]
        pub const REQUIRED_RUNTIME_HASH: &str = #runtime_hash;

        #[doc = " Every op the runtime `heap` interface declares, as structured signature data (sorted)."]
        pub const RUNTIME_OPS: &[RtOp] = &[ #(#rows)* ];

        #[doc = " Typed access to each runtime op by name — `OPS.arr_get` borrows `&RUNTIME_OPS[i]`. A"]
        #[doc = " field per op (kebab-case WIT name → snake_case field), so a rename in the WIT is a"]
        #[doc = " compile error at every use rather than a silent stringly-typed miss."]
        pub struct RuntimeOps { #(#field_decls)* }

        #[doc = " The one `RuntimeOps` value — each field borrows its entry in `RUNTIME_OPS` by offset."]
        pub const OPS: RuntimeOps = RuntimeOps { #(#field_inits)* };
    }
}

/// The Rust field identifier for a runtime op — the WIT's kebab-case name with `-` → `_` (a valid,
/// readable snake-case identifier: `arr-get` → `arr_get`, `map-iter-next` → `map_iter_next`).
fn field_ident(op_name: &str) -> syn::Ident {
    syn::Ident::new(&op_name.replace('-', "_"), proc_macro2::Span::call_site())
}

/// The `//!` module banner prepended to the generated file — the "do-not-edit / regenerate" notice.
/// A module doc is awkward to carry as a token attribute, so it is plain leading text (rustfmt leaves
/// a `//!` block alone).
fn module_banner() -> String {
    "//! @generated by `cargo xtask codegen` from cdz-runtime/wit/runtime.wit — DO NOT hand-edit.\n\
     //!\n\
     //! The value-heap runtime's ABI as STRUCTURED data: each op's name + core signature, a typed\n\
     //! `OPS` accessor, the interface name, and the runtime's content hash. The wasm backend builds a\n\
     //! program's per-program import section from this (importing only the ops it uses) — no opaque\n\
     //! envelope blob, no baked-in op index (imports resolve by name). Regenerate with `cargo xtask\n\
     //! codegen`; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails if it drifts from\n\
     //! the runtime WIT or the built runtime's bytes. Plain data — no dependency, so it ships.\n\n"
        .to_string()
}

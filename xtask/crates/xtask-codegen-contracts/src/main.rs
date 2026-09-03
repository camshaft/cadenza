//! `xtask-codegen-contracts` — project each `cdz-platform/contracts/{kernel,userspace}/*.cdz` contract
//! into its generated Rust schema module (`<name>.rs`) plus the `contracts/mod.rs` listing, at BUILD time.
//!
//! Carved out of `xtask/src/codegen.rs`'s `generate_contracts` (v-xtask-decompose, the operator
//! codegen→build-time-nix directive). This is the RENDER path only: read each contract's `.cdz`, get its
//! canonical AST (`cdz convert --to binary`), execute its `descriptor()` for the contract identity
//! (`cdz compile … --entry <name>` + `cdz run --format binary-ast`, decoded via
//! `cdz_contract::identity_from_descriptor`), and emit the schema as `cadenza_ast` builder calls. It OMITS
//! the `cdz test` @test-validation step that `generate_contracts` also runs — that stays a
//! `cargo xtask codegen --check` concern; this bin/derivation only EMITS.
//!
//! The render logic is copied VERBATIM from codegen.rs so the emitted `.rs` are byte-identical to the
//! committed ones (the acceptance test for the build-phase-overlay flip). The only adaptations are the cdz
//! drivers (run the `CDZ_SEED_BIN_DIR` binary instead of `cargo run -p cdz`) and the `main` entry (a
//! render-only loop; no mtime short-circuit, no `--check`, no store-seeding).
//!
//! Usage: `xtask-codegen-contracts [<out-dir>]` — `out-dir` defaults to the committed
//! `cdz-platform/src/contracts` (for local parity); the `cdzPlatformContracts` derivation passes `$out`.
//! Repo root from `CDZ_REPO_ROOT` (else cwd); `cdz` from `CDZ_SEED_BIN_DIR` (else `<repo>/target/debug`).

use cadenza_ast::ast::{Arenas, Struct, StructId};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::path::{Path, PathBuf};
use xtask_codegen_support::format_tokens;

fn main() {
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let seed = repo.join("implementation/seed");
    let bin_dir = std::env::var_os("CDZ_SEED_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/debug"));
    let cdz = bin_dir.join("cdz");

    let contracts_dir = seed.join("crates/cdz-platform/contracts");
    // The generated sources' destination. The derivation passes its `$out`; a bare local run defaults to the
    // committed tree so `diff` against the committed `.rs` is the byte-identical acceptance test.
    let out_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| seed.join("crates/cdz-platform/src/contracts"));

    // The DIRECTORY is the classification (operator ruling, no hardcoded list): `contracts/kernel/` emit a
    // Rust binding; `contracts/userspace/` are CADENZA-ONLY (validated elsewhere, no Rust binding + no
    // `contracts/mod.rs` entry). A source carries `cadenza_only` = it lives under `userspace/`.
    let read_cdz = |dir: &Path| -> Vec<PathBuf> {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "cdz"))
                    .collect();
                v.sort();
                v
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                eprintln!("xtask codegen: read contracts dir {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
    };
    let mut sources: Vec<(PathBuf, bool)> = read_cdz(&contracts_dir.join("kernel"))
        .into_iter()
        .map(|p| (p, false))
        .collect();
    sources.extend(
        read_cdz(&contracts_dir.join("userspace"))
            .into_iter()
            .map(|p| (p, true)),
    );
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("xtask codegen: create {}: {e}", out_dir.display());
        std::process::exit(1);
    }

    // A contract may `import { contract-id } from "contract-id"`; per-file compile can't resolve that import
    // (the lib lives under guests/, not beside the contract), so stage the contract sources ALONGSIDE a copy
    // of `guests/contract-id.cdz` under the clean name, where cdz's same-directory module resolution finds it.
    let stage = repo.join("target/codegen-contract-stage");
    let lib = seed.join("crates/cdz-platform/guests/contract-id.cdz");
    let _ = std::fs::remove_dir_all(&stage);
    if let Err(e) = std::fs::create_dir_all(&stage) {
        eprintln!("xtask codegen: create staging dir {}: {e}", stage.display());
        std::process::exit(1);
    }
    let stage_copy = |from: &Path, to: PathBuf| {
        if let Err(e) = std::fs::copy(from, &to) {
            eprintln!(
                "xtask codegen: stage {} -> {}: {e}",
                from.display(),
                to.display()
            );
            std::process::exit(1);
        }
    };
    stage_copy(&lib, stage.join("contract-id.cdz"));
    for (src, _) in &sources {
        let file = src.file_name().expect("a contract file name");
        stage_copy(src, stage.join(file));
    }

    let mut names: Vec<String> = Vec::with_capacity(sources.len());
    for (src, cadenza_only) in &sources {
        let name = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| {
                eprintln!(
                    "xtask codegen: contract file has no usable name: {}",
                    src.display()
                );
                std::process::exit(1);
            })
            .to_string();
        // Only a KERNEL contract emits Rust + is declared in `contracts/mod.rs`.
        if !cadenza_only {
            names.push(name.clone());
        }
        // Userspace: no Rust binding — a Cadenza guest consumes it via self-reflection, the host never does.
        if *cadenza_only {
            continue;
        }

        let src_str = src.to_str().expect("a UTF-8 contract path");
        let staged = stage.join(src.file_name().expect("a contract file name"));
        let staged_str = staged.to_str().expect("a UTF-8 staged contract path");

        let ast_bytes = run_cdz_capture(
            &cdz,
            &["convert", src_str, "--to", "binary"],
            &format!("read the AST of {}", src.display()),
        );
        let arenas = cadenza_ast::codec::decode(&ast_bytes).unwrap_or_else(|| {
            eprintln!(
                "xtask codegen: `cdz convert {} --to binary` did not produce a decodable AST",
                src.display()
            );
            std::process::exit(1);
        });

        let decls = type_decls(&arenas);
        if decls.is_empty() {
            eprintln!(
                "xtask codegen: {} declares no `type` — a contract schema needs at least one type",
                src.display()
            );
            std::process::exit(1);
        }

        let identity = contract_identity(&cdz, &stage, staged_str, &name);
        let body = format_tokens(render_schema(&arenas, &decls, &name, identity.as_ref()));
        let source = format!("{}{body}", contract_banner(&name));
        let out = out_dir.join(format!("{name}.rs"));
        write_generated(&out, &source);
        println!(
            "xtask codegen: wrote {} ({} type declarations, from {})",
            out.display(),
            decls.len(),
            src.display()
        );
    }

    // The module file listing every generated contract, projected from the directory.
    let mod_rs = out_dir.join("mod.rs");
    let mod_src = format!(
        "{}{}",
        contracts_mod_banner(),
        format_tokens(render_contracts_mod(&names))
    );
    write_generated(&mod_rs, &mod_src);
    println!(
        "xtask codegen: wrote {} ({} contract module(s))",
        mod_rs.display(),
        names.len()
    );
}

/// Write a generated file (creating parent dirs). The build-time counterpart of codegen's `emit_or_check`
/// write branch — the derivation always regenerates, so there is no `--check` compare here.
fn write_generated(out: &Path, source: &str) {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Err(e) = std::fs::write(out, source) {
        eprintln!("xtask codegen: writing {}: {e}", out.display());
        std::process::exit(1);
    }
}

/// Run the `cdz` binary with `<args>`, inheriting stdio; exit non-zero if it fails. `what` names the step.
fn run_cdz(cdz: &Path, args: &[&str], what: &str) {
    let status = std::process::Command::new(cdz)
        .args(args)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "xtask codegen: could not run `cdz {}` ({what}): {e}",
                args.join(" ")
            )
        });
    if !status.success() {
        eprintln!("xtask codegen: `cdz {}` failed ({what})", args.join(" "));
        std::process::exit(1);
    }
}

/// Run the `cdz` binary and return its stdout bytes (stderr inherited); exit non-zero if it fails. Used to
/// capture `cdz convert --to binary` (the canonical AST) and `cdz run --format binary-ast` (the descriptor).
fn run_cdz_capture(cdz: &Path, args: &[&str], what: &str) -> Vec<u8> {
    let out = std::process::Command::new(cdz)
        .args(args)
        .stderr(std::process::Stdio::inherit())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "xtask codegen: could not run `cdz {}` ({what}): {e}",
                args.join(" ")
            )
        });
    if !out.status.success() {
        eprintln!("xtask codegen: `cdz {}` failed ({what})", args.join(" "));
        std::process::exit(1);
    }
    out.stdout
}

/// The contract's `type` declaration occurrences, in source order. A bare `.cdz` source canonicalizes to
/// a root `(do <form>…)`, and a source comment wraps the form after it as `(comment <text>… <form>)`, so a
/// declaration can sit under a comment chain rather than directly under the `do`. Walk the `do`'s children,
/// unwrapping any comment chain to the form it carries, and collect the `type`-headed ones.
fn type_decls(arenas: &Arenas) -> Vec<StructId> {
    let mut out = Vec::new();
    let Struct::List(items) = arenas.get(arenas.root) else {
        return out;
    };
    for &child in items.iter().skip(1) {
        let form = unwrap_comment(arenas, child);
        if arenas.head_name(form) == Some("type") {
            out.push(form);
        }
    }
    out
}

/// Unwrap a comment chain `(comment <text>… <form>)` to the form it carries (the last child), following
/// nested comments to the innermost wrapped form. A non-comment id is returned unchanged.
fn unwrap_comment(arenas: &Arenas, id: StructId) -> StructId {
    let mut id = id;
    while arenas.head_name(id) == Some("comment") {
        match arenas.get(id) {
            Struct::List(items) => match items.last() {
                Some(&last) if last != id => id = last,
                _ => break,
            },
            Struct::Atom(_) => break,
        }
    }
    id
}

/// Render a contract's generated schema module: the `schema(b)` function that reconstructs the type
/// declarations for `Contract::new`, plus — for every declared constructor — a value BUILDER and a
/// matching READER.
fn render_schema(
    arenas: &Arenas,
    decls: &[StructId],
    name: &str,
    identity: Option<&(String, String, String)>,
) -> TokenStream {
    let mut stmts: Vec<TokenStream> = Vec::new();
    let mut counter = 0usize;
    let decl_idents: Vec<syn::Ident> = decls
        .iter()
        .map(|&d| emit_node(arenas, d, &mut stmts, &mut counter))
        .collect();

    let bindings = decls.iter().flat_map(|&d| emit_value_bindings(arenas, d));

    // When the source declares its identity (via its `descriptor()`), generate the `contract()` constructor
    // from it — so a contract's name and its input/output type references live ONLY in the `.cdz`.
    let contract_fn = match identity {
        Some((contract_name, input, output)) => quote! {
            #[doc = " The contract this module declares — built from its `@!contract` / `@!input` /"]
            #[doc = " `@!output` pragmas and its schema. The one place the contract's name and input/output"]
            #[doc = " type references live is the `.cdz` source; `*_contract()` calls this."]
            pub fn contract() -> crate::Contract {
                crate::Contract::new(crate::Str::from_static(#contract_name), schema, #input, #output)
            }
        },
        None => quote! {},
    };

    let doc = format!(
        " The `{name}` contract's schema: its named Cadenza type declarations, in source order, ready"
    );
    quote! {
        // A generated bindings surface: a builder + reader for every constructor. Not every consumer uses
        // every one (e.g. the output type's constructors until the kernel produces that outcome), so the
        // unused-code lint is allowed for the whole generated module rather than per item.
        #![allow(dead_code)]

        // A contract every one of whose constructors is a single-constructor sum ELIDED to its bare scalar
        // payload (e.g. `timer`, whose `Envelope`/`Event` are both `| C(UInt64)`) never names `v` — the
        // builders return the payload directly — so this import is genuinely unused there. Allow it rather
        // than emit the import conditionally.
        #[allow(unused_imports)]
        use crate::contract_value as v;
        use cadenza_ast::ast::{Arenas, Builder, StructId};

        #[doc = #doc]
        #[doc = " to hand to `Contract::new`. Generated from the contract's Cadenza source, which `cdz`"]
        #[doc = " typechecks and runs the conformance tests of at codegen time, so this is provably a"]
        #[doc = " valid Cadenza schema."]
        pub fn schema(b: &mut Builder) -> Vec<StructId> {
            #(#stmts)*
            vec![#(#decl_idents),*]
        }

        #contract_fn

        #(#bindings)*
    }
}

/// Read a contract's identity — its NAME and its INPUT / OUTPUT type-name references — by COMPILING and
/// EXECUTING the contract's `descriptor()` and reading the folded descriptor record. The staged contract is
/// compiled together with the staged `contract-id` lib into a component exporting `descriptor`, run with
/// `cdz run --format binary-ast` (the descriptor record as canonical binary AST), and decoded;
/// `cdz_contract::identity_from_descriptor` reads the name + input/output type names out.
fn contract_identity(
    cdz: &Path,
    stage: &Path,
    staged_str: &str,
    name: &str,
) -> Option<(String, String, String)> {
    let wasm = stage.join(format!("{name}.wasm"));
    let wasm_str = wasm.to_str().expect("a UTF-8 staged wasm path");
    let lib = stage.join("contract-id.cdz");
    let lib_str = lib.to_str().expect("a UTF-8 staged lib path");
    run_cdz(
        cdz,
        &[
            "compile", staged_str, lib_str, "--entry", name, "-o", wasm_str,
        ],
        &format!("compile {name} to execute its descriptor()"),
    );
    let doc = run_cdz_capture(
        cdz,
        &["run", wasm_str, "--format", "binary-ast"],
        &format!("execute the descriptor() of {name}"),
    );
    let value = cadenza_ast::codec::decode(&doc)?;
    cdz_contract::identity_from_descriptor(&value)
}

/// A declared constructor's shape, as introspected from a `(type T …)` declaration.
enum Ctor {
    /// A variant with no payload — `C`.
    Nullary,
    /// A variant carrying a single non-record payload — `C(SomeType)`.
    Single,
    /// A variant carrying a record — `C(Record(f0: T0, …))` — with its field names in declared order.
    Record(Vec<String>),
}

/// Emit a value builder + reader for every constructor of the type declaration `decl` (a `(type T …)`).
fn emit_value_bindings(arenas: &Arenas, decl: StructId) -> Vec<TokenStream> {
    let Struct::List(items) = arenas.get(decl) else {
        return Vec::new();
    };
    let Some(ty) = items.get(1).and_then(|&n| arenas.as_name(n)) else {
        return Vec::new();
    };
    let variants: Vec<StructId> = items.iter().skip(2).copied().collect();
    let single = variants.len() == 1;
    variants
        .iter()
        .filter_map(|&var| emit_ctor(arenas, ty, var, single))
        .collect()
}

/// Emit the builder + reader for one variant occurrence `var` of type `ty`. `None` if the variant is not a
/// name or `(Ctor …)` form (a parse invariant changed).
fn emit_ctor(arenas: &Arenas, ty: &str, var: StructId, single: bool) -> Option<TokenStream> {
    let (ctor, shape) = if let Some(ctor) = arenas.as_name(var) {
        (ctor.to_string(), Ctor::Nullary)
    } else {
        let ctor = arenas.head_name(var)?.to_string();
        let Struct::List(items) = arenas.get(var) else {
            return None;
        };
        match items.get(1) {
            None => (ctor, Ctor::Nullary),
            Some(&payload) if arenas.head_name(payload) == Some("Record") => {
                (ctor, Ctor::Record(record_field_names(arenas, payload)))
            }
            Some(_) => (ctor, Ctor::Single),
        }
    };

    let build = syn::Ident::new(
        &format!("{}_{}", to_snake(ty), to_snake(&ctor)),
        Span::call_site(),
    );
    let doc_build = format!(" Build a canonical `{ty}.{ctor}` value.");
    Some(match shape {
        Ctor::Nullary => {
            let is = syn::Ident::new(&format!("is_{build}"), Span::call_site());
            let doc_read = format!(" Whether `id` is a `{ty}.{ctor}` value.");
            if single {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder) -> StructId {
                        v::unit(b)
                    }
                    #[doc = #doc_read]
                    pub fn #is(arenas: &Arenas, id: StructId) -> bool {
                        v::is_unit(arenas, id)
                    }
                }
            } else {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder) -> StructId {
                        v::qctor(b, #ty, #ctor, vec![])
                    }
                    #[doc = #doc_read]
                    pub fn #is(arenas: &Arenas, id: StructId) -> bool {
                        v::as_qctor(arenas, id, #ty, #ctor).is_some_and(|t| t.is_empty())
                    }
                }
            }
        }
        Ctor::Single => {
            let as_ = syn::Ident::new(&format!("as_{build}"), Span::call_site());
            let doc_read = format!(" Read the payload of a `{ty}.{ctor}` value, or `None`.");
            if single {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(_b: &mut Builder, x: StructId) -> StructId {
                        x
                    }
                    #[doc = #doc_read]
                    pub fn #as_(_arenas: &Arenas, id: StructId) -> Option<StructId> {
                        Some(id)
                    }
                }
            } else {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder, x: StructId) -> StructId {
                        v::qctor(b, #ty, #ctor, vec![x])
                    }
                    #[doc = #doc_read]
                    pub fn #as_(arenas: &Arenas, id: StructId) -> Option<StructId> {
                        let t = v::as_qctor(arenas, id, #ty, #ctor)?;
                        let [x] = <[StructId; 1]>::try_from(t).ok()?;
                        Some(x)
                    }
                }
            }
        }
        Ctor::Record(fields) => {
            let rec_struct = syn::Ident::new(
                &format!("{}{}", to_pascal(ty), to_pascal(&ctor)),
                Span::call_site(),
            );
            let field_idents: Vec<syn::Ident> = fields
                .iter()
                .map(|f| syn::Ident::new(&to_snake(f), Span::call_site()))
                .collect();
            let field_names: Vec<&str> = fields.iter().map(String::as_str).collect();
            let as_ = syn::Ident::new(&format!("as_{build}"), Span::call_site());
            let struct_doc =
                format!(" The fields of a `{ty}.{ctor}` value — each a built value occurrence.");
            let doc_read = format!(" Read a `{ty}.{ctor}` value's fields by name, or `None`.");
            let (build_body, read_bind) = if single {
                (
                    quote! { v::record(b, vec![#((#field_names, fields.#field_idents)),*]) },
                    quote! { let rec = id; },
                )
            } else {
                (
                    quote! {
                        let rec = v::record(b, vec![#((#field_names, fields.#field_idents)),*]);
                        v::qctor(b, #ty, #ctor, vec![rec])
                    },
                    quote! {
                        let t = v::as_qctor(arenas, id, #ty, #ctor)?;
                        let [rec] = <[StructId; 1]>::try_from(t).ok()?;
                    },
                )
            };
            quote! {
                #[doc = #struct_doc]
                pub struct #rec_struct {
                    #(pub #field_idents: StructId,)*
                }
                #[doc = #doc_build]
                pub fn #build(b: &mut Builder, fields: #rec_struct) -> StructId {
                    #build_body
                }
                #[doc = #doc_read]
                pub fn #as_(arenas: &Arenas, id: StructId) -> Option<#rec_struct> {
                    #read_bind
                    Some(#rec_struct {
                        #(#field_idents: v::record_field(arenas, rec, #field_names)?,)*
                    })
                }
            }
        }
    })
}

/// The field names of a record type `(Record (: f0 T0) (: f1 T1) …)`, in declared order.
fn record_field_names(arenas: &Arenas, record: StructId) -> Vec<String> {
    let Struct::List(items) = arenas.get(record) else {
        return Vec::new();
    };
    items
        .iter()
        .skip(1) // the `Record` head
        .filter_map(|&f| {
            let kv = arenas.as_form(f, ":")?; // `(: <name> <type>)`
            arenas.as_name(*kv.first()?).map(str::to_string)
        })
        .collect()
}

/// A Cadenza name to a PascalCase Rust type identifier: drop `-`/`_` separators and capitalize the first
/// letter of each segment (`deliver-envelope` → `DeliverEnvelope`).
fn to_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for c in s.chars() {
        if c == '-' || c == '_' {
            cap = true;
        } else if cap {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A Cadenza name to a snake_case Rust identifier: `-` → `_`, and an underscore before each interior
/// capital (`MissingHandler` → `missing_handler`, `deliver-envelope` → `deliver_envelope`).
fn to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c == '-' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            if i != 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Emit the builder statements that reconstruct the node at `id`, returning the identifier bound to it.
/// Post-order: children first (so their identifiers exist), then the parent's `b.list`.
fn emit_node(
    arenas: &Arenas,
    id: StructId,
    stmts: &mut Vec<TokenStream>,
    counter: &mut usize,
) -> syn::Ident {
    if let Some(name) = arenas.as_name(id) {
        let ident = fresh_ident(counter);
        let name = name.to_string();
        stmts.push(quote!(let #ident = b.name(#name);));
        return ident;
    }
    match arenas.get(id) {
        Struct::List(items) => {
            let items = items.clone();
            let children: Vec<syn::Ident> = items
                .iter()
                .map(|&c| emit_node(arenas, c, stmts, counter))
                .collect();
            let ident = fresh_ident(counter);
            stmts.push(quote!(let #ident = b.list(vec![#(#children),*]);));
            ident
        }
        Struct::Atom(_) => panic!(
            "xtask codegen: a contract `type` declaration holds a non-name atom — a type declaration is \
             built from names and lists only (a compiler/parse invariant changed)"
        ),
    }
}

/// Render `contracts/mod.rs`: one `pub mod <name>;` per contract, in sorted order.
fn render_contracts_mod(names: &[String]) -> TokenStream {
    let mods = names.iter().map(|n| {
        let ident = syn::Ident::new(&n.replace('-', "_"), Span::call_site());
        quote!(pub mod #ident;)
    });
    quote! { #(#mods)* }
}

/// A fresh temporary identifier `v0`, `v1`, … for the builder statements.
fn fresh_ident(counter: &mut usize) -> syn::Ident {
    let ident = syn::Ident::new(&format!("v{counter}"), Span::call_site());
    *counter += 1;
    ident
}

/// The `//!` banner prepended to each generated `<name>.rs`.
fn contract_banner(name: &str) -> String {
    format!(
        "//! @generated by `cargo xtask codegen` from cdz-platform/contracts/{name}.cdz — DO NOT hand-edit.\n\
         //!\n\
         //! The `{name}` contract's schema as Cadenza-AST builder calls: `schema(b)` reconstructs the\n\
         //! contract's named type declarations, which the platform hands to `Contract::new`. The source is\n\
         //! real Cadenza that `cargo xtask codegen` validates + runs the conformance tests of (via the\n\
         //! `cdz` binary) before generating this file, so the schema is provably valid Cadenza and a\n\
         //! marshalled value type-ascribes against it. Edit the schema in `{name}.cdz`, then regenerate\n\
         //! with `cargo xtask codegen`; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails\n\
         //! if this file is stale. Plain builder calls — no dependency on the compiler, so it ships in\n\
         //! `cdz-platform`.\n\n"
    )
}

/// The `//!` banner prepended to the generated `contracts/mod.rs` — the module listing every contract.
fn contracts_mod_banner() -> String {
    "//! @generated by `cargo xtask codegen` from the cdz-platform/contracts/ directory — DO NOT hand-edit.\n\
     //!\n\
     //! One `pub mod` per built-in contract, projected from the contract sources so a new\n\
     //! `contracts/<name>.cdz` wires itself in on the next `cargo xtask codegen`.\n\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{render_contracts_mod, to_pascal, to_snake};

    /// The Cadenza-name → PascalCase mapping used for generated type/struct identifiers. A regression
    /// here silently mis-names a generated struct, so pin the separator handling (both `-` and `_`) and
    /// idempotence on an already-Pascal name.
    #[test]
    fn to_pascal_maps_separators_and_is_idempotent() {
        assert_eq!(to_pascal("deliver-envelope"), "DeliverEnvelope");
        assert_eq!(to_pascal("missing_handler"), "MissingHandler");
        assert_eq!(to_pascal("event"), "Event");
        assert_eq!(to_pascal("a-b-c"), "ABC");
        // already-Pascal input round-trips unchanged.
        assert_eq!(to_pascal("MissingHandler"), "MissingHandler");
    }

    /// The Cadenza-name → snake_case mapping used for generated `build_*`/`is_*`/`as_*` fn identifiers:
    /// `-` → `_`, and an underscore before each interior capital (no leading underscore). A regression
    /// here silently mis-names a generated accessor.
    #[test]
    fn to_snake_lowercases_and_splits_interior_capitals() {
        assert_eq!(to_snake("MissingHandler"), "missing_handler");
        assert_eq!(to_snake("deliver-envelope"), "deliver_envelope");
        assert_eq!(to_snake("event"), "event");
        // a leading capital gets no leading underscore.
        assert_eq!(to_snake("Envelope"), "envelope");
    }

    /// `render_contracts_mod` emits one `pub mod <name>;` per contract with `-` folded to `_` (a Rust
    /// module ident can't hold a hyphen). Preserves the input order it is handed (the caller sorts).
    #[test]
    fn render_contracts_mod_emits_valid_module_idents() {
        let rust = render_contracts_mod(&["deliver-envelope".to_string(), "state".to_string()])
            .to_string();
        assert!(
            rust.contains("pub mod deliver_envelope"),
            "hyphenated contract name not folded to a valid module ident: {rust}"
        );
        assert!(
            rust.contains("pub mod state"),
            "missing plain contract mod: {rust}"
        );
        assert!(
            !rust.contains('-'),
            "a raw hyphen leaked into a module ident: {rust}"
        );
    }
}

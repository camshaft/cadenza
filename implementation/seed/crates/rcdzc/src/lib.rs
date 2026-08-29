//! `rcdzc` — the reference Cadenza → WebAssembly-component compiler, rebuilt to the reference
//! architecture (`spec/architecture/*.md`). See `Cargo.toml` for the two shaping directives
//! (copy-don't-depend; Cadenza-in-Rust style). This is the Stage-0 skeleton.

// Name the `alloc` sysroot crate so the shared codec-core modules (`ast`/`codec`/`leb128`) can import
// `alloc::{vec::Vec, rc::Rc}` + `alloc::collections::BTreeMap` — imports valid in BOTH this std crate
// AND the `#![no_std]` `cdz-runtime` that `include!`s these same source files (the runtime
// `ast-encode`/`ast-decode` ops reuse ONE serializer for byte-identity). Benign in std (alloc types ARE
// std's — `alloc::vec::Vec` == `std::vec::Vec`); required for the no_std include. `BTreeMap` (not
// `HashMap`) so leaf dedup needs no external hasher under `no_std`; the map is lookup-only and ids are
// insertion-ordered, so the encoded bytes are unchanged.
extern crate alloc;

// The AST value model + canonical binary codec: the SINGLE `cadenza-ast` crate, re-exported so the rest
// of the compiler keeps addressing `crate::{ast,codec,leb128}` unchanged. Formerly copied-in verbatim
// (`ast.rs`/`codec.rs`/`leb128.rs`); consolidated onto the one shared crate (operator directive: one
// source of truth, no diverging copies). `default-features = false` = the `no_std`+alloc CORE (no
// num-bigint / unicode-normalization / canon), keeping the compiler dependency-light for the Cadenza
// self-host port — cadenza-ast is the one sanctioned dependency exception.
pub use cadenza_ast::{ast, codec, leb128};

// DRIFT-GUARD (ast-consolidation): `crate::{ast,codec,leb128}` MUST stay RE-EXPORTS of the single
// `cadenza-ast` crate, never re-forked local copies. These identity-function `const`s compile ONLY while
// the re-exported items are the SAME types/fns as `cadenza_ast`'s; if a future change reverts the
// re-export to a `pub mod ast { … }` fork (reintroducing the divergence this consolidation removed), the
// types stop unifying and the BUILD FAILS here fleet-wide — turning a silent re-fork into a hard error.
// Zero runtime cost (never called). cheap structural insurance for the one-source-of-truth invariant.
const _: fn(cadenza_ast::ast::Leaf) -> crate::ast::Leaf = |x| x;
const _: fn(cadenza_ast::ast::Arenas) -> crate::ast::Arenas = |x| x;
const _: fn(&crate::ast::Arenas) -> Vec<u8> = crate::codec::encode;
const _: fn(&[u8]) -> Option<crate::ast::Arenas> = crate::codec::decode;

// The columns substrate: index-typed arenas + columns, and the diagnostic taxonomy.
pub mod arena;
pub mod diag;

// A fast non-cryptographic hasher (FxHash) + `FxHashMap`/`FxHashSet` for the compiler's internal
// integer/short-string-keyed index maps — SipHash is pure overhead on keys that are always ours.
pub mod fxhash;

// The solved-type universe (target-neutral).
pub mod ty;

// The Hindley-Milner core: substitution, unification (occurs-check), scheme instantiation. The
// machinery the one generic application rule uses; pure over `Ty`.
pub mod unify;

// The compile-time evaluator: the ONE application reduction through `Meta.apply` — projecting a head
// value's meta channel, applying a native primitive (an arithmetic op or a type constructor), and
// building a value (e.g. `(Int 64)` → a width-64 module) by appending arena nodes.
pub mod eval;

// The per-node rung forms (each an entry of a column keyed by AST `StructId`).
pub mod core;
// Backend-agnostic Core-IR analysis primitives shared by the wasm Lir-level LICM/CSE realization and
// the backend-independent Core optimization passes (a pure move out of `backend/wasm/select.rs`).
pub(crate) mod core_analysis;
pub mod resolved;

// The prelude — the one map of built-in bindings, installed as ordinary AST records at load. A
// built-in module is just a record; nothing is privileged by name or by shape.
pub mod prelude;

// Sum-type synthesis — a `(type NAME variant…)` declaration realized as ordinary records (the sum is a
// record whose fields are its variants), the program-driven twin of `prelude`. Reuses the same member
// access / application / `(meta t)` machinery, so nothing about sums is special-cased.
pub mod sums;
// `(effect …)` synthesis — an effect declaration realized as an ordinary record (fields = operation
// values), the effect analogue of `sums`. So `E.op` is member access and a perform is application.
pub mod effects;
// `(module …)` synthesis — a nested module declaration realized as an ordinary record (fields = exported
// defs), the module analogue of `sums`/`effects`. So `(. m field)` is member access, nothing privileged.
pub mod modules;
// Compiler-directed generators for property tests over collection types (F1): synthesize a nullary
// `Test.gen-int`-driven wrapper for a `@test` whose parameter is a `(List <Int>)`, so `cdz test` can
// property-test over a list. Runs at load before `strip_annotations`; all synthesized nodes are ordinary
// AST (`DESIGN-property-test-collection-generators-rcdzc.md`).
pub mod proptest_gen;
// DATA-TYPE INVARIANT ESTABLISH (Part 1): synthesize a typed `__invariant_check_<T>` def per `@invariant`
// type (auto-unwrapping a single-payload newtype so a bare scalar predicate type-checks), so the invariant
// predicate is resolved + type-checked + is the callee for the construct-site establish check (design §10).
pub mod invariant_establish;
// `(quote …)` reification — a quote rewritten to the `Ast` constructor application that BUILDS its value
// (`(quote 42)` -> `(Ast.Int 42)`), so a quote result and a hand-built `Ast.*` value are one thing.
pub mod quote;
// `(eval AST)` desugar — the INVERSE of quote reification: reconstruct the source form an `Ast` value
// denotes and splice it in, so `(eval (quote (+ 1 2)))` folds to `3` through the ordinary path.
pub mod eval_ast;
// Verification-annotation ENFORCEMENT (Inc-b (D), test-tier) — rewrite a PLAIN `@requires` so the def body
// checks the precondition at body-entry and traps on violation. Runs before `proptest_gen`/`strip_annotations`.
pub mod verify_enforce;
// `@param` sidecar — scan every `@param(widget: …) name : Type` site and GENERATE a `Param` effect with
// one typed accessor op per param (the runtime-parameter host-effect codegen; v-effects binds it, v-syntax
// parses the annotation). Runs before the top-level scan so the generated effect is picked up as a decl.
pub mod param_sidecar;
// Tagged-template expansion — `(tagged-template <tag> (chunks …) (holes …))` (the reader's form for
// `tag"…{expr}…"`) rewritten to the binding-dispatched application `(<tag> (list …) (list …))`, which the
// one-tier evaluator reduces and splices — an embedded DSL grows at the AST level via an ordinary function.
pub mod tagged_template;

// Normalize a type-suffixed numeric literal leaf (`100N`/`0.5R`) into the `(: <body> BigInt|Rational)`
// annotation a suffix denotes — restoring rcdzc's "the compiler never sees a `Suffixed` leaf" invariant
// after the ast-consolidation made rcdzc consume cadenza-ast's codec (which preserves the leaf kind).
pub mod suffixed;

// The query engine: the single `Db` is PURE DATA (the AST + the columns); each query is a free
// function in its own module over `&mut Db`, and each module owns exactly one column's fills —
// `resolve` fills `resolved`, `infer` fills `types`, `lower` fills `core`. A query reads another
// module's fact by calling that module's producer (which fills it lazily), never a raw column.
pub mod accum;
pub mod binding_params;
pub mod bytes_of_runtime;
pub mod db;
pub mod infer;
pub mod lower;
pub mod resolve;
pub mod set_of_runtime;

// Cost-tiered optimization levels — the `OptLevel` enum + the `PassManager` that gates each
// backend-independent Core pass by its declared tier, running above the backend seam so every backend
// inherits its passes (`DESIGN-tiered-optimization-levels-rcdzc.md`).
pub mod opt;

// The target-neutral boundary layout (exports by declared signature, reachable set, emission order),
// computed once above the backend seam.
pub mod layout;

// The backend seam + the wasm backend. Everything above the seam is target-neutral; a backend is a
// function of the typed core and the layout, chosen at the seam by `Target`.
pub mod backend;

// The build-tool ABI (kinded artifacts + diagnostics) and the pure compilation entry (no I/O — the
// part that ports to the Cadenza self-host). A CLI bin puts filesystem/args on top of `compile`.
pub mod abi;
// ABI-projection bridges: the rcdzc-internal `diag::{Reject, Fix, Code}` -> shared-crate `Diagnostic`/
// `DiagnosticFix` conversions, as free fns (the orphan rule forbids them as inherent impls on the moved
// boundary types). Only rcdzc PRODUCES diagnostics; a host-boundary helper, not ported to the self-host.
pub(crate) mod abi_bridge;
pub mod compile;
// Package linking — merge N named `ast` input artifacts into ONE compilation unit (one merged arena
// under a synthesized `(do …)` root) BEFORE the pure pipeline runs, so `Db::load` sees one program
// assembled from many files (`DESIGN-package-linking.md`).
pub mod link;
// The sidecar request list — the program that DRIVES a compilation (Emit an output column / Query a
// fact column), crossing as one more kinded input artifact. Generalizes `compile`'s `targets`.
pub mod sidecar;
// The span side-table — source byte ranges keyed by `StructId`, crossing as its OWN kinded input
// artifact (`kind == "spans"`) so the AST stays span-free. Read by the backend to emit debug info
// (`DESIGN-debug-info-rcdzc.md` §2.1a).
pub mod spans;

// The host boundary — process/thread/stack concerns the pure core excludes (NOT ported to the
// Cadenza self-host). Runs compilation on a stack sized to reach the recursive-descent depth guard,
// so `decline-don't-crash` holds in every build profile with no environment to remember.
pub mod host;
pub mod wit_world;

// The `rcdzc` compile command surface (arg parsing + filesystem + the trace sink), factored into the
// library so both the standalone `rcdzc` bin and the unified `cdz` bin drive ONE implementation. Also
// a host-boundary module — NOT ported to the self-host.
pub mod cli;

pub use abi::{
    Artifact, CompileOutput, Diagnostic, DiagnosticFix, FixKind, Severity, WRAP_HOLE,
    wrap_prefix_suffix,
};
pub use backend::Target;
pub use compile::{
    compile, compile_component, compile_with_opt, compile_with_opt_and_overflow, diagnostics,
};
pub use host::run_with_compiler_stack;
pub use opt::{CorePass, OptLevel, PassManager};
pub use sidecar::{Query, Request};

// Shared test fixtures + the Stage-0 end-to-end tests (compiled only under `#[cfg(test)]`).
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;

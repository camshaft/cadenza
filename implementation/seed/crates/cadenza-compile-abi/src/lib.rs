//! The compile-boundary contract types — the vocabulary the build tool speaks across the compiler
//! seam, shared by the compiler that produces them (`rcdzc`) and the thin front-door that delegates to
//! it (`cdz`). See the crate manifest for the extraction rationale (the compile-boundary analogue of
//! `cadenza-ast`'s AST-wire crate).
//!
//! This first slice holds the two pure-data leaf enums the boundary names — the requested [`Target`]
//! and the [`OptLevel`]. Later slices move the kinded `Artifact` list, the sidecar `Request`/`Query`
//! wire + codec, and the source `spans` side-table. The compiler IMPLEMENTATIONS behind the boundary
//! (query eval over a live `Db`, `compile`, the backends) stay in `rcdzc`.

pub mod opt;
pub mod target;

pub use opt::OptLevel;
pub use target::Target;

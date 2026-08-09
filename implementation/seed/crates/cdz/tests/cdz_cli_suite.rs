//! Single integration-test binary aggregating `cdz`'s 58 CLI suites.
//!
//! Each `*_cli.rs` was its OWN `tests/*.rs`, which Cargo links as a SEPARATE test binary — 58 full
//! links of `cdz` (the whole-workspace crate: the heaviest, least-incremental crate in the gate) + 58
//! codegen cycles per `cargo test`. They all drive the built `cdz` binary via `CARGO_BIN_EXE_cdz`
//! (genuine E2E), but nothing requires a SEPARATE binary per file — the split only multiplied link
//! time 58x. Consolidating them here as `mod`s of files under `tests/suite/` (a SUBDIR Cargo does NOT
//! auto-compile as its own binary) collapses the 58 links into ONE while keeping every test function,
//! its module path, and its `CARGO_BIN_EXE_cdz`-driven E2E semantics byte-identical. Each file becomes
//! its own `mod`, so the same-named per-file helpers (`fn run`, `temp_dir`, `PROG`, …) do not collide.
//! The four files that use the shared `tests/common/` helper reach it via `#[path = "../common/mod.rs"]
//! mod common;` (common/ stays where it is, single-sourced).
//!
//! To add another CLI suite: drop the file in `tests/suite/` and add a `mod` line below — do NOT create
//! a new top-level `tests/*.rs` (that re-introduces a separate binary and the 58x link cost).

#![allow(clippy::all)]

mod suite {
    mod add_cli;
    mod build_cli;
    mod cad_cli;
    mod calc_cli;
    mod check_imports_cli;
    mod check_project_cli;
    mod chor_cli;
    mod clean_cli;
    mod clones_cli;
    mod compile_closure_cli;
    mod compile_debug_cli;
    mod compile_opt_level_cli;
    mod completions_cli;
    mod convert_cli;
    mod corpus_cli;
    mod cross_component_cli;
    mod def_cli;
    mod diff_cli;
    mod doc_at_cli;
    mod doc_cli;
    mod doc_module_cli;
    mod doctor_cli;
    mod exports_cli;
    mod fix_cli;
    mod fmt_comment_guard_cli;
    mod fmt_project_cli;
    mod func_layout_cli;
    mod func_layout_witness_cli;
    mod help_smoke_cli;
    mod highlight_cli;
    mod init_cli;
    mod instantiations_cli;
    mod lint_cli;
    mod lsp_cli;
    mod metadata_cli;
    mod new_cli;
    mod normalize_cli;
    mod param_manifest_cli;
    mod path_deps_cli;
    mod peer_list_handle_cli;
    mod query_cli;
    mod remove_cli;
    mod run_cli;
    mod run_emitted_cli;
    mod run_ml_cli;
    mod run_rust_cli;
    mod scope_cli;
    mod smith_cli;
    mod symbols_cli;
    mod test_backtrace_cli;
    mod test_manifest_cli;
    mod test_per_file_cli;
    mod tracing_cli;
    mod tree_cli;
    mod type_at_cli;
    mod type_cli;
    mod uses_cli;
    mod watch_cli;
}

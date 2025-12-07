//! Shared LSP (Language Server Protocol) utilities for Cadenza.
//!
//! This crate provides common LSP functionality that can be used by both:
//! - Native LSP server (via tower-lsp in the cadenza CLI)
//! - WASM LSP server (via wasm-bindgen in cadenza-web)

pub mod core;

pub use core::{offset_to_position, parse_to_diagnostics, position_to_offset};

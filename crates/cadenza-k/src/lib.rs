//! Cadenza K Framework Reference Implementation
//!
//! This crate contains a formal semantic definition of the Cadenza language
//! using the K Framework. The actual implementation is in K definition files
//! (`.k` files) in this crate's root directory.
//!
//! ## Purpose
//!
//! This crate serves as:
//! - A reference implementation for the Cadenza semantics
//! - A formal specification that can be executed
//! - A testing harness for validating the language semantics
//!
//! ## Usage
//!
//! See the README.md in this crate for detailed usage instructions.
//! In brief:
//!
//! 1. Install K Framework
//! 2. Build: `make` or `make kompile`
//! 3. Test: `make test`
//!
//! ## Structure
//!
//! - `cadenza.k` - Main K definition file
//! - `Makefile` - Build and test automation
//! - `tests/` - Test scripts and utilities
//!
//! ## Note
//!
//! This is an empty Rust crate - the actual implementation is in K.
//! This crate exists only to integrate with the Cargo workspace.

#![doc = include_str!("../README.md")]

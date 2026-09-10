//! Program resolution + hashing (`DESIGN-http-outpost-conformance-harness.md` §3.2). A run-spec's
//! `config.programs` names programs by manifest name; the nix harness rig compiles each `programs/…` guest
//! and hands the driver a [`ProgramManifest`] of name → compiled `.wasm` path. To seed one, the driver needs
//! the program's content hash in two forms, and they must be the SAME hash:
//!
//! - the base62 [`Hash`] text — the CAS path key (`PUT /{hash}`), and
//! - the raw 33 bytes — the `ProgramHash` the mock ships in `ControlConfig.root_router`.
//!
//! Rather than trust a pre-computed value, the driver computes it itself from the bytes with
//! [`program_hash`] — `Hash::of(HashTag::Program, …)`, the exact call the deploy tool (`cdz-http-programhash`)
//! and the mock's route table use — so both forms are one canonical hash and can never drift.

use bytes::Bytes;
use cdz_contract::{Hash, HashTag};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A compiled program resolved for seeding: its wasm `bytes` + its content hash in the two forms the harness
/// needs (`hash_text` = the base62 CAS path key; `hash_bytes` = the raw 33-byte `ProgramHash`). Both derive
/// from the same [`program_hash`] call over `bytes`.
#[derive(Debug, Clone)]
pub struct ResolvedProgram {
    pub bytes: Bytes,
    pub hash_text: String,
    pub hash_bytes: Bytes,
}

/// A compiled component's `ProgramHash`, in both forms: `(base62 text, raw 33 bytes)`. Computed with the
/// canonical `Hash::of(HashTag::Program, …)` — matching `cdz-http-programhash` + the mock — so the CAS key
/// and the shipped `ProgramHash` are the same hash.
#[must_use]
pub fn program_hash(wasm: &[u8]) -> (String, Bytes) {
    let h = Hash::of(HashTag::Program, wasm);
    (h.to_string(), Bytes::copy_from_slice(h.as_bytes()))
}

/// The programs the harness can seed: program name → its compiled `.wasm` path (provided by the nix rig,
/// which compiles `programs/`). [`resolve`](Self::resolve) reads the bytes + computes the hash.
#[derive(Debug, Clone, Default)]
pub struct ProgramManifest {
    paths: HashMap<String, PathBuf>,
}

impl ProgramManifest {
    /// An empty manifest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a program `name` → its compiled `.wasm` `path`.
    pub fn insert(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        self.paths.insert(name.into(), path.into());
    }

    /// The registered wasm path for `name`, if any.
    #[must_use]
    pub fn path(&self, name: &str) -> Option<&Path> {
        self.paths.get(name).map(PathBuf::as_path)
    }

    /// Resolve `name`: read its compiled wasm + compute its `ProgramHash` (both forms).
    ///
    /// # Errors
    /// `name` is not in the manifest, or its wasm file cannot be read.
    pub fn resolve(&self, name: &str) -> Result<ResolvedProgram, String> {
        let path = self
            .paths
            .get(name)
            .ok_or_else(|| format!("program {name:?} is not in the harness manifest"))?;
        let bytes = std::fs::read(path)
            .map_err(|e| format!("reading program {name:?} at {}: {e}", path.display()))?;
        let (hash_text, hash_bytes) = program_hash(&bytes);
        Ok(ResolvedProgram {
            bytes: Bytes::from(bytes),
            hash_text,
            hash_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_hash_is_the_canonical_program_hash_in_both_forms() {
        let wasm = b"\0asm\x01\0\0\0 pretend component bytes";
        let (text, bytes) = program_hash(wasm);
        // The canonical hash: exactly Hash::of(HashTag::Program, wasm), in text + raw form.
        let canonical = Hash::of(HashTag::Program, wasm);
        assert_eq!(text, canonical.to_string());
        assert_eq!(bytes.as_ref(), canonical.as_bytes());
        // A ProgramHash is 33 bytes (1 tag + 32 digest); the text is the fixed-width base62 rendering.
        assert_eq!(bytes.len(), 33);
        assert_eq!(text.len(), Hash::TEXT_LEN);
        // Distinct bytes → distinct hash (sanity that it hashes the content, not a constant).
        let (other, _) = program_hash(b"different bytes");
        assert_ne!(text, other);
    }

    /// A unique temp path for a test wasm blob (no tempfile dep; cleaned up by the test).
    fn temp_wasm(tag: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cdz-http-conf-{}-{}-{tag}.wasm",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn resolve_reads_the_wasm_and_computes_its_hash() {
        let wasm = b"a compiled guest component";
        let path = temp_wasm("resolve", wasm);
        let mut manifest = ProgramManifest::new();
        manifest.insert("http-hello", &path);

        let resolved = manifest.resolve("http-hello").expect("resolves");
        assert_eq!(resolved.bytes.as_ref(), wasm);
        let (text, bytes) = program_hash(wasm);
        assert_eq!(resolved.hash_text, text);
        assert_eq!(resolved.hash_bytes, bytes);

        assert_eq!(manifest.path("http-hello"), Some(path.as_path()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolve_errors_on_unknown_program_or_missing_file() {
        let mut manifest = ProgramManifest::new();
        // Unknown name.
        assert!(
            manifest
                .resolve("nope")
                .unwrap_err()
                .contains("not in the harness manifest")
        );
        // Known name, missing file.
        manifest.insert("gone", "/nonexistent/does-not-exist.wasm");
        assert!(
            manifest
                .resolve("gone")
                .unwrap_err()
                .contains("reading program")
        );
    }
}

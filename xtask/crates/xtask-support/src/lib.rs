//! `xtask-support` — shared foundation for the decomposed xtask commands (v-xtask-decompose). First
//! slice: the content-addressing / build-cache-fingerprint helpers, carved out of the xtask monolith so
//! per-command crates reuse them without duplication. (The corpus/Tools/convert machinery follows here.)

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The platform CONTENT ADDRESS of `bytes`: `Hash::of(Blob, bytes)` rendered as the canonical string —
/// byte-identical to `cdz-run`'s `content_address` and the store's `put()` key, so the store address ==
/// blob key == compose-dep `+hash` == `REQUIRED_RUNTIME_HASH` are one string across the fleet (design §8).
pub fn content_address(bytes: &[u8]) -> String {
    cdz_contract::Hash::of(cdz_contract::HashTag::Blob, bytes).to_string()
}

/// A deterministic SHA-256 fingerprint of a whole directory TREE (sorted path + content), used as an
/// internal build-cache key (NOT the content-address digest above — this is a private cache fingerprint,
/// no cross-boundary contract). `None` if the tree can't be enumerated.
pub fn hash_tree(root: &Path) -> Option<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(root, &mut files).ok()?;
    files.sort();
    let mut h = Sha256::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0u8]); // path/content separator
        if let Ok(bytes) = std::fs::read(f) {
            h.update(&bytes);
        }
        h.update([0u8]); // file separator
    }
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    Some(s)
}

/// Recursively collect every regular file under `dir` into `out` (used by `hash_tree`).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_files(&path, out)?;
        } else if ty.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

// ── Corpus record model (v-xtask-decompose slice 2a) — the parsed shape of the `cdz-syntax corpus`
// stream, SHARED by the gate/roundtrip/emit commands. Moved here so the per-command crates (xtask-gate,
// xtask-roundtrip, …) reuse the ONE parser + model instead of duplicating it (drift-sensitive). All
// std-only; fields are `pub` so a consumer crate can construct/read them.

/// A parsed corpus record (the flat stream `cdz-syntax corpus` emits).
pub struct CorpusRecord {
    pub description: String,
    pub program: String,
    /// Sibling LIBRARY modules of a multi-file PACKAGE case, each a `(name, program)` from a `module`
    /// record line. Empty for a single-file case.
    pub modules: Vec<(String, String)>,
    /// PEER components of a CROSS-COMPONENT case — each an `(interface, provider-program)` from a `peer`
    /// record line. Empty for a single-component case.
    pub peers: Vec<(String, String)>,
    /// One or more TRIALS — each an optional `(call …)` paired with the `expect` payload it must produce.
    pub trials: Vec<Trial>,
    /// The `(needs …)` capabilities a case documents (documentation only now — grading is by what the
    /// compiler actually does).
    #[allow(dead_code)]
    pub needs: Vec<String>,
    /// The HOST-CALL RESPONSES (E2h) — `(op, value)` pairs from the stream's `host-response` lines.
    pub host_responses: Vec<(String, String)>,
    /// The recorded HOST-CALL sequence (E2h) — the dotted `E.op` names from the stream's `host-call` lines.
    pub host_calls: Vec<String>,
    /// The WARNING pins — `(code, optional message-substring)` from the case's `(warns …)` clauses.
    pub warns: Vec<(String, Option<String>)>,
    /// An explicit WIT WORLD the case imposes (from the stream's `wit-world` line). `None` for synthesized.
    pub wit_world: Option<String>,
    /// The interface a `(wit-world …)` case's guest exports under (stream `component-name` line).
    pub component_name: Option<String>,
    /// The live-heap-cell count a `(live-objects N)` clause asserts after the run. `None` if absent.
    pub live_objects: Option<u32>,
}

/// One (call, expected-payload) trial of a case — a single run of the compiled program.
pub struct Trial {
    /// The `(call …)` for this trial, or `None` to invoke the sole export with no arguments.
    pub call: Option<Call>,
    /// The `expect` payload, e.g. `output (: 42 Int64)`, `error CDZ0201`, `trap "…"`.
    pub expect: String,
}

/// A corpus case's `(call <export> <arg>…)` clause, parsed from the record stream.
pub struct Call {
    pub export: String,
    pub args: Vec<String>,
    /// A `(then <arg>…)` continuation (two-call-on-one-handle): the SECOND call's arguments, or `None`.
    pub second_call: Option<Vec<String>>,
    /// A `(drop)` clause: resource-drop the minted closure handle after the call(s) before reading.
    pub drop_handle: bool,
    /// A `(call-method <member> …)` clause: the NAMED value-resource member to invoke. `None` otherwise.
    pub method: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_tree_is_deterministic_and_change_sensitive() {
        let base = std::env::temp_dir().join(format!("cdz-hashtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.cdz"), "alpha").unwrap();
        std::fs::write(base.join("sub/b.cdz"), "beta").unwrap();

        let h1 = hash_tree(&base).expect("hashable");
        let h2 = hash_tree(&base).expect("hashable");
        assert_eq!(
            h1, h2,
            "same tree → same hash (order-independent, deterministic)"
        );

        // A content edit changes the hash.
        std::fs::write(base.join("a.cdz"), "alpha!").unwrap();
        let h3 = hash_tree(&base).expect("hashable");
        assert_ne!(h1, h3, "editing a file's content changes the tree hash");

        // Adding a file changes the hash.
        std::fs::write(base.join("c.cdz"), "gamma").unwrap();
        let h4 = hash_tree(&base).expect("hashable");
        assert_ne!(h3, h4, "adding a file changes the tree hash");

        // A rename (same bytes, different path) changes the hash — path is folded in.
        std::fs::remove_file(base.join("c.cdz")).unwrap();
        std::fs::write(base.join("d.cdz"), "gamma").unwrap();
        let h5 = hash_tree(&base).expect("hashable");
        assert_ne!(h4, h5, "a rename changes the tree hash (path folded in)");

        let _ = std::fs::remove_dir_all(&base);
    }
}

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

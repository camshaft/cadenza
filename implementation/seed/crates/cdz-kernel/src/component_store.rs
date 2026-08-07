//! component_store — resolve a Cadenza component's bytes from an on-disk content-addressed store, the
//! layout v-nix's `packages.store` (nix `componentStore`) uses and the reference resolver (`cdz-run`)
//! reads. This is the RESOLUTION half of the §19e/§23 transitive-dep compose: given a store dir, fetch a
//! component's bytes either by its content HASH (a reducer's `+<hash>` dep import) or by a MANIFEST NAME
//! (the runtime's own bare inter-runtime imports like `cadenza:nfc/normalize`, mapped in `runtime.toml`).
//!
//! ## Store layout (v-nix-confirmed, mirrors cdz-run's `resolve_nfc_from_store`)
//! - `<store>/<sha256hex>.wasm` — one component per file, named by its SHA-256 content address.
//! - `<store>/runtime.toml` — a `name = "<sha256hex>"` manifest mapping the well-known runtime-internal
//!   components (`runtime`, `debug_runtime`, `nfc`, …) to their SHA-256 content hashes. This is how a BARE
//!   interface import (no `+<sha256hex>` build-metadata) is resolved: the importer names the interface, the
//!   manifest names the providing component's SHA-256 hash.
//!
//! ## The dual-hash boundary (operator ruling A, concierge answer 2026-08-05)
//! This reader is the ONE place two content-address algorithms meet, so it is documented explicitly rather
//! than left implicit (a silent dual-hash system is where someone later assumes uniformity and reintroduces
//! a mismatch):
//! - The EXTERNAL seed/nix component store is **SHA-256**-addressed — by ALL its producers: `xtask`'s
//!   `content_address`, `cdz-run::cli::content_address` (the canonical impl, `+ resolve_nfc_from_store`),
//!   and v-nix's `componentStore` (`flake.nix`, `sha256sum → <sha256hex>.wasm`). `REQUIRED_RUNTIME_HASH` IS
//!   this SHA-256. So THIS reader content-verifies each fetched blob with SHA-256 (see [`sha256_digest`], a
//!   byte-compare) to MATCH the store it reads — NOT the kernel's blake3 [`Hash::of`], which would mismatch.
//! - Kernel-INTERNAL durable state (events, KV nodes, blobs — `blob::DiskBlobStore`) stays **blake3**
//!   ([`crate::hash::Hash::of`]). That address never crosses into this external store and vice versa.
//!
//! The full SHA-256 store contract is anchored on `cdz-run::cli::content_address` (v-nix owns it): SHA-256
//! lowercase-hex of the component bytes; `<sha256hex>.wasm` + `runtime.toml` layout; `runtime.toml` maps the
//! runtime's BARE inter-runtime deps by name→hash, distinct from a program's own `+hash`-in-import dep.
//!
//! Every fetch CONTENT-VERIFIES ([`sha256_digest(bytes)`](sha256_digest) `== *hash.as_bytes()`, a raw
//! `[u8; 32]` byte-compare — `as_bytes()` returns `&[u8; 32]`, so the check derefs) before returning — a
//! corrupt or substituted blob can never compose silently. The two paths
//! differ only in how the hash is obtained: a `+<hash>` dep carries it in the import name; a bare dep looks
//! it up by name in `runtime.toml`.
//!
//! This module is pure resolution (filesystem reads + hashing) — no wasmtime, no compose. The
//! transitive-dep compose in [`crate::wasm_host`] consumes it to supply a dep's own dep bytes.

use crate::hash::Hash;
use std::path::{Path, PathBuf};

/// A content-addressed component store rooted at a directory (`<hash>.wasm` files + a `runtime.toml`
/// name→hash manifest). Cheap to construct (just holds the root path); every resolve hits the filesystem.
#[derive(Debug, Clone)]
pub struct ComponentStore {
    root: PathBuf,
}

/// Why a component-store resolution failed. A sum (no-sentinels) so each failure mode is distinguishable:
/// a missing manifest, a name absent from it, a missing blob file, an unreadable file, or a
/// content-address MISMATCH (the integrity gate — bytes on disk don't hash to their key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// `runtime.toml` is absent or unreadable at the store root — needed to resolve a bare/named dep.
    ManifestMissing { path: String },
    /// The manifest has no `<name> = "<hash>"` line for the requested name (e.g. `nfc`).
    NameNotInManifest { name: String },
    /// A manifest hash string isn't a valid content address (malformed `runtime.toml` entry).
    MalformedHash { name: String, value: String },
    /// No `<hash>.wasm` blob in the store for the resolved hash.
    BlobMissing { hash: String },
    /// The blob file exists but couldn't be read (I/O error).
    BlobUnreadable { hash: String, source: String },
    /// The blob's bytes do NOT hash to their key — a corrupt or substituted entry (integrity failure).
    ContentAddressMismatch { hash: String },
}

impl ComponentStore {
    /// Open a store at `root` (no I/O — resolves are lazy). The root is the dir holding `<hash>.wasm` +
    /// `runtime.toml` (v-nix's `componentStore` / the `CDZ_STORE` env path).
    pub fn open(root: impl AsRef<Path>) -> Self {
        ComponentStore {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Fetch a component's bytes BY CONTENT HASH — a reducer's `+<hash>` dep import (the hash is in the
    /// import name). Reads `<root>/<hash>.wasm` and verifies its content address matches `hash`.
    ///
    /// WARNING:`hash` is the EXTERNAL store's **SHA-256** address (the 32-byte value carried in the dep's
    /// `+<hash>` import name, e.g. via `Hash::from_hex(sha256hex)`), even though the parameter is typed as
    /// the kernel's [`Hash`] (a scheme-agnostic 32-byte container). Do NOT pass `Hash::of(bytes)` — that is
    /// the kernel-internal **blake3** address and will fail the SHA-256 content-verify below, surfacing as
    /// `ContentAddressMismatch` (looks like corruption, isn't). See the dual-hash boundary at the module
    /// head. (A dedicated `StoreAddr` newtype to make this un-mixable at the type level is a follow-up —
    /// #2218 review; deferred as a broader API change since `ComponentDep.hash` + `declared_deps` share the
    /// type.)
    pub fn get_by_hash(&self, hash: &Hash) -> Result<Vec<u8>, StoreError> {
        let hex = hash.to_hex();
        let path = self.root.join(format!("{hex}.wasm"));
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::BlobMissing { hash: hex.clone() }
            } else {
                StoreError::BlobUnreadable {
                    hash: hex.clone(),
                    source: e.to_string(),
                }
            }
        })?;
        // Integrity gate: the bytes MUST hash to their key, or a corrupt/substituted blob would compose.
        // Verify with SHA-256 — the EXTERNAL store's address algorithm (see the dual-hash boundary note at
        // the module head) — NOT the kernel's blake3 `Hash::of`, which would mismatch every real blob.
        // Compare the raw 32-byte digest to the expected key's bytes directly — no hex-encode + String
        // compare (#2220 review c1). `Hash` is a scheme-agnostic 32-byte container, so `as_bytes()` is the
        // sha256 value here (see the get_by_hash # Note).
        if sha256_digest(&bytes) != *hash.as_bytes() {
            return Err(StoreError::ContentAddressMismatch { hash: hex });
        }
        Ok(bytes)
    }

    /// Fetch a runtime-internal component's bytes BY MANIFEST NAME — a bare inter-runtime import (e.g.
    /// `nfc`), resolved via `runtime.toml`'s `<name> = "<hash>"` line, then fetched + content-verified by
    /// that hash. This is the resolution path for the runtime's own bare `cadenza:nfc/normalize` import
    /// (the transitive dep the §19e handle-lowered fold must compose before instantiating the runtime).
    pub fn get_by_manifest_name(&self, name: &str) -> Result<Vec<u8>, StoreError> {
        let manifest_path = self.root.join("runtime.toml");
        let manifest =
            std::fs::read_to_string(&manifest_path).map_err(|_| StoreError::ManifestMissing {
                path: manifest_path.display().to_string(),
            })?;
        let hex =
            manifest_hash_for(&manifest, name).ok_or_else(|| StoreError::NameNotInManifest {
                name: name.to_string(),
            })?;
        let hash = Hash::from_hex(&hex).ok_or_else(|| StoreError::MalformedHash {
            name: name.to_string(),
            value: hex.clone(),
        })?;
        self.get_by_hash(&hash)
    }
}

/// The raw SHA-256 digest of `bytes` — the EXTERNAL component store's content address, as 32 bytes. The
/// store reader ([`ComponentStore::get_by_hash`]) compares THIS directly to the expected key's bytes (no
/// hex round-trip). SHA-256, NOT the kernel-internal blake3 [`Hash::of`], because the on-disk blobs are
/// SHA-256-addressed by ALL their producers — `xtask`, `cdz-run::cli::content_address` (the canonical
/// impl + full contract), and v-nix's `componentStore` (`flake.nix`, `sha256sum`) — and
/// `REQUIRED_RUNTIME_HASH` IS this value. See the dual-hash boundary at the module head.
fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

/// The SHA-256 content address as lowercase hex (the `<sha256hex>.wasm` file-name form). A thin hex
/// wrapper over [`sha256_digest`]; used where the hex string is needed (e.g. constructing a store key in
/// tests). The verify path byte-compares via [`sha256_digest`] directly and never allocates this.
#[cfg(test)]
fn sha256_content_address(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in sha256_digest(bytes) {
        let _ = write!(s, "{b:02x}"); // writing to a String is infallible
    }
    s
}

/// Parse a `runtime.toml` for a `<name> = "<hash>"` line, returning the hash string. A minimal line-based
/// scan (the manifest is a flat `key = "value"` map — `runtime`/`debug_runtime`/`nfc`); we avoid a full
/// TOML-parser dep for a two-field manifest. Matches the KEY exactly (so `nfc` doesn't match `nfc_extra`).
fn manifest_hash_for(manifest: &str, name: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(name)?;
        // The char right after the key must be `=` or whitespace-then-`=`, so `nfc` doesn't match `nfcx`.
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=')?;
        Some(rest.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) {
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    /// The store's content address for `bytes` as a `Hash` key — built from the SHA-256 address (the
    /// EXTERNAL store's algorithm), the same way production derives it from a dep's `+<sha256hex>` import
    /// name. NOT `Hash::of` (blake3), which would name a file the reader's sha256 verify rejects.
    fn store_hash(bytes: &[u8]) -> Hash {
        Hash::from_hex(&sha256_content_address(bytes))
            .expect("sha256 hex is a valid 64-char content address")
    }

    // A hash-addressed fetch round-trips + content-verifies. Uses a real temp dir with a `<hash>.wasm` file.
    #[test]
    fn get_by_hash_reads_and_verifies() {
        let dir = std::env::temp_dir().join(format!("cdzstore-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"component-bytes-abc".to_vec();
        let hash = store_hash(&bytes);
        write(&dir, &format!("{}.wasm", hash.to_hex()), &bytes);
        let store = ComponentStore::open(&dir);
        assert_eq!(store.get_by_hash(&hash).unwrap(), bytes);
        // A hash with no blob → BlobMissing.
        let absent = store_hash(b"nope");
        assert!(matches!(
            store.get_by_hash(&absent),
            Err(StoreError::BlobMissing { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    // A CORRUPT blob (bytes don't hash to their filename) is rejected, not composed.
    #[test]
    fn get_by_hash_rejects_a_content_address_mismatch() {
        let dir = std::env::temp_dir().join(format!("cdzstore-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = b"the-real-bytes".to_vec();
        let hash = store_hash(&good);
        // Write DIFFERENT bytes under the good hash's name → integrity failure.
        write(&dir, &format!("{}.wasm", hash.to_hex()), b"tampered");
        let store = ComponentStore::open(&dir);
        assert!(matches!(
            store.get_by_hash(&hash),
            Err(StoreError::ContentAddressMismatch { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    // A manifest-name fetch resolves nfc → hash → verified bytes (the runtime's bare-dep path).
    #[test]
    fn get_by_manifest_name_resolves_nfc_via_runtime_toml() {
        let dir = std::env::temp_dir().join(format!("cdzstore-nfc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let nfc_bytes = b"nfc-component".to_vec();
        let nfc_hash = store_hash(&nfc_bytes);
        let rt_bytes = b"runtime-component".to_vec();
        let rt_hash = store_hash(&rt_bytes);
        write(&dir, &format!("{}.wasm", nfc_hash.to_hex()), &nfc_bytes);
        write(&dir, &format!("{}.wasm", rt_hash.to_hex()), &rt_bytes);
        write(
            &dir,
            "runtime.toml",
            format!(
                "# a store manifest\nruntime = \"{}\"\nnfc = \"{}\"\n",
                rt_hash.to_hex(),
                nfc_hash.to_hex()
            )
            .as_bytes(),
        );
        let store = ComponentStore::open(&dir);
        assert_eq!(store.get_by_manifest_name("nfc").unwrap(), nfc_bytes);
        assert_eq!(store.get_by_manifest_name("runtime").unwrap(), rt_bytes);
        // A name not in the manifest → NameNotInManifest.
        assert!(matches!(
            store.get_by_manifest_name("debug_runtime"),
            Err(StoreError::NameNotInManifest { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    // A missing runtime.toml → ManifestMissing (distinct from a name absent from a present manifest).
    #[test]
    fn get_by_manifest_name_without_a_manifest_is_manifest_missing() {
        let dir = std::env::temp_dir().join(format!("cdzstore-nomanifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ComponentStore::open(&dir);
        assert!(matches!(
            store.get_by_manifest_name("nfc"),
            Err(StoreError::ManifestMissing { .. })
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    // The manifest scan matches the KEY exactly — `nfc` must not match a `nfc_extra` line.
    #[test]
    fn manifest_scan_matches_the_key_exactly() {
        assert_eq!(
            manifest_hash_for("nfc = \"abc\"\n", "nfc"),
            Some("abc".to_string())
        );
        // A longer key sharing the prefix must NOT match the shorter name.
        assert_eq!(manifest_hash_for("nfc_extra = \"xyz\"\n", "nfc"), None);
        // Whitespace tolerance.
        assert_eq!(
            manifest_hash_for("  nfc   =   \"def\"  \n", "nfc"),
            Some("def".to_string())
        );
    }
}

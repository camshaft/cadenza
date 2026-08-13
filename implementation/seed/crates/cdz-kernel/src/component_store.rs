//! component_store — resolve a Cadenza component's bytes from an on-disk content-addressed store, the
//! layout v-nix's `packages.store` (nix `componentStore`) uses and the reference resolver (`cdz-run`)
//! reads. This is the RESOLUTION half of the §19e/§23 transitive-dep compose: given a store dir, fetch a
//! component's bytes either by its content HASH (a reducer's `+<hash>` dep import) or by a MANIFEST NAME
//! (the runtime's own bare inter-runtime imports like `cadenza:nfc/normalize`, mapped in `runtime.toml`).
//!
//! ## Store layout (v-nix-confirmed, mirrors cdz-run's `resolve_nfc_from_store`)
//! - `<store>/<hash>.wasm` — one component per file, named by its content address (lowercase hex).
//! - `<store>/runtime.toml` — a `name = "<hash>"` manifest mapping the well-known runtime-internal
//!   components (`runtime`, `debug_runtime`, `nfc`, …) to their content hashes. This is how a BARE
//!   interface import (no `+<hash>` build-metadata) is resolved: the importer names the interface, the
//!   manifest names the providing component's hash.
//!
//! ## Content addressing — ONE hash everywhere (operator directive 2026-08-08)
//! The kernel unified onto a SINGLE content-address algorithm — BLAKE3, via [`crate::hash::Hash::of`]. The
//! former dual-hash boundary (kernel blake3 vs external SHA-256) is GONE: this external store's producers
//! (`xtask`'s `content_address`, `cdz-run::cli::content_address`, and v-nix's `componentStore` `b3sum →
//! <hash>.wasm`) and `REQUIRED_RUNTIME_HASH` are ALL blake3 now, the same digest as kernel-internal durable
//! state (events, KV nodes, blobs). So THIS reader content-verifies each fetched blob with `Hash::of` — the
//! same algorithm that addresses everything — and a `+<hash>` dep import, a blob-store key, and a
//! `CDZ_STORE` address are one interchangeable space (killing the resolve-time mismatch a split used to
//! risk). The store contract is anchored on `cdz-run::cli::content_address` (v-nix owns it): blake3
//! lowercase-hex of the component bytes; `<hash>.wasm` + `runtime.toml` layout; `runtime.toml` maps the
//! runtime's BARE inter-runtime deps by name→hash, distinct from a program's own `+hash`-in-import dep.
//!
//! Every fetch CONTENT-VERIFIES (`Hash::of(bytes).as_bytes() == hash.as_bytes()`, a raw `[u8; 32]`
//! byte-compare) before returning — a corrupt or substituted blob can never compose silently. The two paths
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
    ManifestMissing { path: std::sync::Arc<str> },
    /// The manifest has no `<name> = "<hash>"` line for the requested name (e.g. `nfc`).
    NameNotInManifest { name: std::sync::Arc<str> },
    /// A manifest hash string isn't a valid content address (malformed `runtime.toml` entry).
    MalformedHash {
        name: std::sync::Arc<str>,
        value: std::sync::Arc<str>,
    },
    /// No `<hash>.wasm` blob in the store for the resolved hash.
    BlobMissing { hash: std::sync::Arc<str> },
    /// The blob file exists but couldn't be read (I/O error).
    BlobUnreadable {
        hash: std::sync::Arc<str>,
        source: std::sync::Arc<str>,
    },
    /// The blob's bytes do NOT hash to their key — a corrupt or substituted entry (integrity failure).
    ContentAddressMismatch { hash: std::sync::Arc<str> },
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
    /// `hash` is the store's content address (carried in the dep's `+<hash>` import name, e.g. via
    /// `Hash::from_hex(hex)`) — now the SAME algorithm as [`Hash::of`] since the kernel unified onto one
    /// hash (2026-08-08), so `Hash::of(bytes)` and this address are interchangeable and the content-verify
    /// below can't spuriously mismatch on an algorithm split.
    pub fn get_by_hash(&self, hash: &Hash) -> Result<Vec<u8>, StoreError> {
        let hex = hash.to_hex();
        let path = self.root.join(format!("{hex}.wasm"));
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::BlobMissing {
                    hash: hex.as_str().into(),
                }
            } else {
                StoreError::BlobUnreadable {
                    hash: hex.as_str().into(),
                    source: e.to_string().into(),
                }
            }
        })?;
        // Integrity gate: the bytes MUST hash to their key, or a corrupt/substituted blob would compose.
        // Verify with the ONE unified content-address algorithm — `Hash::of` (blake3) — the same digest the
        // external store's producers (xtask/cdz-run/v-nix `componentStore`) now use post-collapse (operator
        // directive 2026-08-08: one hash everywhere; the former sha256-vs-blake3 dual boundary is gone).
        // Compare the raw 32-byte digest directly — no hex-encode + String compare (#2220 review c1).
        if Hash::of(&bytes).as_bytes() != hash.as_bytes() {
            return Err(StoreError::ContentAddressMismatch { hash: hex.into() });
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
                path: manifest_path.display().to_string().into(),
            })?;
        let hex = manifest_hash_for(&manifest, name)
            .ok_or_else(|| StoreError::NameNotInManifest { name: name.into() })?;
        // ACCEPTED hash-encoding exception (concierge scope ruling 2026-08-13): `runtime.toml` is a
        // BUILD/human-authored config manifest (written by v-nix/xtask/cdz-run), so parsing its
        // `<name> = "<hash>"` value is a permitted config-LOAD decode — the same input-edge class as the
        // sanctioned `outpost_session` config value, NOT a runtime/wire from_hex violation. The blanket
        // raw-bytes directive covers runtime/wire/storage DATA values, not build-config inputs.
        let hash = Hash::from_hex(&hex).ok_or_else(|| StoreError::MalformedHash {
            name: name.into(),
            value: hex.as_str().into(),
        })?;
        self.get_by_hash(&hash)
    }
}

/// The raw SHA-256 digest of `bytes` — the EXTERNAL component store's content address, as 32 bytes. The
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

    /// The store's content address for `bytes` as a `Hash` key. Post-collapse this is just `Hash::of` —
    /// the ONE unified content-address algorithm the store's producers and the reader's verify now share
    /// (operator directive 2026-08-08: one hash everywhere).
    fn store_hash(bytes: &[u8]) -> Hash {
        Hash::of(bytes)
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

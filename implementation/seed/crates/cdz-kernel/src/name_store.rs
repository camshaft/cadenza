//! Mutable-name → hash store — the §4c global-store WRITE layer (set/resolve), non-crypto half.
//!
//! The content-addressed [`crate::blob::BlobStore`] is the immutable half: the hash IS the authorization,
//! no write-control needed. This module is the OTHER half — **mutable name → hash pointers**
//! (`system/compiler/latest → <hash>`, `session/<id>/scratch → <hash>`, `memory/… → <hash>`), the ONLY
//! part of the store that needs write control and thus the entire anti-hijack surface (§4c).
//!
//! **A mutable name is an append-only log of `set(name, hash)` entries** (§4c point 1 — "everything is a
//! log" applied to pointers). `set` appends; `resolve` returns the CURRENT value = the latest entry's
//! hash. The value-over-time IS the log, so a name carries audit (who-set-what-when — once producers ride
//! the envelope), rollback (the prior entries are still there), and — because resolution reads the log —
//! a resolver can *freeze* the resolved hash into its own history (§4c point 3: a hijacked pointer can
//! only mislead FUTURE opt-in resolvers, never retroactively alter a running session).
//!
//! **What's here (concierge/operator directive 2026-08-03 "do as much as possible without the crypto"):**
//! the append-only set/resolve mechanics + the write-authority PARSE seam. **What's deferred:** the
//! signature bytes on each `set` — the [`SetEntry`] carries an OPTIONAL `producer` today (unset in P0),
//! and the signature rides the EVENT envelope around the entry (§10), so signing layers in later with NO
//! migration: a signed store is these same entries + verified envelopes, not a different shape. The
//! write-authority DECISION (which prefix may a given writer set) is a Cedar prefix-grant on the existing
//! [`crate::authz::Authorize`] seam — see [`NameStore::authority_prefix_of`] for the pure prefix parse the
//! grant keys on (this replaces the deleted standalone `namespace.rs` module — authority is a Cedar
//! resource + prefix grant, not a separate component).

use crate::hash::Hash;
use std::collections::HashMap;

/// One `set(name, hash)` — an append to a mutable name's value-over-time log. The PAYLOAD (`name`, `hash`)
/// is what [`crate::event_ast::encode_name_set`] serializes for durability; `producer` is the §10 envelope
/// identity of who set it, **optional and unset in P0** (the operator deferred crypto — a signed store
/// populates + verifies it later with no change to this shape). Ordering in a name's log IS the
/// value-over-time; the last entry is the current value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SetEntry {
    /// The content hash this `set` points the name at.
    pub hash: Hash,
    /// Who set it (§10 producer identity), or `None` in P0 (crypto deferred). When signing lands this is
    /// the verified producer; an unauthorized `set` is refused at write time by the authority check, so a
    /// stored entry is by-construction from an authorized writer.
    pub producer: Option<String>,
}

impl SetEntry {
    /// An unsigned entry (the P0 shape: no producer identity yet — crypto deferred per operator).
    pub fn unsigned(hash: Hash) -> Self {
        SetEntry {
            hash,
            producer: None,
        }
    }
}

/// The §4c write-authority NAMESPACE a mutable name falls under, parsed from its PREFIX (§4c point 2).
/// This is the signature-INDEPENDENT classification the Cedar prefix-grant keys on ("which authority
/// governs this name"), ORTHOGONAL to "is a given writer actually that authority" (the grant check). The
/// prefixes mirror the §4c design: `system/` (release authority), `team/<team>/` (team membership),
/// `session/<id>/` (that session), `memory/` (the promotion authority); anything else is `Unscoped` and —
/// fail-closed — writable by no one (an unrecognized namespace is never open by default).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameAuthority {
    /// `system/…` — only a system/release grant may set (e.g. `system/compiler/latest`).
    System,
    /// `team/<team>/…` — team membership governs.
    Team,
    /// `session/<id>/…` — owned by that session (its own delegated identity, once delegation lands).
    Session,
    /// `memory/…` — the memory-promotion authority (§9 graduation gate).
    Memory,
    /// Any other prefix — NO known authority owns it, so no one may set it (fail-closed).
    Unscoped,
}

/// Why a `set`/`resolve` couldn't be served.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NameStoreError {
    /// `resolve` of a name that has never been `set` (no entries in its log).
    NoSuchName,
    /// A `set` to an `Unscoped` name — fail-closed: no authority governs the prefix, so it's unwritable
    /// (the authorizer is the real gate; this is the store's own last-line refusal for a nonsense prefix).
    UnscopedNameUnwritable,
    /// An [`NameStore::apply_effect`] whose (family, hash) shape is invalid: a `store/set` with no hash,
    /// a `store/resolve` carrying a hash, or a non-`store/*` family. Structural — a malformed store effect,
    /// never a panic.
    MalformedStoreEffect,
}

/// The mutable-name store: per-name append-only logs of [`SetEntry`]. In-memory for v0/tests; a durable
/// backend (its own global-store session, per §4c "the global store is itself a session") layers behind
/// the same set/resolve API later — the mechanics here are backend-independent. Single-writer-per-name is
/// the concurrency model (§4c: hot names partition by scope), so a plain map of append-logs suffices.
#[derive(Default)]
pub struct NameStore {
    /// name → its value-over-time log (append-only; last = current). A name with an empty/absent log has
    /// never been set (`resolve` → `NoSuchName`).
    names: HashMap<String, Vec<SetEntry>>,
}

impl NameStore {
    pub fn new() -> Self {
        NameStore {
            names: HashMap::new(),
        }
    }

    /// Append a `set(name, entry)` — the name now points at `entry.hash`; prior entries are retained (the
    /// value-over-time log, §4c point 1). Fail-closed on an `Unscoped` prefix (no authority governs it).
    /// NOTE: this is the STORE mechanic — the *authorization* (may THIS writer set THIS prefix) is the
    /// authorizer's job (a Cedar prefix-grant), checked BEFORE calling this; the `Unscoped` refusal here is
    /// the store's own backstop so a nonsense prefix can never accumulate entries even if authz is lax.
    pub fn set(&mut self, name: &str, entry: SetEntry) -> Result<(), NameStoreError> {
        if Self::authority_prefix_of(name) == NameAuthority::Unscoped {
            return Err(NameStoreError::UnscopedNameUnwritable);
        }
        self.names.entry(name.to_string()).or_default().push(entry);
        Ok(())
    }

    /// Resolve a name to its CURRENT hash = the latest `set` (§4c point 3: a resolver freezes THIS hash
    /// into its own log, so a later hijacking `set` can't retroactively change what this resolve returned).
    /// `Err(NoSuchName)` if the name was never set.
    pub fn resolve(&self, name: &str) -> Result<Hash, NameStoreError> {
        self.names
            .get(name)
            .and_then(|log| log.last())
            .map(|e| e.hash)
            .ok_or(NameStoreError::NoSuchName)
    }

    /// The full value-over-time log for a name (oldest → newest), for audit / rollback (§4c point 1). Empty
    /// slice for a never-set name.
    pub fn history(&self, name: &str) -> &[SetEntry] {
        self.names.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The §4c write-authority namespace for `name`, by its prefix — the pure classification a Cedar
    /// prefix-grant keys on. Signature-independent and total (every string maps to a variant; unknown →
    /// `Unscoped`, fail-closed).
    ///
    /// A prefix ONLY governs when there's a NON-EMPTY segment after it: `system/compiler/latest` is System,
    /// but a DEGENERATE `"system/"` (empty tail) — or `"team/"` (no team), `"session/"` (no id) — is
    /// `Unscoped` and thus UNWRITABLE. A malformed prefix must NOT be treated as a valid scoped authority
    /// (that would weaken the fail-closed anti-hijack posture — a name with no real scope segment naming no
    /// actual resource is a mistake, not a grantable target). Fail-closed: only a well-formed scoped name
    /// gets an authority.
    pub fn authority_prefix_of(name: &str) -> NameAuthority {
        // `governs(prefix)` = `name` starts with `prefix` AND has a non-empty segment after it.
        let governs = |prefix: &str| name.len() > prefix.len() && name.starts_with(prefix);
        if governs("system/") {
            NameAuthority::System
        } else if governs("team/") {
            NameAuthority::Team
        } else if governs("session/") {
            NameAuthority::Session
        } else if governs("memory/") {
            NameAuthority::Memory
        } else {
            NameAuthority::Unscoped
        }
    }

    /// Apply a `store/*` effect by its FAMILY (slice-3a vocab: [`crate::effect::effect_ct::STORE_SET`] /
    /// [`STORE_RESOLVE`](crate::effect::effect_ct::STORE_RESOLVE)) to this store, returning the outcome the
    /// drive loop folds back as an `EffectResult` (§4c slice 3b calls this from the store-family arm; the
    /// arm handles the AUTHZ gate + durable dispatch — this is the pure store SEMANTIC).
    ///
    /// - `store/set`: `hash` MUST be `Some` (the value to point `name` at) — append it as an
    ///   [`SetEntry::unsigned`] (producer/signature ride the event envelope, deferred). Returns the set
    ///   hash on success. `None` hash is a `MalformedStoreEffect` (a set needs a value).
    /// - `store/resolve`: `hash` MUST be `None` (a resolve carries no value) — returns the name's CURRENT
    ///   hash (§4c-pt3: the caller freezes it into its log). A `Some` hash is `MalformedStoreEffect`.
    /// - any other family: `MalformedStoreEffect` (not a store verb — the drive loop only routes `store/*`
    ///   here, so this is a defensive total-ness backstop).
    pub fn apply_effect(
        &mut self,
        family: &str,
        name: &str,
        hash: Option<Hash>,
    ) -> Result<StoreOutcome, NameStoreError> {
        use crate::effect::effect_ct;
        match family {
            effect_ct::STORE_SET => {
                let h = hash.ok_or(NameStoreError::MalformedStoreEffect)?;
                self.set(name, SetEntry::unsigned(h))?;
                Ok(StoreOutcome::Set(h))
            }
            effect_ct::STORE_RESOLVE => {
                if hash.is_some() {
                    return Err(NameStoreError::MalformedStoreEffect);
                }
                self.resolve(name).map(StoreOutcome::Resolved)
            }
            _ => Err(NameStoreError::MalformedStoreEffect),
        }
    }
}

/// The result of a [`NameStore::apply_effect`] — what the drive loop folds back as the `store/*` effect's
/// outcome. `Set` echoes the hash the name now points at; `Resolved` carries the frozen current hash.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StoreOutcome {
    /// A `store/set` succeeded: the name now points at this hash (its new latest value).
    Set(Hash),
    /// A `store/resolve` succeeded: the name's CURRENT hash (the caller freezes it into its log, §4c-pt3).
    Resolved(Hash),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_resolve_returns_the_latest_hash() {
        let mut s = NameStore::new();
        let v1 = Hash::of(b"compiler v1");
        let v2 = Hash::of(b"compiler v2");
        s.set("system/compiler/latest", SetEntry::unsigned(v1))
            .unwrap();
        assert_eq!(s.resolve("system/compiler/latest").unwrap(), v1);
        // A later set moves the pointer; resolve returns the NEW latest (value-over-time).
        s.set("system/compiler/latest", SetEntry::unsigned(v2))
            .unwrap();
        assert_eq!(s.resolve("system/compiler/latest").unwrap(), v2);
    }

    #[test]
    fn resolve_of_a_never_set_name_is_no_such_name() {
        let s = NameStore::new();
        assert_eq!(
            s.resolve("system/compiler/latest"),
            Err(NameStoreError::NoSuchName)
        );
    }

    #[test]
    fn history_is_the_full_append_only_value_log_oldest_to_newest() {
        let mut s = NameStore::new();
        let (a, b, c) = (Hash::of(b"a"), Hash::of(b"b"), Hash::of(b"c"));
        for h in [a, b, c] {
            s.set("session/abc/scratch", SetEntry::unsigned(h)).unwrap();
        }
        let hist: Vec<Hash> = s
            .history("session/abc/scratch")
            .iter()
            .map(|e| e.hash)
            .collect();
        assert_eq!(hist, vec![a, b, c], "prior sets are retained, in order");
        assert_eq!(s.resolve("session/abc/scratch").unwrap(), c);
    }

    #[test]
    fn unscoped_prefix_is_unwritable_fail_closed() {
        let mut s = NameStore::new();
        // No known authority governs a bare/unknown prefix → the store refuses the set (backstop; the
        // authorizer is the primary gate).
        assert_eq!(
            s.set("random-name", SetEntry::unsigned(Hash::of(b"x"))),
            Err(NameStoreError::UnscopedNameUnwritable)
        );
        assert_eq!(s.resolve("random-name"), Err(NameStoreError::NoSuchName));
    }

    #[test]
    fn authority_prefix_parse_covers_the_four_namespaces_and_fails_closed() {
        assert_eq!(
            NameStore::authority_prefix_of("system/compiler/latest"),
            NameAuthority::System
        );
        assert_eq!(
            NameStore::authority_prefix_of("team/rust-backend/x"),
            NameAuthority::Team
        );
        assert_eq!(
            NameStore::authority_prefix_of("session/abc/y"),
            NameAuthority::Session
        );
        assert_eq!(
            NameStore::authority_prefix_of("memory/graduated/z"),
            NameAuthority::Memory
        );
        // Unknown / bare prefixes fail closed.
        assert_eq!(
            NameStore::authority_prefix_of("system"),
            NameAuthority::Unscoped,
            "prefix must be `system/` — a bare `system` is not the namespace"
        );
        assert_eq!(
            NameStore::authority_prefix_of("evil/compiler"),
            NameAuthority::Unscoped
        );
    }

    #[test]
    fn degenerate_prefix_with_empty_segment_is_unscoped_not_writable() {
        // Security posture (github-liaison #1829): a prefix only governs with a NON-EMPTY segment after it.
        // A degenerate name — the bare prefix with an empty tail — must NOT classify as a scoped authority
        // (that would let a malformed name masquerade as a valid grantable target, weakening fail-closed).
        for degenerate in ["system/", "team/", "session/", "memory/"] {
            assert_eq!(
                NameStore::authority_prefix_of(degenerate),
                NameAuthority::Unscoped,
                "{degenerate:?} has no segment after the prefix → Unscoped (fail-closed)"
            );
        }
        // ...and an Unscoped name is unwritable end-to-end (the store refuses the set).
        let mut s = NameStore::new();
        assert_eq!(
            s.set("system/", SetEntry::unsigned(Hash::of(b"evil"))),
            Err(NameStoreError::UnscopedNameUnwritable),
            "a degenerate `system/` must not be writable"
        );
        // The MINIMAL well-formed name (one non-empty segment) IS governed.
        assert_eq!(
            NameStore::authority_prefix_of("system/x"),
            NameAuthority::System
        );
    }

    #[test]
    fn durable_entry_round_trips_through_the_name_set_codec() {
        // The store's entry payload IS what event_ast::encode_name_set serializes for a durable backend —
        // pin that the value survives the wire (slice-1 codec + slice-2 store agree).
        let h = Hash::of(b"the compiler wasm");
        let bytes = crate::event_ast::encode_name_set("system/compiler/latest", &h);
        let (name, hash) = crate::event_ast::decode_name_set(&bytes).unwrap();
        let mut s = NameStore::new();
        s.set(&name, SetEntry::unsigned(hash)).unwrap();
        assert_eq!(s.resolve("system/compiler/latest").unwrap(), h);
    }

    #[test]
    fn apply_effect_dispatches_store_set_and_store_resolve_by_family() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let h = Hash::of(b"compiler wasm");

        // store/set with a hash → appends + echoes the set hash.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "system/compiler/latest", Some(h)),
            Ok(StoreOutcome::Set(h))
        );
        // store/resolve with no hash → the current (frozen) hash.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_RESOLVE, "system/compiler/latest", None),
            Ok(StoreOutcome::Resolved(h))
        );
        // resolve of a never-set name surfaces NoSuchName (the drive loop folds it as an Err outcome).
        assert_eq!(
            s.apply_effect(effect_ct::STORE_RESOLVE, "system/never", None),
            Err(NameStoreError::NoSuchName)
        );
    }

    #[test]
    fn apply_effect_is_total_on_malformed_shapes() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        // store/set REQUIRES a hash (the value); None is malformed, not a panic.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "system/x", None),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/resolve must NOT carry a hash.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_RESOLVE, "system/x", Some(Hash::of(b"v"))),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // a non-store/* family is not a store verb (defensive backstop — the drive loop only routes store/*).
        assert_eq!(
            s.apply_effect("http", "system/x", Some(Hash::of(b"v"))),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/set to an Unscoped name still fails closed via the underlying set().
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "bare-name", Some(Hash::of(b"v"))),
            Err(NameStoreError::UnscopedNameUnwritable)
        );
    }
}

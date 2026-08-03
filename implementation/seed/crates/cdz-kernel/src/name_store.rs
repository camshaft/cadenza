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
    /// Idempotency keys of `store/set`s already applied to this store — the crash/re-drive dedup (§16c-S1/D).
    /// The kernel's recovery re-drives an open (dispatched-but-unsettled) store effect by its stable
    /// idempotency key; without dedup a `store/set` re-applied after a crash appends a DUPLICATE entry
    /// (`history` divergence). A key seen here means "already applied" → a re-drive is a no-op returning the
    /// same outcome. NOTE (durability boundary): this set is IN-MEMORY, so it dedups re-drives against a
    /// LIVE store (in-session replay, a re-emitted set); a durable backend (§4c "the store is itself a
    /// session") must persist these keys to dedup across a PROCESS crash — tracked as the durable-store slice.
    applied_set_keys: std::collections::HashSet<Hash>,
}

impl NameStore {
    /// The well-known mutable name for the current compiler build (the v0.2 seam: an agent `store/set`s
    /// this to publish a freshly-compiled `rcdzc→wasm` hash; another agent `store/resolve`s it, then
    /// blob-fetches + calls the compiled program). ONE source of truth for the pointer name across the
    /// kernel, cdz-agent-host's demo agents, and any real user — so it isn't a string duplicated per crate.
    /// Its `system/` prefix means only a `store/set` grant scoped to `system/` may repoint it (the §4c
    /// anti-hijack write-authority — a random agent can't swap the compiler out from under everyone).
    pub const COMPILER_LATEST: &'static str = "system/compiler/latest";

    pub fn new() -> Self {
        NameStore {
            names: HashMap::new(),
            applied_set_keys: std::collections::HashSet::new(),
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

    /// Reconstruct a `NameStore` by REPLAYING an ordered stream of `(name, hash)` `set` entries — the
    /// recovery primitive (§4c "the global store is itself a session": its state is derived from its
    /// set-event log, exactly as a session's KV is derived from its event log). A durable backend persists
    /// each authorized `store/set` as a `name-set` frame ([`crate::event_ast::encode_name_set`]); on boot it
    /// decodes them in order and hands the `(name, hash)` sequence here to rebuild the value-over-time.
    ///
    /// The replay goes through [`NameStore::set`], so the SAME invariants hold on recovery as on live write:
    /// an `Unscoped` name in the stream is rejected (fail-closed — a durable log should never carry one, but
    /// a corrupt/tampered stream can't smuggle a nonsense name into the recovered store). Returns the rebuilt
    /// store, or the first `NameStoreError` a bad entry produces (the caller decides recover-vs-halt, like the
    /// session-log `Corrupt` path). `applied_set_keys` starts EMPTY — recovered entries are historical, not
    /// in-flight, so there's no re-drive to dedup against them (the crash-recovery window is per-live-dispatch).
    pub fn replay_set_entries<'a>(
        entries: impl IntoIterator<Item = (&'a str, Hash)>,
    ) -> Result<NameStore, NameStoreError> {
        let mut store = NameStore::new();
        for (name, hash) in entries {
            store.set(name, SetEntry::unsigned(hash))?;
        }
        Ok(store)
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
    ///   hash on success. `None` hash is a `MalformedStoreEffect` (a set needs a value). IDEMPOTENT by
    ///   `idempotency_key` (§16c-S1/D): a set whose key was already applied is a NO-OP returning the same
    ///   `Set(hash)` — so the kernel's crash-recovery re-drive of an open store dispatch does NOT append a
    ///   duplicate entry.
    /// - `store/resolve`: `hash` MUST be `None` (a resolve carries no value) — returns the name's CURRENT
    ///   hash (§4c-pt3: the caller freezes it into its log). A `Some` hash is `MalformedStoreEffect`. A
    ///   resolve is a pure READ, so `idempotency_key` doesn't matter (re-driving it is naturally idempotent).
    /// - any other family: `MalformedStoreEffect` (not a store verb — the drive loop only routes `store/*`
    ///   here, so this is a defensive total-ness backstop).
    pub fn apply_effect(
        &mut self,
        family: &str,
        name: &str,
        hash: Option<Hash>,
        idempotency_key: Hash,
    ) -> Result<StoreOutcome, NameStoreError> {
        use crate::effect::effect_ct;
        match family {
            effect_ct::STORE_SET => {
                let h = hash.ok_or(NameStoreError::MalformedStoreEffect)?;
                // Dedup by idempotency key: a re-driven set (crash recovery, §16c-S1/D) is a NO-OP — it must
                // not append a duplicate entry. Validate the name FIRST (so a malformed/Unscoped set still
                // Errs on re-drive rather than silently "succeeding" as already-applied).
                if Self::authority_prefix_of(name) == NameAuthority::Unscoped {
                    return Err(NameStoreError::UnscopedNameUnwritable);
                }
                if self.applied_set_keys.insert(idempotency_key) {
                    // First time this key is applied → append.
                    self.set(name, SetEntry::unsigned(h))?;
                }
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

    /// Drop a `store/set`'s idempotency key from the dedup set once its `EffectResult` is RECORDED IN THE
    /// SESSION LOG (the effect is SETTLED in this session — its dedup entry is no longer needed). Note the
    /// boundary: "recorded" here means the in-memory session log; DURABILITY is separate (the write-through
    /// append may latch a persist error), so this is NOT gated on a fsync'd EffectResult. That's still
    /// re-drive-safe — the caller's prune site documents why (recovery re-attaches a FRESH store; an
    /// in-process re-drive is blocked by the settled set) — so pruning after the in-memory record can't
    /// re-open the crash-recovery re-apply. BOUNDS `applied_set_keys` to the in-flight window instead of
    /// letting it grow monotonically with every set (an unbounded-memory / DoS vector otherwise). Idempotent
    /// + total: forgetting an absent key (a resolve's, or an already-forgotten one) is a harmless no-op.
    pub fn forget_applied_key(&mut self, idempotency_key: &Hash) {
        self.applied_set_keys.remove(idempotency_key);
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
    fn compiler_latest_const_is_a_system_scoped_well_formed_name() {
        // The v0.2 compiler-pointer const: pin its value (one source of truth) + that it's System-scoped
        // (so a store/set is write-gated by a system/ grant — the anti-hijack property the demo relies on).
        assert_eq!(NameStore::COMPILER_LATEST, "system/compiler/latest");
        assert_eq!(
            NameStore::authority_prefix_of(NameStore::COMPILER_LATEST),
            NameAuthority::System,
            "the compiler pointer must be System-scoped (write-gated)"
        );
        // It's a well-formed (writable) name — a set to it is accepted (given the authz grant, which the
        // store's own backstop doesn't check; here just the store mechanic).
        let mut s = NameStore::new();
        s.set(
            NameStore::COMPILER_LATEST,
            SetEntry::unsigned(Hash::of(b"wasm")),
        )
        .unwrap();
        assert_eq!(
            s.resolve(NameStore::COMPILER_LATEST).unwrap(),
            Hash::of(b"wasm")
        );
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
    fn replay_set_entries_rebuilds_value_over_time_and_resolves_the_latest() {
        // §4c recovery: a durable backend persists set-events; replaying them (oldest→newest) reconstructs
        // the store's value-over-time, and resolve returns the LATEST per name — like KV rebuilt from the log.
        let (v1, v2) = (Hash::of(b"compiler v1"), Hash::of(b"compiler v2"));
        let sess = Hash::of(b"scratch");
        let rebuilt = NameStore::replay_set_entries([
            ("system/compiler/latest", v1),
            ("session/abc/scratch", sess),
            ("system/compiler/latest", v2), // a later set moves the pointer
        ])
        .expect("replay a well-formed set-event stream");
        assert_eq!(rebuilt.resolve("system/compiler/latest").unwrap(), v2);
        assert_eq!(rebuilt.resolve("session/abc/scratch").unwrap(), sess);
        // The full value-over-time is preserved (audit/rollback), not just the latest.
        let hist: Vec<Hash> = rebuilt
            .history("system/compiler/latest")
            .iter()
            .map(|e| e.hash)
            .collect();
        assert_eq!(hist, vec![v1, v2]);
        // applied_set_keys starts EMPTY on recovery — replayed entries are historical, not in-flight, so
        // there's nothing to dedup against them. PIN it directly (child module → private field): a
        // regression that left the dedup set populated after recovery would otherwise pass green and
        // undercut the #1852 unbounded-set fix's guarantee (liaison #1865).
        assert!(
            rebuilt.applied_set_keys.is_empty(),
            "recovery must NOT populate the dedup set — replayed entries are historical, not in-flight"
        );
    }

    #[test]
    fn replay_set_entries_fails_closed_on_an_unscoped_name_in_the_stream() {
        // A corrupt/tampered durable stream can't smuggle a nonsense (Unscoped) name into the recovered
        // store — replay goes through set(), which rejects it (same fail-closed invariant as live write).
        // (match on the Err — NameStore has no Debug, so avoid unwrap_err's Ok-side formatting.)
        match NameStore::replay_set_entries([("bare-name", Hash::of(b"x"))]) {
            Err(e) => assert_eq!(e, NameStoreError::UnscopedNameUnwritable),
            Ok(_) => panic!("an Unscoped name in the stream must fail-closed on replay"),
        }
    }

    #[test]
    fn apply_effect_dispatches_store_set_and_store_resolve_by_family() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let h = Hash::of(b"compiler wasm");
        let k = Hash::of(b"key-1");

        // store/set with a hash → appends + echoes the set hash.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "system/compiler/latest", Some(h), k),
            Ok(StoreOutcome::Set(h))
        );
        // store/resolve with no hash → the current (frozen) hash.
        assert_eq!(
            s.apply_effect(
                effect_ct::STORE_RESOLVE,
                "system/compiler/latest",
                None,
                Hash::of(b"key-2")
            ),
            Ok(StoreOutcome::Resolved(h))
        );
        // resolve of a never-set name surfaces NoSuchName (the drive loop folds it as an Err outcome).
        assert_eq!(
            s.apply_effect(
                effect_ct::STORE_RESOLVE,
                "system/never",
                None,
                Hash::of(b"key-3")
            ),
            Err(NameStoreError::NoSuchName)
        );
    }

    #[test]
    fn apply_effect_store_set_is_idempotent_by_key_no_duplicate_entry() {
        // §16c-S1/D: re-driving the SAME store/set (same idempotency key) after a crash must NOT append a
        // duplicate entry — the dedup makes the re-drive a no-op returning the same Set outcome.
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let h = Hash::of(b"compiler wasm v1");
        let key = Hash::of(b"dispatch-key-A");
        let name = "system/compiler/latest";

        // First apply appends.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, name, Some(h), key),
            Ok(StoreOutcome::Set(h))
        );
        assert_eq!(s.history(name).len(), 1);
        // RE-DRIVE with the SAME key → no-op (same outcome), history unchanged (no duplicate).
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, name, Some(h), key),
            Ok(StoreOutcome::Set(h))
        );
        assert_eq!(
            s.history(name).len(),
            1,
            "re-drive by same key must NOT duplicate"
        );
        // A genuinely NEW set (different key) DOES append (the value-over-time log advances).
        let h2 = Hash::of(b"compiler wasm v2");
        assert_eq!(
            s.apply_effect(
                effect_ct::STORE_SET,
                name,
                Some(h2),
                Hash::of(b"dispatch-key-B")
            ),
            Ok(StoreOutcome::Set(h2))
        );
        assert_eq!(s.history(name).len(), 2);
        assert_eq!(s.resolve(name).unwrap(), h2);
    }

    #[test]
    fn forget_applied_key_bounds_the_dedup_set_and_is_a_total_noop_on_absent() {
        // liaison #1852: applied_set_keys must be BOUNDED — pruned once the effect settles, not grown
        // forever. After forget_applied_key, the SAME key is no longer deduped (proving it was dropped);
        // and forgetting an absent key (a resolve's, or an already-forgotten one) is a harmless no-op.
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let name = "system/compiler/latest";
        let key = Hash::of(b"dispatch-key-A");
        let h = Hash::of(b"v1");

        s.apply_effect(effect_ct::STORE_SET, name, Some(h), key)
            .unwrap();
        assert_eq!(s.history(name).len(), 1);
        // Settle → prune the key.
        s.forget_applied_key(&key);
        // The SAME key now applies AGAIN (no longer deduped) — confirms it was dropped from the set (bounded).
        // (In the kernel this can't double-apply because the EffectId — hence the key — is unique per
        // dispatch; the test drives apply_effect directly to observe the prune.)
        s.apply_effect(effect_ct::STORE_SET, name, Some(h), key)
            .unwrap();
        assert_eq!(
            s.history(name).len(),
            2,
            "after forget, the key is no longer deduped (was pruned → bounded set)"
        );
        // Forgetting an absent key is a total no-op.
        s.forget_applied_key(&Hash::of(b"never-inserted"));
        s.forget_applied_key(&key); // already forgotten (well, re-applied) — still fine
    }

    #[test]
    fn apply_effect_is_total_on_malformed_shapes() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let k = Hash::of(b"k");
        // store/set REQUIRES a hash (the value); None is malformed, not a panic.
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "system/x", None, k),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/resolve must NOT carry a hash.
        assert_eq!(
            s.apply_effect(
                effect_ct::STORE_RESOLVE,
                "system/x",
                Some(Hash::of(b"v")),
                k
            ),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // a non-store/* family is not a store verb (defensive backstop — the drive loop only routes store/*).
        assert_eq!(
            s.apply_effect("http", "system/x", Some(Hash::of(b"v")), k),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/set to an Unscoped name still fails closed via the underlying set().
        assert_eq!(
            s.apply_effect(effect_ct::STORE_SET, "bare-name", Some(Hash::of(b"v")), k),
            Err(NameStoreError::UnscopedNameUnwritable)
        );
    }
}

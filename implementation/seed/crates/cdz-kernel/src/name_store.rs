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

/// One add/remove operation in a GROUP name's OR-set log (§4c session-directory I1). A group name (e.g.
/// `session/room/lobby`) is multi-writer (each member adds ITSELF), so its membership is a CRDT — an OR-set:
/// current members = fold the log with **add-wins** semantics (a member is present iff it has ≥1 `add` tag
/// not covered by a `remove`). This is the ONLY multi-writer-safe model — the single-value prefix-append
/// merge ([`NameStore::merge_appends_from`]) drops a concurrent writer's entry, an OR-set's tag-union does
/// not (D2, the load-bearing reason).
///
/// `tag` makes each add UNIQUE — an `(origin, seq)` pair where `origin` is the emitting session's identity
/// (its genesis hash = SessionId; converges with the naming work) and `seq` its local per-op counter. The
/// tag is what makes the merge idempotent + a `remove` precise (a remove carries the tags it observed, so a
/// concurrent re-add with a FRESH tag survives — add-wins). The tag is DETERMINISTIC (never random/clock) so
/// replay reproduces it. This is the pure in-memory model; the durable add/remove-frame codec is I2.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MemberOp {
    /// `true` = add this member, `false` = remove the add-tags this op observed.
    pub add: bool,
    /// The member hash (a session's genesis hash = its SessionId, for a session group).
    pub member: Hash,
    /// The unique add-tag `(origin, seq)`: origin = emitting session's identity, seq = its local op counter.
    /// For a `remove`, this identifies the SPECIFIC add-tag being retracted (observed-remove: EXACTLY the
    /// add carrying this tag is cleared — not a range; a later re-add with a DIFFERENT tag survives, which
    /// is the add-wins property).
    pub tag: (Hash, u64),
}

impl MemberOp {
    /// An `add(member)` tagged by `(origin, seq)` — the join op. `origin` is the adding session's identity.
    pub fn add(member: Hash, origin: Hash, seq: u64) -> Self {
        MemberOp {
            add: true,
            member,
            tag: (origin, seq),
        }
    }

    /// A `remove(member)` carrying the tag it observed — the leave/evict op (add-wins: retracts EXACTLY the
    /// add carrying this tag; a concurrent re-add with a fresh tag is NOT retracted).
    pub fn remove(member: Hash, origin: Hash, seq: u64) -> Self {
        MemberOp {
            add: false,
            member,
            tag: (origin, seq),
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
    /// `effect/<family>` — the USERSPACE-EFFECT registration authority (userspace-effects I1): the name
    /// `effect/<family>` points at a handler session's `SessionId` (= genesis hash), so a Cedar-granted
    /// `store/set effect/<family>` is how a session CLAIMS/repoints an effect family to itself. This is the
    /// anti-hijack surface for userspace effects (mirrors `system/`: only a grant over the exact
    /// `effect/<family>` may repoint it, else a rogue session could steal another's effect family).
    Effect,
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
    /// A [`NameStore::from_snapshot_bytes`] blob was malformed: a short length prefix, a truncated/garbage
    /// frame, or a frame that isn't a valid `name-set`. Total — a corrupt snapshot fails cleanly, not a panic.
    MalformedSnapshot,
    /// A name was used with the WRONG kind (§4c session-directory D7 mode-guard): a group `add`/`remove` (or
    /// `resolve_all`) on a name that's a SINGLE-VALUE pointer, or a `set`/`resolve` on a GROUP name. A name's
    /// kind is fixed by its first write (pointer XOR group); a mismatching verb is refused, not coerced —
    /// fail-closed, so a `set` can't silently clobber a group (or a `resolve` mis-read one) into nonsense.
    NameModeMismatch,
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
    /// GROUP name → its OR-set add/remove log (§4c session-directory I1). SEPARATE from `names` so the
    /// single-value pointer path (`resolve`/`to_set_entries`/`snapshot_bytes`/`merge_appends_from`) stays
    /// BYTE-IDENTICAL to before — a name lives in `names` XOR `groups`, never both (structural enforcement
    /// of "a name can never switch kinds": `add_member` refuses a name already in `names`, `set` refuses one
    /// in `groups`). Current membership = fold the log ([`NameStore::resolve_all`]); the multi-writer merge
    /// is the CRDT tag-union ([`NameStore::merge_appends_from`] extended to unite this map too).
    groups: HashMap<String, Vec<MemberOp>>,
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

    /// The well-known mutable name for the CURRENT authorization policy component (the §20b seam:
    /// policy-referenced-by-mutable-name). An admin `store/set`s this to a Cedar-policy component's
    /// blob-hash; the host `store/resolve`s it, blob-fetches the bytes, and rebuilds the session's
    /// [`ComponentAuthorizer`](crate::wasm_host::ComponentAuthorizer) — so swapping the live policy is just
    /// a `store/set` to this name (which is ALSO an I6 `capabilities-changed` trigger: a policy swap can move
    /// a session's grant-states, so the host re-runs `push_capabilities_changed` after the swap). ONE source
    /// of truth for the pointer name across the kernel, the host, and any admin — not a string duplicated
    /// per crate. Its `system/` prefix means only a `store/set` grant scoped to `system/` may repoint it (the
    /// §4c anti-hijack write-authority — a random agent can't swap the policy that governs everyone out from
    /// under them; the thing that DECIDES authorization is itself write-gated by that authorization).
    pub const POLICY_CURRENT: &'static str = "system/policy/current";

    pub fn new() -> Self {
        NameStore {
            names: HashMap::new(),
            groups: HashMap::new(),
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
        if self.groups.contains_key(name) {
            // Already an OR-set group → can't also be a single-value pointer (no kind-switch, D7).
            return Err(NameStoreError::NameModeMismatch);
        }
        self.names.entry(name.to_string()).or_default().push(entry);
        Ok(())
    }

    /// Resolve a name to its CURRENT hash = the latest `set` (§4c point 3: a resolver freezes THIS hash
    /// into its own log, so a later hijacking `set` can't retroactively change what this resolve returned).
    /// `Err(NoSuchName)` if the name was never set; `Err(NameModeMismatch)` if it's a GROUP name (use
    /// [`resolve_all`](Self::resolve_all)) — the mode-guard mirrors [`set`](Self::set), so a group can't be
    /// silently mis-read as a single-value pointer (D7).
    pub fn resolve(&self, name: &str) -> Result<Hash, NameStoreError> {
        if self.groups.contains_key(name) {
            return Err(NameStoreError::NameModeMismatch);
        }
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

    /// Append an `add`/`remove` op to a GROUP name's OR-set log (§4c session-directory I1). Fail-closed on an
    /// `Unscoped` prefix (same backstop as [`set`](Self::set)); refuses a name already used as a SINGLE-VALUE
    /// pointer ([`NameStoreError::NameModeMismatch`]) — a name is a pointer XOR a group, never both, and the
    /// kind is fixed by its FIRST write (mode-guard, D7). Authorization (may THIS writer touch THIS prefix)
    /// is the authorizer's job, checked BEFORE this; the mode/Unscoped refusals are the store's backstops.
    pub fn add_op(&mut self, name: &str, op: MemberOp) -> Result<(), NameStoreError> {
        if Self::authority_prefix_of(name) == NameAuthority::Unscoped {
            return Err(NameStoreError::UnscopedNameUnwritable);
        }
        if self.names.contains_key(name) {
            // Already a single-value pointer → can't also be a group (no kind-switch).
            return Err(NameStoreError::NameModeMismatch);
        }
        self.groups.entry(name.to_string()).or_default().push(op);
        Ok(())
    }

    /// Resolve a GROUP name's CURRENT membership — fold its OR-set log with ADD-WINS semantics: a member is
    /// present iff it has an `add` tag NOT covered by a `remove` of the same tag (§4c D1). Returns a
    /// `BTreeSet<Hash>` (deterministic ascending-hash order → a frozen member set is byte-stable + the
    /// multicast fan-out order is deterministic). `Err(NameModeMismatch)` for a SINGLE-VALUE (pointer) name —
    /// resolve-all of a pointer is a misuse (use [`resolve`](Self::resolve)); `Err(NoSuchName)` if the name
    /// has no group log at all.
    pub fn resolve_all(
        &self,
        name: &str,
    ) -> Result<std::collections::BTreeSet<Hash>, NameStoreError> {
        if self.names.contains_key(name) {
            return Err(NameStoreError::NameModeMismatch);
        }
        let log = self.groups.get(name).ok_or(NameStoreError::NoSuchName)?;
        // Add-wins fold: collect the set of removed tags, then a member is present iff it has an add-tag not
        // in that removed set. Tags are unique per add, so this is order-independent (CRDT).
        let removed: std::collections::HashSet<&(Hash, u64)> =
            log.iter().filter(|o| !o.add).map(|o| &o.tag).collect();
        let mut members = std::collections::BTreeSet::new();
        for op in log.iter().filter(|o| o.add) {
            if !removed.contains(&op.tag) {
                members.insert(op.member);
            }
        }
        Ok(members)
    }

    /// The full OR-set op log for a group name (oldest → newest), for audit (§4c point 1). Empty for a
    /// never-touched / non-group name.
    pub fn group_history(&self, name: &str) -> &[MemberOp] {
        self.groups.get(name).map(|v| v.as_slice()).unwrap_or(&[])
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

    /// Export this store's full set-event stream — the DUAL of [`NameStore::replay_set_entries`], the other
    /// half of the durable snapshot/restore pair (like [`crate::kv::Kv::encode`]/`decode` for the session
    /// KV). A durable backend calls this to serialize the store (each `(name, hash)` becomes a `name-set`
    /// frame via [`crate::event_ast::encode_name_set`]); `replay_set_entries` on that stream reconstructs
    /// an identical store.
    ///
    /// DETERMINISTIC order: names in ascending byte order, and within each name its value-over-time
    /// (oldest→newest, the append order). So the exported stream is byte-STABLE (a snapshot content-addresses
    /// the same regardless of insertion history — the property the session KV snapshot relies on), and
    /// replaying it re-establishes each name's latest correctly. `producer` is dropped for now (unset in P0;
    /// when signing lands the export carries the signed envelope, not just the payload). Full history is
    /// exported (audit/rollback preserved), not just the latest per name.
    pub fn to_set_entries(&self) -> Vec<(String, Hash)> {
        let mut names: Vec<&String> = self.names.keys().collect();
        names.sort_unstable();
        let mut out = Vec::new();
        for name in names {
            for entry in &self.names[name] {
                out.push((name.clone(), entry.hash));
            }
        }
        out
    }

    /// Export every GROUP name's OR-set ops (§4c session-directory I2), name-sorted, ops in log order — the
    /// group counterpart to [`to_set_entries`](Self::to_set_entries). This is what [`snapshot_bytes`](Self::snapshot_bytes)
    /// serializes (as `member-op` frames) so a store COPY / snapshot preserves the GROUP kind + membership,
    /// not just single-value pointers (the #2414-c3 "copies drop the group kind" gap — a copy seeded only
    /// from `to_set_entries` would lose groups entirely; this + the snapshot round-trip closes it). Byte-STABLE
    /// (name-sorted; each group's log order is its append order, already deterministic).
    pub fn to_group_ops(&self) -> Vec<(String, MemberOp)> {
        let mut names: Vec<&String> = self.groups.keys().collect();
        names.sort_unstable();
        let mut out = Vec::new();
        for name in names {
            for op in &self.groups[name] {
                out.push((name.clone(), op.clone()));
            }
        }
        out
    }

    /// Serialize the whole store to a SINGLE durable-snapshot blob — the §4c cascade-free durability path:
    /// a backend `blob.put(store.snapshot_bytes())`s this and `from_snapshot_bytes(blob.get(..))`s it back on
    /// recovery (BlobStore is content-addressed + async, so the snapshot self-verifies by hash). The blob is
    /// a stream of `u32-LE-length`-framed frames (the SAME framing `log_store` uses — one shared discipline),
    /// each self-describing by its AST head: FIRST every single-value pointer as a
    /// [`crate::event_ast::encode_name_set`] `name-set` frame ([`NameStore::to_set_entries`] order), THEN every
    /// GROUP op as an [`crate::event_ast::encode_member_op`] `member-op` frame ([`NameStore::to_group_ops`]
    /// order) — so a snapshot carries BOTH the pointer values AND the OR-set group membership+kind (restore
    /// routes per-frame on the head). Byte-STABLE (both streams name-sorted; group log order is append order),
    /// so the snapshot content-addresses identically regardless of insertion history.
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut push_frame = |frame: Vec<u8>| {
            // u32-LE length prefix, matching the log-store framing (a frame is at most a small name+hash).
            buf.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            buf.extend_from_slice(&frame);
        };
        // Single-value pointer names first (name-set frames), then GROUP names (member-op frames). Each
        // frame is self-describing by its AST head (`name-set` vs `member-op`), so from_snapshot_bytes routes
        // per-frame — the two kinds can interleave freely; this order is just deterministic (both sorted).
        for (name, hash) in self.to_set_entries() {
            push_frame(crate::event_ast::encode_name_set(&name, &hash));
        }
        for (name, op) in self.to_group_ops() {
            push_frame(crate::event_ast::encode_member_op(
                &name, op.add, &op.member, &op.tag,
            ));
        }
        buf
    }

    /// Reconstruct a store from a [`NameStore::snapshot_bytes`] blob — the restore half. Reads the framed
    /// `name-set` stream and replays it via [`NameStore::replay_set_entries`], so the SAME fail-closed
    /// invariants hold (an Unscoped name in a corrupt/tampered snapshot is rejected). Total: any malformed
    /// framing (short length prefix, truncated/garbage frame) is a clean `Err`, never a panic — the bytes
    /// come from a store/CAS, so a corrupt snapshot must fail cleanly (the caller falls back / halts), like
    /// the session-log `Corrupt` path.
    pub fn from_snapshot_bytes(bytes: &[u8]) -> Result<NameStore, NameStoreError> {
        let mut store = NameStore::new();
        let mut pos = 0usize;
        while pos < bytes.len() {
            let len_end = pos
                .checked_add(4)
                .ok_or(NameStoreError::MalformedSnapshot)?;
            let len_bytes = bytes
                .get(pos..len_end)
                .ok_or(NameStoreError::MalformedSnapshot)?;
            let len = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]])
                as usize;
            let frame_end = len_end
                .checked_add(len)
                .ok_or(NameStoreError::MalformedSnapshot)?;
            let frame = bytes
                .get(len_end..frame_end)
                .ok_or(NameStoreError::MalformedSnapshot)?;
            // Decode the frame ONCE + route on its self-describing AST head (name-set → set, member-op →
            // add_op) — NOT try-one-then-fall-back, which re-parses every pointer frame (github-liaison
            // #2424 c2). A frame that's neither head = MalformedSnapshot. Both kinds go through the
            // authority-checked write path (set/add_op) with the underlying error PROPAGATED VERBATIM (a
            // tampered snapshot naming an Unscoped/mode-mismatched name fails on the AUTHORITY check, not as
            // MalformedSnapshot — only a bad FRAME is malformed).
            match crate::event_ast::decode_store_frame(frame)
                .map_err(|_| NameStoreError::MalformedSnapshot)?
            {
                crate::event_ast::StoreFrame::NameSet { name, hash } => {
                    store.set(&name, SetEntry::unsigned(hash))?;
                }
                crate::event_ast::StoreFrame::MemberOp {
                    name,
                    add,
                    member,
                    tag,
                } => {
                    let op = if add {
                        MemberOp::add(member, tag.0, tag.1)
                    } else {
                        MemberOp::remove(member, tag.0, tag.1)
                    };
                    store.add_op(&name, op)?;
                }
            }
            pos = frame_end;
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
        } else if governs(crate::effect::effect_ct::EFFECT_REGISTRY_PREFIX) {
            // `effect/<family>` — the userspace-effect registration authority (I1). Only a session
            // Cedar-granted `store/set effect/<family>` may claim/repoint the handler pointer.
            NameAuthority::Effect
        } else {
            NameAuthority::Unscoped
        }
    }

    /// Resolve `effect/<family>` → the registered handler's [`SessionId`] (genesis hash), or `None` if no
    /// handler has claimed that family (userspace-effects I1). A thin typed wrapper over [`resolve`](Self::resolve)
    /// for the `effect/` registration namespace: the host's delegating executor calls this to decide
    /// `handles_family` ("is a handler registered?") and to route a forwarded request. `family` is the bare
    /// family string (e.g. `weather`); this prepends the `effect/` prefix. Returns `None` for an
    /// unregistered family (never an error — an absent handler is a normal "not a userspace effect" answer,
    /// the caller falls through to the built-in partitions).
    pub fn resolve_effect_handler(&self, family: &str) -> Option<Hash> {
        let name = format!(
            "{}{}",
            crate::effect::effect_ct::EFFECT_REGISTRY_PREFIX,
            family
        );
        self.resolve(&name).ok()
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

    /// Apply a GROUP `store/*` effect by its FAMILY (§4c session-directory I3: [`STORE_ADD`](crate::effect::effect_ct::STORE_ADD)
    /// / [`STORE_REMOVE`](crate::effect::effect_ct::STORE_REMOVE) / [`STORE_RESOLVE_ALL`](crate::effect::effect_ct::STORE_RESOLVE_ALL))
    /// — the OR-set counterpart to [`apply_effect`](Self::apply_effect). The drive loop's store arm routes here
    /// on [`is_group_store_family`](crate::effect::effect_ct::is_group_store_family) after the SEC-F1 authorize
    /// gate; this is the pure store SEMANTIC (mode/Unscoped/idempotency backstops, no authz).
    ///
    /// - `store/add` / `store/remove`: `op` MUST be `Some` (the member+tag payload) — append it via
    ///   [`add_op`](Self::add_op), which enforces the pointer-XOR-group mode-guard + Unscoped fail-close. Returns
    ///   [`StoreOutcome::GroupOpApplied`]. IDEMPOTENT by `idempotency_key` (§16c-S1/D): a re-driven op (crash
    ///   recovery) is a NO-OP returning the same success — no duplicate log entry. (Note the OR-set fold is
    ///   ALSO idempotent by the op's unique `tag`, so a duplicate would be harmless to membership; the key dedup
    ///   additionally keeps the LOG itself append-exact, matching the pointer path's invariant.) A `None` op is
    ///   a [`MalformedStoreEffect`](NameStoreError::MalformedStoreEffect) (an add/remove needs a member).
    /// - `store/resolve-all`: `op` MUST be `None` (a resolve carries no payload) — returns the current member
    ///   set via [`resolve_all`](Self::resolve_all) (mode-guard: a pointer name is `NameModeMismatch`). A pure
    ///   READ, so `idempotency_key` doesn't matter. A `Some` op is `MalformedStoreEffect`.
    /// - any other family: `MalformedStoreEffect` (defensive total-ness backstop — the drive loop only routes a
    ///   group `store/*` family here).
    pub fn apply_group_effect(
        &mut self,
        family: &str,
        name: &str,
        op: Option<MemberOp>,
        idempotency_key: Hash,
    ) -> Result<StoreOutcome, NameStoreError> {
        use crate::effect::effect_ct;
        match family {
            effect_ct::STORE_ADD | effect_ct::STORE_REMOVE => {
                let op = op.ok_or(NameStoreError::MalformedStoreEffect)?;
                // The op's `add` flag MUST agree with the family verb — a `store/add` carrying a remove-op (or
                // vice-versa) is a malformed effect, not a silent coercion (the family is what authz gated).
                let expect_add = family == effect_ct::STORE_ADD;
                if op.add != expect_add {
                    return Err(NameStoreError::MalformedStoreEffect);
                }
                // Dedup by idempotency key BEFORE add_op: the check must precede the append (add_op is
                // unconditional-append, so re-applying would DOUBLE-log). The mode/Unscoped guards live inside
                // add_op, so a first-application failure propagates via `?`; re-drive safety comes from the
                // architecture (recovery re-attaches a FRESH store → empty keys; an in-process re-drive of a
                // settled id is blocked by record_result), the SAME rationale the pointer `apply_effect` relies
                // on — not from a pre-check here.
                if self.applied_set_keys.insert(idempotency_key) {
                    self.add_op(name, op)?;
                }
                Ok(StoreOutcome::GroupOpApplied)
            }
            effect_ct::STORE_RESOLVE_ALL => {
                if op.is_some() {
                    return Err(NameStoreError::MalformedStoreEffect);
                }
                self.resolve_all(name).map(StoreOutcome::Members)
            }
            _ => Err(NameStoreError::MalformedStoreEffect),
        }
    }

    /// Drop a `store/set`'s idempotency key from the dedup set once its `EffectResult` is APPENDED to the
    /// in-memory session log (the effect is SETTLED in this session — its dedup entry is no longer needed).
    /// Boundary, in the log-store's v0 durability terms ([`crate::log_store::LogStore::append`]): "appended"
    /// here is the in-memory record; ON-DISK durability is the SEPARATE tier-B write-through — the v0
    /// contract is "append + flush to the OS" and a failure LATCHES `persist_error` (the event did not reach
    /// stable storage). So the prune is NOT gated on the EffectResult reaching stable storage. That's still
    /// re-drive-safe — the caller's prune site documents why (recovery re-attaches a FRESH store; an
    /// in-process re-drive is blocked by the settled set) — so pruning after the in-memory record can't
    /// re-open the crash-recovery re-apply. BOUNDS `applied_set_keys` to the in-flight window instead of
    /// letting it grow monotonically with every set (an unbounded-memory / DoS vector otherwise). Idempotent
    /// + total: forgetting an absent key (a resolve's, or an already-forgotten one) is a harmless no-op.
    pub fn forget_applied_key(&mut self, idempotency_key: &Hash) {
        self.applied_set_keys.remove(idempotency_key);
    }

    /// Merge the NEW appends from `other` into `self` — the single-host-owned shared-store primitive (§4c
    /// v0.3): the host holds ONE canonical `NameStore`, hands each session a fresh copy (a
    /// [`replay_set_entries`](NameStore::replay_set_entries) of `to_set_entries`), lets the session drive a
    /// turn (its `store/set`s append to ITS copy), then folds those new writes back here. This is what makes
    /// a pointer published by session A visible to session B WITHOUT the explicit export/replay bridge — the
    /// host reconciles into canonical after each turn, and hands B a copy that already has A's write.
    ///
    /// For each name in `other`, if `other`'s value-over-time log is LONGER than `self`'s, the extra tail
    /// entries are appended here (they're the session's new `set`s this turn). This is correct — and
    /// conflict-free — under the store's concurrency model (§4c: single-writer-per-name, hot names partition
    /// by scope): a session only appends to names its grant authorizes, so two sessions never race the same
    /// name, and each name's log here is a PREFIX of the copy the writing session extended. A name `other`
    /// has fewer/equal entries for contributes nothing (no rewind — append-only). Entries are appended via
    /// the raw log (not [`set`](NameStore::set)) because they were ALREADY authority-checked when the session
    /// wrote them; re-checking would be redundant, and merge-back is a host-internal reconcile, not a write.
    /// `applied_set_keys` is untouched: those are per-live-dispatch crash-dedup keys, not shared state.
    pub fn merge_appends_from(&mut self, other: &NameStore) {
        // Single-value pointer names: the EXACT prior path, BYTE-IDENTICAL (longer-log-wins tail-append —
        // correct because a pointer name is single-writer, so the logs are prefixes of each other). A name
        // that's a GROUP on either side is NOT a pointer — skip it here (the XOR-invariant guard, c3):
        // never let a merge admit a name into BOTH maps.
        for (name, other_log) in &other.names {
            if self.groups.contains_key(name) || other.groups.contains_key(name) {
                continue; // pointer-XOR-group: this name is a group somewhere → not a pointer merge
            }
            let mine = self.names.entry(name.clone()).or_default();
            if other_log.len() > mine.len() {
                mine.extend_from_slice(&other_log[mine.len()..]);
            }
        }
        // GROUP names: the OR-set CRDT merge (§4c D2) — a group is MULTI-writer, so the logs are NOT prefixes
        // of each other and the prefix-append above would DROP a concurrent writer's ops. Instead union by
        // TAG: append every op from `other` not already present. Commutative + idempotent + order-independent
        // (the tag makes each add unique + a remove precise), so a name's membership converges regardless of
        // merge order. A name that's a POINTER on either side is skipped (XOR-invariant, c3). Dedup via a
        // HashSet of the ops already present (O(n) merge, not O(n²) repeated Vec::contains).
        for (name, other_ops) in &other.groups {
            if self.names.contains_key(name) || other.names.contains_key(name) {
                continue; // pointer-XOR-group: this name is a pointer somewhere → not a group merge
            }
            let mine = self.groups.entry(name.clone()).or_default();
            let mut seen: std::collections::HashSet<&MemberOp> = mine.iter().collect();
            // Collect the ops to append first (can't borrow `mine` mutably while `seen` borrows it).
            let to_add: Vec<MemberOp> = other_ops
                .iter()
                .filter(|op| seen.insert(op))
                .cloned()
                .collect();
            mine.extend(to_add);
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
    /// A `store/add` / `store/remove` succeeded: the group op was appended to the name's OR-set log (§4c
    /// session-directory I3). Empty-success — like `Set`, the op's payload already carried the member+tag,
    /// so there is nothing to echo back; the reducer keyed its continuation by EffectId.
    GroupOpApplied,
    /// A `store/resolve-all` succeeded: the group name's CURRENT membership, add-wins folded into a
    /// deterministic ascending-hash set (§4c D1). The caller freezes it into its log (the multicast §8
    /// fan-out iterates this frozen set — byte-stable order).
    Members(std::collections::BTreeSet<Hash>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_resolve_returns_the_latest_hash() {
        let mut s = NameStore::new();
        let v1 = Hash::of(b"compiler v1");
        let v2 = Hash::of(b"compiler v2");
        s.set(NameStore::COMPILER_LATEST, SetEntry::unsigned(v1))
            .unwrap();
        assert_eq!(s.resolve(NameStore::COMPILER_LATEST).unwrap(), v1);
        // A later set moves the pointer; resolve returns the NEW latest (value-over-time).
        s.set(NameStore::COMPILER_LATEST, SetEntry::unsigned(v2))
            .unwrap();
        assert_eq!(s.resolve(NameStore::COMPILER_LATEST).unwrap(), v2);
    }

    #[test]
    fn resolve_of_a_never_set_name_is_no_such_name() {
        let s = NameStore::new();
        assert_eq!(
            s.resolve(NameStore::COMPILER_LATEST),
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
    fn policy_current_const_is_a_system_scoped_well_formed_name() {
        // The §20b policy-pointer const: pin its value (one source of truth) + that it's System-scoped, so a
        // store/set that swaps the live authorization policy is write-gated by a system/ grant (only an admin
        // authority may repoint the policy governing everyone — the same anti-hijack property as the compiler
        // pointer, applied to the thing that DECIDES authorization).
        assert_eq!(NameStore::POLICY_CURRENT, "system/policy/current");
        assert_eq!(
            NameStore::authority_prefix_of(NameStore::POLICY_CURRENT),
            NameAuthority::System,
            "the policy pointer must be System-scoped (write-gated)"
        );
        let mut s = NameStore::new();
        s.set(
            NameStore::POLICY_CURRENT,
            SetEntry::unsigned(Hash::of(b"policy-wasm")),
        )
        .unwrap();
        assert_eq!(
            s.resolve(NameStore::POLICY_CURRENT).unwrap(),
            Hash::of(b"policy-wasm")
        );
    }

    #[test]
    fn authority_prefix_parse_covers_the_four_namespaces_and_fails_closed() {
        assert_eq!(
            NameStore::authority_prefix_of(NameStore::COMPILER_LATEST),
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
        let bytes = crate::event_ast::encode_name_set(NameStore::COMPILER_LATEST, &h);
        let (name, hash) = crate::event_ast::decode_name_set(&bytes).unwrap();
        let mut s = NameStore::new();
        s.set(&name, SetEntry::unsigned(hash)).unwrap();
        assert_eq!(s.resolve(NameStore::COMPILER_LATEST).unwrap(), h);
    }

    #[test]
    fn replay_set_entries_rebuilds_value_over_time_and_resolves_the_latest() {
        // §4c recovery: a durable backend persists set-events; replaying them (oldest→newest) reconstructs
        // the store's value-over-time, and resolve returns the LATEST per name — like KV rebuilt from the log.
        let (v1, v2) = (Hash::of(b"compiler v1"), Hash::of(b"compiler v2"));
        let sess = Hash::of(b"scratch");
        let rebuilt = NameStore::replay_set_entries([
            (NameStore::COMPILER_LATEST, v1),
            ("session/abc/scratch", sess),
            (NameStore::COMPILER_LATEST, v2), // a later set moves the pointer
        ])
        .expect("replay a well-formed set-event stream");
        assert_eq!(rebuilt.resolve(NameStore::COMPILER_LATEST).unwrap(), v2);
        assert_eq!(rebuilt.resolve("session/abc/scratch").unwrap(), sess);
        // The full value-over-time is preserved (audit/rollback), not just the latest.
        let hist: Vec<Hash> = rebuilt
            .history(NameStore::COMPILER_LATEST)
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
    fn to_set_entries_round_trips_through_replay_and_is_deterministic() {
        // The durable snapshot/restore pair: to_set_entries → replay_set_entries reconstructs an identical
        // store, and the export is byte-STABLE regardless of insertion order (name-sorted + per-name append
        // order), like the KV snapshot.
        let (a1, a2, b1) = (Hash::of(b"a1"), Hash::of(b"a2"), Hash::of(b"b1"));
        let mut s = NameStore::new();
        // Insert in a deliberately non-sorted, interleaved order.
        s.set("system/z", SetEntry::unsigned(b1)).unwrap();
        s.set("system/a", SetEntry::unsigned(a1)).unwrap();
        s.set("system/a", SetEntry::unsigned(a2)).unwrap();

        let entries = s.to_set_entries();
        // Deterministic: names ascending, per-name oldest→newest.
        assert_eq!(
            entries,
            vec![
                ("system/a".to_string(), a1),
                ("system/a".to_string(), a2),
                ("system/z".to_string(), b1),
            ]
        );

        // Round-trip: replay reconstructs the same latest values + full history.
        let rebuilt =
            NameStore::replay_set_entries(entries.iter().map(|(n, h)| (n.as_str(), *h))).unwrap();
        assert_eq!(rebuilt.resolve("system/a").unwrap(), a2);
        assert_eq!(rebuilt.resolve("system/z").unwrap(), b1);
        assert_eq!(
            rebuilt.to_set_entries(),
            entries,
            "export is idempotent across a round-trip"
        );

        // Insertion-order-independent: a store built in a different order exports IDENTICALLY.
        let mut s2 = NameStore::new();
        s2.set("system/a", SetEntry::unsigned(a1)).unwrap();
        s2.set("system/z", SetEntry::unsigned(b1)).unwrap();
        s2.set("system/a", SetEntry::unsigned(a2)).unwrap();
        assert_eq!(
            s2.to_set_entries(),
            entries,
            "byte-stable regardless of insertion order"
        );
    }

    #[test]
    fn merge_appends_from_folds_a_sessions_new_writes_back_into_canonical() {
        // The single-host-owned shared-store flow (§4c v0.3): host holds canonical, hands session A a copy,
        // A writes, host merges A's new appends back — then a copy handed to B sees A's write. No bridge.
        let (v1, v2, other) = (Hash::of(b"v1"), Hash::of(b"v2"), Hash::of(b"other"));
        let mut canonical = NameStore::new();
        canonical
            .set("system/compiler/latest", SetEntry::unsigned(v1))
            .unwrap();

        // Session A gets a COPY of canonical, then store/set's the pointer to v2 (append to its own log).
        let mut session_a = NameStore::replay_set_entries(
            canonical
                .to_set_entries()
                .iter()
                .map(|(n, h)| (n.as_str(), *h)),
        )
        .unwrap();
        session_a
            .set("system/compiler/latest", SetEntry::unsigned(v2))
            .unwrap();
        // A also touches a name canonical never had.
        session_a
            .set("session/a/scratch", SetEntry::unsigned(other))
            .unwrap();

        // Host merges A's new appends back into canonical.
        canonical.merge_appends_from(&session_a);
        // Canonical now resolves the pointer to A's new value, with full history preserved (v1 then v2)...
        assert_eq!(canonical.resolve("system/compiler/latest").unwrap(), v2);
        assert_eq!(
            canonical
                .history("system/compiler/latest")
                .iter()
                .map(|e| e.hash)
                .collect::<Vec<_>>(),
            vec![v1, v2],
            "merge appends the new tail, not a duplicate of the shared prefix"
        );
        // ...and the brand-new name A created is now in canonical too.
        assert_eq!(canonical.resolve("session/a/scratch").unwrap(), other);

        // A COPY handed to session B now sees A's published pointer — the whole point (no export/replay bridge).
        let session_b = NameStore::replay_set_entries(
            canonical
                .to_set_entries()
                .iter()
                .map(|(n, h)| (n.as_str(), *h)),
        )
        .unwrap();
        assert_eq!(session_b.resolve("system/compiler/latest").unwrap(), v2);

        // Idempotent: re-merging the SAME session (no new writes since) appends nothing (no duplicate tail).
        let before = canonical.to_set_entries();
        canonical.merge_appends_from(&session_a);
        assert_eq!(
            canonical.to_set_entries(),
            before,
            "re-merging an unchanged session is a no-op (append-only, prefix already present)"
        );
    }

    #[test]
    fn snapshot_bytes_round_trips_through_from_snapshot_bytes() {
        // The single-blob durable snapshot/restore: snapshot_bytes → from_snapshot_bytes reconstructs an
        // identical store (a backend blob.put's the bytes, from_snapshot_bytes(blob.get(..)) on recovery).
        let (a1, a2, z1) = (Hash::of(b"a1"), Hash::of(b"a2"), Hash::of(b"z1"));
        let mut s = NameStore::new();
        s.set("system/a", SetEntry::unsigned(a1)).unwrap();
        s.set("system/z", SetEntry::unsigned(z1)).unwrap();
        s.set("system/a", SetEntry::unsigned(a2)).unwrap();

        let blob = s.snapshot_bytes();
        let restored = NameStore::from_snapshot_bytes(&blob).expect("valid snapshot round-trips");
        assert_eq!(restored.resolve("system/a").unwrap(), a2);
        assert_eq!(restored.resolve("system/z").unwrap(), z1);
        assert_eq!(
            restored.to_set_entries(),
            s.to_set_entries(),
            "identical store"
        );
        // Byte-stable: re-snapshotting the restored store yields identical bytes (content-addresses same).
        assert_eq!(restored.snapshot_bytes(), blob);
        // Empty store → empty snapshot → empty store (degenerate round-trip).
        assert!(NameStore::new().snapshot_bytes().is_empty());
        assert_eq!(
            NameStore::from_snapshot_bytes(&[])
                .unwrap()
                .to_set_entries(),
            Vec::<(String, Hash)>::new()
        );
    }

    // §4c session-directory I2 (the #2414-c3 "copies drop the group kind" close): a store with BOTH a
    // single-value pointer AND an OR-set group round-trips through snapshot — the restored store preserves
    // group MEMBERSHIP (add-wins, incl. an observed-remove) AND the group KIND (resolve_all works, resolve
    // is a mode-mismatch), so a store COPY seeded from a snapshot is a faithful replica of both maps.
    #[test]
    fn snapshot_round_trips_or_set_groups_preserving_membership_and_kind() {
        let (a, b, c) = (Hash::of(b"A"), Hash::of(b"B"), Hash::of(b"C"));
        let origin = Hash::of(b"origin");
        let mut s = NameStore::new();
        // A single-value pointer (must still round-trip unchanged) + a group with an observed-remove.
        s.set("system/compiler/latest", SetEntry::unsigned(a))
            .unwrap();
        s.add_op("session/room/lobby", MemberOp::add(a, origin, 0))
            .unwrap();
        s.add_op("session/room/lobby", MemberOp::add(b, origin, 1))
            .unwrap();
        s.add_op("session/room/lobby", MemberOp::add(c, origin, 2))
            .unwrap();
        s.add_op("session/room/lobby", MemberOp::remove(b, origin, 1))
            .unwrap(); // B removed → {A, C}

        let blob = s.snapshot_bytes();
        let restored = NameStore::from_snapshot_bytes(&blob).expect("group snapshot round-trips");

        // GROUP membership survives (add-wins fold reproduced through the snapshot).
        assert_eq!(
            restored.resolve_all("session/room/lobby").unwrap(),
            [a, c].into_iter().collect(),
            "the OR-set membership (B removed) round-trips through the snapshot"
        );
        // GROUP kind survives: resolve_all works, resolve is a mode-mismatch (not restored as a pointer).
        assert_eq!(
            restored.resolve("session/room/lobby"),
            Err(NameStoreError::NameModeMismatch),
            "the group kind is preserved — a copy doesn't degrade it to a pointer"
        );
        // The single-value pointer still round-trips unchanged (no regression from the group frames).
        assert_eq!(restored.resolve("system/compiler/latest").unwrap(), a);
        // Byte-stable: re-snapshotting the restored store yields identical bytes (groups + pointers).
        assert_eq!(
            restored.snapshot_bytes(),
            blob,
            "snapshot is byte-stable across a round-trip"
        );
    }

    #[test]
    fn from_snapshot_bytes_is_total_on_a_malformed_blob() {
        // A corrupt/tampered snapshot fails cleanly (never a panic): a short length prefix, a truncated
        // frame body, and garbage-in-a-frame all → MalformedSnapshot. (match on the Err — NameStore has no
        // Debug, so avoid assert_eq's Ok-side formatting.)
        let is_malformed = |bytes: &[u8]| {
            matches!(
                NameStore::from_snapshot_bytes(bytes),
                Err(NameStoreError::MalformedSnapshot)
            )
        };
        assert!(is_malformed(&[1, 2]), "< 4 bytes: short length prefix");
        // A length prefix claiming 100 bytes but only 3 present → truncated frame.
        let mut truncated = 100u32.to_le_bytes().to_vec();
        truncated.extend_from_slice(&[1, 2, 3]);
        assert!(is_malformed(&truncated), "truncated frame body");
        // A well-framed but non-name-set body (garbage) → the frame doesn't decode.
        let garbage = [0xFFu8; 5];
        let mut bad = (garbage.len() as u32).to_le_bytes().to_vec();
        bad.extend_from_slice(&garbage);
        assert!(is_malformed(&bad), "garbage frame body");
    }

    #[test]
    fn from_snapshot_bytes_fails_closed_on_an_injected_unscoped_name() {
        // §4c anti-hijack: from_snapshot_bytes decodes bytes from the CAS, so a TAMPERED blob is in the
        // threat model. snapshot_bytes itself can never emit an Unscoped name (a valid store can't hold one),
        // but an attacker with CAS-write could hand-craft a WELL-FRAMED name-set naming an unwritable prefix
        // (`evil/x → hash`) to smuggle a nonsense pointer into a recovered store. The restore path must reject
        // it — replay_set_entries_fails_closed pins this for replay DIRECTLY; this pins it THROUGH the decode
        // path, where the untrusted bytes actually enter. (match on Err — NameStore has no Debug.)
        let frame = crate::event_ast::encode_name_set("evil/x", &Hash::of(b"hijack"));
        let mut adversarial = (frame.len() as u32).to_le_bytes().to_vec();
        adversarial.extend_from_slice(&frame);
        // The blob is perfectly well-FRAMED (not MalformedSnapshot) — it fails on the AUTHORITY check.
        match NameStore::from_snapshot_bytes(&adversarial) {
            Err(e) => assert_eq!(
                e,
                NameStoreError::UnscopedNameUnwritable,
                "an injected Unscoped name is rejected on the authority check, not silently restored"
            ),
            Ok(_) => panic!("a tampered snapshot naming an unwritable prefix must fail-closed"),
        }
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
            s.apply_effect(effect_ct::STORE_SET, NameStore::COMPILER_LATEST, Some(h), k),
            Ok(StoreOutcome::Set(h))
        );
        // store/resolve with no hash → the current (frozen) hash.
        assert_eq!(
            s.apply_effect(
                effect_ct::STORE_RESOLVE,
                NameStore::COMPILER_LATEST,
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
        let name = NameStore::COMPILER_LATEST;

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
        let name = NameStore::COMPILER_LATEST;
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

    // ── §4c session-directory I3: the GROUP store-effect semantic (apply_group_effect) ──────────────────

    #[test]
    fn apply_group_effect_dispatches_add_remove_resolve_all_by_family() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let g = "session/room/lobby";
        let (a, b) = (Hash::of(b"A"), Hash::of(b"B"));
        let origin = Hash::of(b"origin");

        // store/add A, store/add B → GroupOpApplied each; resolve-all → {A, B}.
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_ADD,
                g,
                Some(MemberOp::add(a, origin, 0)),
                Hash::of(b"k-add-a")
            ),
            Ok(StoreOutcome::GroupOpApplied)
        );
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_ADD,
                g,
                Some(MemberOp::add(b, origin, 1)),
                Hash::of(b"k-add-b")
            ),
            Ok(StoreOutcome::GroupOpApplied)
        );
        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_RESOLVE_ALL, g, None, Hash::of(b"k-r1")),
            Ok(StoreOutcome::Members([a, b].into_iter().collect()))
        );

        // store/remove B (its observed add-tag) → membership {A}.
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_REMOVE,
                g,
                Some(MemberOp::remove(b, origin, 1)),
                Hash::of(b"k-rm-b")
            ),
            Ok(StoreOutcome::GroupOpApplied)
        );
        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_RESOLVE_ALL, g, None, Hash::of(b"k-r2")),
            Ok(StoreOutcome::Members([a].into_iter().collect()))
        );
    }

    #[test]
    fn apply_group_effect_add_is_idempotent_by_key_no_duplicate_log_entry() {
        // §16c-S1/D: re-driving the SAME store/add (same idempotency key) after a crash must NOT append a
        // duplicate op — the dedup makes the re-drive a no-op returning the same GroupOpApplied. (Membership
        // would be tag-idempotent anyway, but the LOG must stay append-exact, matching the pointer path.)
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let g = "session/room/lobby";
        let a = Hash::of(b"A");
        let op = MemberOp::add(a, Hash::of(b"origin"), 0);
        let key = Hash::of(b"dispatch-key-A");

        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_ADD, g, Some(op.clone()), key),
            Ok(StoreOutcome::GroupOpApplied)
        );
        assert_eq!(s.group_history(g).len(), 1);
        // RE-DRIVE with the SAME key → no-op, log unchanged (no duplicate op).
        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_ADD, g, Some(op), key),
            Ok(StoreOutcome::GroupOpApplied)
        );
        assert_eq!(
            s.group_history(g).len(),
            1,
            "re-drive by same key must NOT duplicate the group op"
        );
    }

    #[test]
    fn apply_group_effect_is_total_on_malformed_shapes() {
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let g = "session/room/lobby";
        let k = Hash::of(b"k");
        let origin = Hash::of(b"origin");
        // store/add REQUIRES an op (the member); None is malformed, not a panic.
        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_ADD, g, None, k),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/resolve-all must NOT carry an op.
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_RESOLVE_ALL,
                g,
                Some(MemberOp::add(Hash::of(b"m"), origin, 0)),
                k
            ),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // The op's add-flag MUST agree with the family verb — a store/add carrying a remove-op is malformed
        // (never a silent coercion; the family is what authz gated).
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_ADD,
                g,
                Some(MemberOp::remove(Hash::of(b"m"), origin, 0)),
                k
            ),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // …and a store/remove carrying an add-op is likewise malformed.
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_REMOVE,
                g,
                Some(MemberOp::add(Hash::of(b"m"), origin, 0)),
                k
            ),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // a non-group store family (or any non-store family) is not a group verb (defensive backstop).
        assert_eq!(
            s.apply_group_effect(effect_ct::STORE_SET, g, None, k),
            Err(NameStoreError::MalformedStoreEffect)
        );
        // store/add to an Unscoped name fails closed via the underlying add_op().
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_ADD,
                "bare-group",
                Some(MemberOp::add(Hash::of(b"m"), origin, 0)),
                k
            ),
            Err(NameStoreError::UnscopedNameUnwritable)
        );
    }

    #[test]
    fn apply_group_effect_refuses_a_name_already_a_single_value_pointer() {
        // pointer-XOR-group mode-guard (D7) at the effect layer: a name written as a pointer via store/set
        // can't then be joined as a group — add_op surfaces NameModeMismatch, and apply_group_effect
        // propagates it (an observable Err the drive loop folds, never a coercion into both maps).
        use crate::effect::effect_ct;
        let mut s = NameStore::new();
        let name = NameStore::COMPILER_LATEST; // system/… — a valid scoped pointer name
        s.apply_effect(
            effect_ct::STORE_SET,
            name,
            Some(Hash::of(b"v")),
            Hash::of(b"k-set"),
        )
        .unwrap();
        assert_eq!(
            s.apply_group_effect(
                effect_ct::STORE_ADD,
                name,
                Some(MemberOp::add(Hash::of(b"m"), Hash::of(b"o"), 0)),
                Hash::of(b"k-add")
            ),
            Err(NameStoreError::NameModeMismatch)
        );
    }

    // ── §4c session-directory I1: OR-set group membership ──────────────────────────────────────────────

    // FOLD CORRECTNESS + ADD-WINS: membership = fold the add/remove log; a member re-added after a remove
    // (with a FRESH tag) is present (add-wins), and a member whose only add-tag was removed is absent.
    #[test]
    fn resolve_all_folds_add_remove_with_add_wins() {
        let mut s = NameStore::new();
        let g = "session/room/lobby";
        let (a, b, c) = (Hash::of(b"A"), Hash::of(b"B"), Hash::of(b"C"));
        let origin = Hash::of(b"origin");
        // add A, add B, add C, remove B(tag it was added with), add B(FRESH tag) → {A, B, C} (B re-added).
        s.add_op(g, MemberOp::add(a, origin, 0)).unwrap();
        s.add_op(g, MemberOp::add(b, origin, 1)).unwrap();
        s.add_op(g, MemberOp::add(c, origin, 2)).unwrap();
        s.add_op(g, MemberOp::remove(b, origin, 1)).unwrap(); // retracts the (origin,1) add of B
        s.add_op(g, MemberOp::add(b, origin, 3)).unwrap(); // re-add B with a fresh tag
        assert_eq!(
            s.resolve_all(g).unwrap(),
            [a, b, c].into_iter().collect(),
            "add-wins: B re-added with a fresh tag is present; A + C untouched"
        );

        // Remove the sole remaining add of A → A absent.
        s.add_op(g, MemberOp::remove(a, origin, 0)).unwrap();
        assert_eq!(
            s.resolve_all(g).unwrap(),
            [b, c].into_iter().collect(),
            "a member whose every add-tag is removed is absent"
        );
    }

    // MULTI-WRITER MERGE is commutative + idempotent (the load-bearing CRDT property, D2): two sessions add
    // to the SAME group concurrently (divergent logs, neither a prefix of the other); merging either
    // direction — and twice — yields the SAME membership. The single-value prefix-append merge could NOT do
    // this (it would drop one writer's op).
    #[test]
    fn group_merge_is_commutative_and_idempotent_over_concurrent_writers() {
        let g = "session/room/lobby";
        let (a, b) = (Hash::of(b"A"), Hash::of(b"B"));
        // Writer 1's replica: adds A (its own identity).
        let mut r1 = NameStore::new();
        r1.add_op(g, MemberOp::add(a, a, 0)).unwrap();
        // Writer 2's replica: adds B. Divergent — r1's + r2's logs are NOT prefixes of each other.
        let mut r2 = NameStore::new();
        r2.add_op(g, MemberOp::add(b, b, 0)).unwrap();

        // Merge r2 into r1, and (separately) r1 into r2 — both converge to {A, B}.
        let mut r1_then_r2 = NameStore::new();
        r1_then_r2.merge_appends_from(&r1);
        r1_then_r2.merge_appends_from(&r2);
        let mut r2_then_r1 = NameStore::new();
        r2_then_r1.merge_appends_from(&r2);
        r2_then_r1.merge_appends_from(&r1);
        assert_eq!(
            r1_then_r2.resolve_all(g).unwrap(),
            [a, b].into_iter().collect(),
            "concurrent adds from two writers both survive the merge (multi-writer-safe)"
        );
        assert_eq!(
            r1_then_r2.resolve_all(g).unwrap(),
            r2_then_r1.resolve_all(g).unwrap(),
            "merge is COMMUTATIVE — order doesn't change membership"
        );
        // Idempotent: merging the same replica again changes nothing (tag-union dedups).
        let before = r1_then_r2.group_history(g).len();
        r1_then_r2.merge_appends_from(&r1);
        r1_then_r2.merge_appends_from(&r2);
        assert_eq!(
            r1_then_r2.group_history(g).len(),
            before,
            "merge is IDEMPOTENT — re-merging appends no duplicate ops (tag-union)"
        );
    }

    // REGRESSION PIN (D1 no-regression): a SINGLE-VALUE pointer name's merge is BYTE-IDENTICAL to before the
    // OR-set change — the group path must not perturb compiler/latest / policy/current. Merges the same
    // pointer log both ways + asserts the resolved pointer + full history are unchanged.
    #[test]
    fn single_value_pointer_merge_is_unchanged_by_the_group_layer() {
        let name = NameStore::COMPILER_LATEST;
        let (v1, v2) = (Hash::of(b"v1"), Hash::of(b"v2"));
        let mut src = NameStore::new();
        src.set(name, SetEntry::unsigned(v1)).unwrap();
        src.set(name, SetEntry::unsigned(v2)).unwrap();

        let mut dst = NameStore::new();
        dst.merge_appends_from(&src);
        assert_eq!(
            dst.resolve(name).unwrap(),
            v2,
            "pointer merge yields the latest set (unchanged)"
        );
        assert_eq!(
            dst.history(name).len(),
            2,
            "the full value-over-time log merged (unchanged)"
        );
        // Re-merge is a no-op (longer-log-wins prefix path — the exact prior behavior).
        dst.merge_appends_from(&src);
        assert_eq!(
            dst.history(name).len(),
            2,
            "re-merge appends nothing (prefix path unchanged)"
        );
    }

    // MODE-GUARD (D7): a name is a pointer XOR a group, fixed by its first write. A group verb on a pointer
    // name (and vice-versa), and resolve-all of a pointer, are refused with NameModeMismatch — never coerced.
    #[test]
    fn a_name_is_pointer_xor_group_mode_mismatch_is_refused() {
        let mut s = NameStore::new();
        // Establish a POINTER, then a group add on it is refused.
        s.set("system/x", SetEntry::unsigned(Hash::of(b"v")))
            .unwrap();
        assert_eq!(
            s.add_op("system/x", MemberOp::add(Hash::of(b"m"), Hash::of(b"o"), 0)),
            Err(NameStoreError::NameModeMismatch),
            "a group add on a single-value pointer name is refused"
        );
        assert_eq!(
            s.resolve_all("system/x"),
            Err(NameStoreError::NameModeMismatch),
            "resolve_all of a pointer name is a misuse"
        );
        // Establish a GROUP, then a set on it is refused.
        s.add_op(
            "session/g/room",
            MemberOp::add(Hash::of(b"m"), Hash::of(b"o"), 0),
        )
        .unwrap();
        assert_eq!(
            s.set("session/g/room", SetEntry::unsigned(Hash::of(b"v"))),
            Err(NameStoreError::NameModeMismatch),
            "a set on an OR-set group name is refused"
        );
        // Group verbs on an Unscoped prefix still fail closed.
        assert_eq!(
            s.add_op("bare", MemberOp::add(Hash::of(b"m"), Hash::of(b"o"), 0)),
            Err(NameStoreError::UnscopedNameUnwritable),
            "a group add on an Unscoped name fails closed"
        );
    }

    // resolve_all of a name that was never touched as a group → NoSuchName (not an empty set — distinguish
    // "unknown group" from "empty group").
    #[test]
    fn resolve_all_of_an_unknown_group_is_no_such_name() {
        let s = NameStore::new();
        assert_eq!(
            s.resolve_all("session/room/ghost"),
            Err(NameStoreError::NoSuchName)
        );
    }

    // userspace-effects I1: the effect/* registration namespace. effect/<family> is a governed (writable)
    // authority pointing at a handler SessionId; resolve_effect_handler round-trips it; a degenerate
    // "effect/" (no family) is Unscoped/unwritable (the anti-hijack fail-closed backstop).
    #[test]
    fn effect_registry_registers_and_resolves_a_handler_and_is_anti_hijack_scoped() {
        let mut s = NameStore::new();
        let handler = Hash::of(b"weather-handler-session-id");
        // effect/<family> is the Effect authority (governed, writable — a Cedar-granted store/set claims it),
        // NOT Unscoped: this is what lets the drive-loop authz gate require a grant over the exact name.
        assert_eq!(
            NameStore::authority_prefix_of("effect/weather"),
            NameAuthority::Effect
        );
        // Register effect/weather → H via the store set (the store SEMANTIC; the Cedar who-may-write gate is
        // the drive loop's, keyed on the Effect authority above).
        s.set("effect/weather", SetEntry::unsigned(handler))
            .unwrap();
        // Resolve it back through the typed resolver (bare family → prepends effect/).
        assert_eq!(s.resolve_effect_handler("weather"), Some(handler));
        // An unregistered family resolves to None (a normal "not a userspace effect" answer, not an error).
        assert_eq!(s.resolve_effect_handler("stocks"), None);
        // ANTI-HIJACK fail-closed: a degenerate "effect/" (empty family tail) governs NO real name → Unscoped
        // → unwritable, so a malformed registration can never claim the whole namespace.
        assert_eq!(
            NameStore::authority_prefix_of("effect/"),
            NameAuthority::Unscoped
        );
        assert_eq!(
            s.set("effect/", SetEntry::unsigned(handler)),
            Err(NameStoreError::UnscopedNameUnwritable),
            "a degenerate effect/ (no family) is unwritable — fail-closed anti-hijack"
        );
    }

    // is_registered_effect_family: the SYNTACTIC partition boundary — a family that is NOT a built-in
    // well-known partition is a userspace-effect candidate (built-ins are never shadowed by a handler).
    #[test]
    fn is_registered_effect_family_excludes_builtins_admits_novel_families() {
        use crate::effect::effect_ct;
        // Novel families (not a kernel built-in) → userspace-effect candidates.
        assert!(effect_ct::is_registered_effect_family("weather"));
        assert!(effect_ct::is_registered_effect_family("stocks"));
        // Built-in well-known families are NOT userspace effects (no shadowing).
        assert!(!effect_ct::is_registered_effect_family(effect_ct::SHELL));
        assert!(!effect_ct::is_registered_effect_family(effect_ct::HTTP));
        assert!(!effect_ct::is_registered_effect_family(
            effect_ct::STORE_SET
        ));
        assert!(!effect_ct::is_registered_effect_family(effect_ct::FS_READ));
        assert!(!effect_ct::is_registered_effect_family(effect_ct::BLOB_PUT));
        assert!(!effect_ct::is_registered_effect_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_registered_effect_family(
            effect_ct::CAPABILITIES
        ));
        // The lifecycle partition (lifecycle/spawn|suspend|resume|terminate) is a built-in executor-routed
        // family, NOT a userspace-effect candidate — else a session Cedar-granted `store/set effect/lifecycle/
        // spawn` could register a handler that SHADOWS the real lifecycle executor once I3 routes on this
        // predicate (a lifecycle hijack). This arm is the missed witness that let the omission slip in.
        assert!(!effect_ct::is_registered_effect_family(
            effect_ct::LIFECYCLE_SPAWN
        ));
        assert!(!effect_ct::is_registered_effect_family(
            effect_ct::LIFECYCLE_TERMINATE
        ));
        // Empty family is not a candidate.
        assert!(!effect_ct::is_registered_effect_family(""));
    }
}

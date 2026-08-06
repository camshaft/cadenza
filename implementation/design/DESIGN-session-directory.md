# Session directory — multi-value names → group addressing / multicast

Owner: a new `vertical` (area = `cdz-kernel`), coordinated with `v-agent-harness` (owns the session model +
`name_store.rs`) and `v-agent-harness-host` (owns the Emit executor + peer-inbox routing). Design by
`design-session-directory`. Status: **PROPOSAL — shaped autonomously (operator delegated the design and is
not iterating live); coordinated with `v-agent-harness` via `note`.** Operator idea via concierge 2026-08-06.

> **Operator spark (verbatim).** *"Yeah a directory would be interesting. For that I wonder if we make the
> global name service able to store multiple values? And that would make it naturally support resolving
> multiple targets for a single name."*

This is the richer follow-on to cross-session messaging (the by-id `EffectKind::Emit → peer inbox → Inbound`
path `v-agent-harness` + `v-agent-harness-host` are building NOW). Where messaging answers *"send to the
session id I already hold,"* this answers *"send to a **name** that resolves to **many** sessions"* — a
directory + multicast. It layers ON TOP of by-id Emit; it does not replace it.

## The one subtlety that shapes everything

The Global Name Service (`cdz-kernel/src/name_store.rs`, §4c) **already stores multiple values per name** —
but as an **append-only value-over-time log** where `resolve` returns the **latest** entry (last-write-wins).
That is the right model for a *pointer* (`system/compiler/latest` → the newest wasm hash): audit + rollback
fall out, and a resolver freezes the resolved hash into its own log (§4c point 3, replay-safe).

A **directory / group** wants something categorically different: multiple values that are **all
simultaneously current** — a *set of live members* — with the ability to **leave** (retract a value), not
just "supersede with a newer one." So "store multiple values" is not one feature; it is a **second
interpretation of the same per-name log**:

| notion                     | log semantics                    | `resolve` returns        |
|----------------------------|----------------------------------|--------------------------|
| **pointer** (today)        | last-write-wins over time        | the **latest** hash      |
| **group** (this design)    | add/remove **set** membership    | the **current set**      |

The design's core decision is **how to add the group interpretation without disturbing the pointer one** —
because `system/compiler/latest` and `system/policy/current` (the anti-hijack pointers) MUST keep their exact
current last-write-wins + single-writer semantics.

## Decisions (made autonomously; rationale inline)

### D1. Data model — a group name's log is an **OR-set** (add/remove entries; membership = fold the log)

A group name carries the SAME per-name append-only log, but its entries are `add(member)` / `remove(member)`
events, and **current membership = fold the log** (add-wins OR-set semantics). Single-value pointer names are
unchanged: they keep last-write-wins, `resolve → latest`.

```
session/room/lobby → [ add A, add B, add C, remove B, add B ]
   resolve_all  → { A, B, C }     (fold add/remove; add-wins)
   (a pointer name, e.g. system/compiler/latest, still resolves → latest, untouched)
```

Why OR-set and not name→list-forever or a separate store:

- **"Everything is a log" (§4c point 1) is preserved.** Membership, audit (who joined/left when — once
  producers ride the envelope), rollback, and — critically — **snapshot/replay** (`snapshot_bytes` /
  `from_snapshot_bytes` / `replay_set_entries` / `to_set_entries`) all extend to groups for free: a group is
  just a name whose frames are add/remove instead of set. No parallel durability machinery.
- **It supports `leave` and (later) death-retraction**, which a monotonic name→list cannot.
- **It is the ONLY multi-writer-safe choice** — see D2, the load-bearing reason.

**Rejected — a separate `directory/*` effect family with its own store:** clean separation, but it duplicates
the log framing, the Cedar prefix-authority, and the snapshot/replay/merge machinery `name_store` already has.
The group log IS a name-store log; reusing it is the smaller, more coherent change.

### D2. Multi-writer safety — the OR-set must be a **CRDT** (the real reason D1 is forced)

This is the load-bearing constraint, and the one thing that **touches `v-agent-harness`'s existing code.**

A pointer name has **one writer** (only a `system/` grant repoints `system/compiler/latest`). `name_store`'s
`merge_appends_from` (host reconcile after each session turn) bakes this in: *"if `other`'s log is LONGER
than mine, its log is my log plus a tail — append the tail."* That "longer log ⊇ my log as a prefix" property
**holds only for a single writer.**

A **group name has MANY concurrent writers** — every member adds/removes ITSELF, in different sessions, in
the same turn window. Two sessions each appending to `session/room/lobby` in parallel produce two logs that
are **NOT prefixes of each other**, so the prefix-tail `merge_appends_from` would **drop one session's write**
(or worse, interleave incorrectly). The single-writer assumption breaks.

**The fix — and why OR-set specifically:** an OR-set is a well-known CRDT. Each `add` carries a **unique tag**
`(member, nonce)` (the nonce derived deterministically — e.g. from the emitting session's id + its local
effect id — never `Math.random`, to stay replay-stable). Merge = **union of add-tags minus the set of
remove-tags**; membership = a member is present iff it has ≥1 add-tag not covered by a remove. This merge is
**commutative, associative, idempotent, order-independent** — exactly what multi-writer reconcile needs. The
current prefix-append is a *degenerate correct case* of it (single writer → logs ARE prefixes → tag-union =
tail-append), so:

- **Pointer names keep the EXACT current path.** `merge_appends_from` stays byte-for-byte for single-value
  logs; the CRDT merge is a NEW code path taken only for group (add/remove-framed) names. No regression risk
  to `compiler/latest` / `policy/current`.
- **`resolve` (latest) is unchanged** for pointers; **`resolve_all` (fold)** is the new read for groups.

Remove semantics: **add-wins observed-remove** (standard OR-set) — a `remove` cancels only the add-tags it has
*observed*; a concurrent fresh `add` (new tag) survives. This gives the intuitive "if you rejoin while someone
evicts your old membership, you stay in."

### D3. New effect vocabulary — under the existing `store/` namespace, not a new top-level family

Add three families beside `store/set` + `store/resolve` in `cdz-kernel/src/effect.rs::effect_ct`:

| family              | shape                               | semantics                                                        |
|---------------------|-------------------------------------|------------------------------------------------------------------|
| `store/add`         | target = group name, payload = member value | append an `add(member)` entry (tagged) to the group's log   |
| `store/remove`      | target = group name, payload = member value | append a `remove(member)` entry                            |
| `store/resolve-all` | target = group name, no payload     | **query**: freeze + return the CURRENT member set (see D4)       |

They inherit `is_store_family` routing and the **Cedar name-prefix authority gate** for free (D5). They are
all NEW families (no wire history), so they carry the `store/` prefix — the same asymmetry `store/set` already
documents. `store/resolve` (latest) and `store/resolve-all` (set) coexist: a name is used as a pointer OR a
group by which verbs write it; the design does not force a type tag (a name written only by `add`/`remove` IS
a group), though I3 adds a cheap consistency guard (D7).

### D4. Multicast — `resolve-all` is a **query effect that freezes the set**, then fan-out (replay-safe)

Membership is **mutable current-view** state ("who is in the group *right now*"). The kernel design's bridge
rule (§4b, and exactly the constraint `DESIGN-host-capability-discovery.md` turns on) says a mutable
current-view read **MUST be a query effect frozen into the local log** — never a live read, which poisons
replay (the same event folds differently tomorrow). So:

`store/resolve-all` resolves the member set **as-of-now**, and **freezes that set into the sender's log** at a
hash. Fan-out then happens over the **frozen** set — deterministic on replay even though membership keeps
changing. Two ways to fan out:

- **v0 (this design): reducer-side loop, reusing by-id Emit.** The reducer does `resolve-all` → gets the
  frozen member set → loops and emits one ordinary **by-id `Emit`** per member. This **reuses the entire
  `Emit → peer inbox → Inbound` path `v-agent-harness`/`v-agent-harness-host` are building now** — zero new
  delivery mechanism, and each per-member Emit gets its own Cedar check for free. Multicast = "resolve the
  set, then N unicasts," which is exactly the operator's "resolve multiple targets for a single name."
- **Deferred: a kernel `emit-group` convenience.** One effect carrying (group name, payload); the host does
  resolve + fan-out + per-member authz. Cleaner reducer surface, but a new effect family + a new authz shape
  (per-member gate inside one effect). **Not needed for the operator's ask** — defer until the reducer-side
  loop proves the boilerplate is worth removing.

### D5. Authorization — reuse the prefix-authority; **self-join default, owner-eviction layered**

Writes stay gated by the name's **prefix authority** (`NameStore::authority_prefix_of` + a Cedar prefix
grant), exactly as `store/set` is today. On top of that, membership adds a **subject constraint** the
authorizer checks:

- **Self-registration (default).** A session may `store/add` / `store/remove` **only its own SessionId** as
  the member value, under a group name its grant authorizes. No session can add or evict another. This is the
  natural, least-authority model for opt-in groups (a `topic/…` / `session/…`-scoped group).
- **Owner-managed eviction (layered).** Removing *another* member requires the group name's **owning
  authority** (the prefix grant on that name). Covers moderated groups / curated access lists. Adding another
  session as a member is likewise owner-only.

So the rule is: **`add/remove self` = per your join grant; `remove other` = owner only.** This is a Cedar
policy shape on the existing authz seam (subject = member value vs the emitting session), not new kernel
mechanism — the kernel passes the member value + emitter identity to the authorizer; Cedar decides.

### D6. Scope — naming/multicast NOW; liveness is a **separate** feature

The seed asked whether this is one feature or two ("naming-multicast vs liveness-query"). **It is two.** This
design ships **explicit** membership (join/leave/resolve-all/multicast). Two things are explicitly OUT, as
their own later increments/designs:

- **Session-death auto-retract** — a member vanishing when its session dies. Needs a **host liveness signal**
  + a retract-on-death hook (the host injects a `remove` when it observes a session end). This is host
  mechanism `v-agent-harness-host` owns, and it is cleanly additive: it just emits `store/remove(member)` on
  death, using D1's remove path. Flagged as increment **I5 (deferred / own slice)**.
- **A "who is alive" liveness query** — orthogonal to naming. A directory here answers *"who joined this
  name,"* not *"who is currently running."* A member can be in a group and its session dead (until I5 prunes
  it). Liveness is its own query-effect design (same freeze-into-log shape as capability discovery), NOT part
  of this doc.

Keeping these out keeps the multicast feature tight and shippable, and avoids coupling in death-detection now.

### D7. A group is self-describing by its frames (no separate type tag), with a cheap misuse guard

A name is a **pointer** if written by `set`, a **group** if written by `add`/`remove`. To prevent silent
corruption (a `set` onto a group name, or `resolve` (latest) of a group), I3 adds a per-name **mode guard**:
the first write kind fixes the name's mode; a mismatching verb (`set` on an add/remove name, or vice-versa)
returns a new `NameStoreError::NameModeMismatch` rather than producing nonsense. `resolve-all` of a pointer
name and `resolve` (latest) of a group name are likewise mismatches. Cheap, total, fail-closed — and it means
the existing pointer names are provably untouched by group verbs.

## Increments (top-to-bottom, the way a vertical will land them)

Each increment is independently green (per-commit-green invariant) and carries its own gate.

- **I1 — OR-set fold + CRDT merge in `name_store.rs` (pure, no wire).** Add the add/remove entry kind
  (`SetEntry` grows a variant, or a sibling `MemberEntry` — implementer's call at the struct seam), the
  tagged-add model, `resolve_all(name) -> BTreeSet<Hash>` (deterministic order), and the CRDT branch of
  `merge_appends_from` taken only for group names. Pointer names keep the exact current path. **Gate:** rcdzc
  lib unit tests — fold correctness, add-wins observed-remove, and the multi-writer merge property (two
  divergent group logs merge commutatively + idempotently; a single-value log's merge is byte-identical to
  today). This is the load-bearing slice; it lands before any wire/effect change.

- **I2 — durable codec + snapshot/replay for add/remove frames.** Extend `event_ast` (`encode/decode`) with
  add/remove frames beside `name-set`; confirm `snapshot_bytes` / `from_snapshot_bytes` /
  `replay_set_entries` / `to_set_entries` round-trip a group log deterministically (byte-stable, name-sorted).
  **Gate:** round-trip + malformed-frame totality tests, mirroring the existing `name_store` snapshot tests.

- **I3 — the three `store/*` effect families + apply_effect + mode guard.** Add `STORE_ADD`, `STORE_REMOVE`,
  `STORE_RESOLVE_ALL` consts + `apply_effect` arms + `NameStoreError::NameModeMismatch` (D7). Wire them into
  the drive loop's store-family routing (they inherit `is_store_family` + the authorize gate). Add them to
  `wellknown_static_str` (the off-box-logging safety, #2180). **Gate:** `apply_effect` dispatch + idempotency
  (re-driven `store/add` by key = no duplicate tag) + mode-mismatch reject + a wasmtime run where a reducer
  adds, resolves-all, and observes the set.

- **I4 — the `resolve-all`-freeze query semantics + reducer-side multicast, E2E through the host.** The
  `resolve-all` result freezes into the sender's log; a demo reducer does resolve-all → loops by-id `Emit`
  per member → each peer folds `Inbound`. Coordinated with `v-agent-harness-host` (owns the Emit executor;
  this reuses their by-id path). **Gate:** an E2E (a group of 2–3 sessions; a multicast from one is observed
  as an `Inbound` state change in each member) + a replay test (re-folding the frozen set redelivers the same
  members even after membership changed post-freeze).

- **I5 — (DEFERRED, own slice) session-death auto-retract.** Host injects `store/remove(member)` on session
  end (D6). Needs the host liveness signal; own increment, likely `v-agent-harness-host`-led. Named here so
  the vertical knows the boundary; NOT in the initial build.

## Seams / file anchors

- `implementation/seed/crates/cdz-kernel/src/name_store.rs` — the store; add-set entry kind, `resolve_all`,
  CRDT `merge_appends_from` branch, `NameModeMismatch`, the new `apply_effect` arms. (I own the design; the
  vertical edits here — coordinate with `v-agent-harness`, the file owner.)
- `implementation/seed/crates/cdz-kernel/src/effect.rs::effect_ct` — `STORE_ADD` / `STORE_REMOVE` /
  `STORE_RESOLVE_ALL` consts beside `STORE_SET`/`STORE_RESOLVE`; add to `is_store_family` (prefix already
  covers), `wellknown_static_str`.
- `implementation/seed/crates/cdz-kernel/src/event_ast.rs` — add/remove durable frames beside `name-set`
  (`encode/decode`), keeping the `u32-LE`-length framing shared with `log_store`.
- The drive loop (`kernel.rs` / `reducer.rs` store-family arm) — route the three new families through the
  authorize gate to `apply_effect`, same as `store/set`/`store/resolve` today.
- `cdz-agent-host` (Emit executor + peer-inbox routing) — reused unchanged for the reducer-side fan-out in I4;
  extended for I5 death-retract.
- `wit/reducer.wit` — the guest ABI comment for the store host-import/effect surface documents the new verbs
  (a doc/comment touch; NOT a `REQUIRED_RUNTIME_HASH` change — that hash is `cdz-runtime`'s value heap, not
  this kernel surface).

## The gate that protects it

- **I1/I2** — `cargo test -p rcdzc --lib` (or the kernel crate's unit tests; note `cargo xtask gate` omits
  the rcdzc lib unit tests — run them explicitly, per the standing trap). Fold correctness, the CRDT merge
  property (commutative/idempotent/order-independent), and a **regression pin that a single-value pointer
  log's `merge_appends_from` output is byte-identical to today** (guards `compiler/latest`/`policy/current`).
- **I3** — `apply_effect` dispatch + idempotency + mode-mismatch reject; a wasmtime run where a value
  (the resolved set) executes; a reject test for `NameModeMismatch`.
- **I4** — the cross-session multicast E2E + the replay-stability test (frozen set redelivers identically).
- Standard `cargo xtask check` (fmt + clippy `-D warnings` + `codegen --check`) throughout.

## Open decisions with a chosen default

- **Add-tag nonce source** → derive deterministically from `(emitting SessionId, local effect id)`, NEVER a
  random/clock source (replay-stability). *Default chosen; the vertical picks the exact bytes at the
  `apply_effect` seam where both are in hand.*
- **`resolve_all` order** → `BTreeSet<Hash>` (ascending hash bytes) so the frozen set is byte-stable and the
  multicast fan-out order is deterministic. *Default chosen.*
- **Group vs pointer type tag** → NO explicit tag; mode inferred from first write kind + guarded by
  `NameModeMismatch` (D7). *Default chosen; revisit only if a name legitimately needs both interpretations —
  no current use case.*
- **`emit-group` kernel primitive** → deferred (D4); reducer-side loop is the v0. *Default chosen; promote if
  the boilerplate proves costly.*
- **Cedar `add/remove self` vs `remove other` policy shape** → owner-eviction layered on self-join (D5);
  the exact Cedar policy text is `v-agent-harness-host`'s to author against their authz seam. *Direction
  chosen; policy text is theirs.*

## Coordination note

Sent `v-agent-harness` a `note` flagging the OR-set-on-`name_store` direction and specifically the
multi-writer `merge_appends_from` change (the one thing that touches their code), with the invariant that
single-value pointer names keep the exact current merge path. Absent an objection, this proceeds; the vertical
owner coordinates the `name_store.rs` edits with them as file owner, and `v-agent-harness-host` on the I4 Emit
fan-out + the I5 death-retract boundary.

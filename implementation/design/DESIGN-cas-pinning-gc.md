# CAS pinning and garbage collection — reference counting over the content-addressed DAG, with pins tied to reducer lifetime

**Status:** design of record — reviewed and **adopted by `v-platform`** (the store + lifecycle owner) as
the store-GC design, 2026-08-24. This is a **design-doc-only PR** at the operator's request ("I want a PR
just for the design doc on that. There shouldn't be any implementation of it yet") so the operator can read
it and rule on the open capability question below. The pure-logic core (increments 1–3: ref-carrying store
`put`+`delete`, the `GcLedger` pin/edge sets + liveness, and `collect`) was **prototyped** as PRs
#3175/#3177/#3178 but is **held/drafted** pending this review — the operator was "not super happy with the
direction of the PRs" and asked to hold; nothing is merged. Increment 4 (the `cas-pin`/`cas-unpin` host
calls) is **not implemented** — its **direct-gated-host-call vs routed-effect** shape (§4) is the open
question the operator will decide after reading this doc. Written 2026-08-24 by the `design-cas-pinning-gc`
fleet design agent with the operator; v-platform's three review catches — base62 hash rendering; the terminal
pin-drop realized as the ledger folding the lifecycle event rather than a kernel write; and the pin-key-order
note in §6 (hash-first for liveness vs. session-bulk-drop) — are folded in. It grounds in the platform contract
`design/cadenza-platform.md` (§7 state & lifecycle, §8 the store) and the real store surfaces in
`implementation/seed/crates/cdz-platform/src/` (`blob_store.rs`, `kv.rs`) plus `Hash`/base62, which now
live in `crates/cdz-contract` and are re-exported by `cdz-platform` (per the 2026-08-23 base62 flag-day).
It hands a build
plan to **`v-platform`** (owner of the store + reducer lifecycle/supervision). File/line anchors are
landmarks at the tip this was written against, not pins.

> **The operator's spark (verbatim, via the concierge seed).** *"I think the CAS API needs a way to pin
> hashes, or at least we need to think about CAS GC and how to prevent values from getting purged if they
> are being referenced by programs. [...] I also think the pin needs to be attached to the lifetime of the
> reducer as well. So if it closes then the reference count on the CAS values it pinned would be
> decremented."*

> **The four forking decisions, resolved with the operator in this session (all took the recommended
> default):**
> - **GC model — reference counting.** Content-addressing makes the reference graph a DAG (a blob's hash
>   is derived from its bytes, so a blob can never name itself, transitively or otherwise); therefore there
>   is no cyclic garbage and refcounting is *complete* — no tracing collector is required for correctness.
> - **Where pins/edges live — a pin-ledger reducer above the store.** The `BlobStore` trait stays the
>   minimal put/get/has surface (§8); a `GcLedger` reducer holds the pin set and the reference edges in its
>   own KV state and drives collection. This is "storage is a reducer" (§8) applied to GC.
> - **Reference discovery — declared at put time.** `cas-put(bytes, refs)` — the putter names the hashes
>   its blob points at. The store never parses opaque bytes to find references.
> - **Pin API — direct, auto-scoped host calls.** `cas-pin(hash)` / `cas-unpin(hash)`, async host calls
>   alongside `cas-put`/`cas-get` (§8), attributed by the kernel to the *calling* session. On that session's
>   terminal outcome the kernel delivers its lifecycle event and the `GcLedger` (subscribing) folds it to
>   drop all of that session's pins — this *is* the "tied to reducer lifetime" mechanism.

---

## 1. The gap, stated precisely

Today the store is exactly three async operations and holds bytes forever:

```rust
// implementation/seed/crates/cdz-platform/src/blob_store.rs
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, bytes: Bytes) -> Hash;      // put bytes -> content hash
    async fn get(&self, hash: Hash) -> Option<Bytes>;
    async fn has(&self, hash: Hash) -> bool;
}
```

There is no reference tracking, no pinning, no deletion, and no collection anywhere in `cdz-platform`.
§8 frames the store deliberately as "put bytes and get a hash, get bytes by hash, ask whether a hash is
present" — and stops there. GC is not discussed in the contract at all. So an unbounded store grows
without bound, and nothing prevents a future deletion path from purging a blob a live program still needs.
This proposal fills that hole **without** compromising the two properties §8 prizes: the minimal store
surface, and "the hash is the capability" (an unpermissioned read).

## 2. Two contract facts that shape the whole design

1. **The reference graph is a DAG — cycles are impossible.** A blob's identity is `Hash::of(bytes)`
   (blake3 over its bytes, `cdz-contract`'s `Hash`). To embed hash `h` in a blob you must already possess `h`, which
   means `h`'s bytes already exist; a blob therefore can only reference blobs that predate it. No blob can
   reference itself, and no cycle can form. **Consequence:** reference counting collects *all* garbage —
   the one thing naive refcounting cannot handle (cycles) cannot occur here. We get incremental, no
   stop-the-world collection with no soundness gap.

2. **Storage is a reducer, and lifecycle is the one thing the kernel manages (§7–§8).** The kernel does
   not persist; the blob store and the log store are boundary reducers. But the kernel *does* own reducer
   lifecycle through built-in effects, and lifecycle events — `spawned`, `closed` (with outcome),
   `terminated`, `failed` — are already first-class (§7 `subscribe`). So "a pin drops when the reducer
   closes" has a natural hook: the kernel already observes every session's terminal outcome and can drop
   that session's pins at exactly that moment.

## 3. The model — reference counting over roots

A blob is **retained** iff it is *rooted* or *reachable from a root*. It is **collectable** iff neither.
Reachability follows the declared reference edges (§5). There are three kinds of root; a blob is retained
if **any** of them keeps it:

- **Explicit pins (reducer-scoped).** A `(session, hash)` record. Present for as long as the pinning
  session is live and has not unpinned. This is the operator's mechanism and §4's API. Pins are the escape
  hatch for content a reducer *holds* (in a local, in an in-flight obligation, a large payload it handed a
  child) that is not otherwise rooted.
- **Live-session roots (implicit, kernel-held).** For each live session: its **program hash**, its
  **current state root hash**, and the **log blobs / payloads** its retained log references. These are
  held by the kernel/session machinery, not by explicit pins — the session cannot forget to root its own
  program or state.
- **Retained-history roots (compaction-governed).** Every blob referenced by an *un-pruned* event or a
  *retained snapshot* of any session (live or terminal). This is what §7's conservative "keep raw history"
  default keeps alive so a session stays replayable, and it is released only by a deliberate compaction/
  retention decision — never by a session merely closing.

The liveness predicate, then:

```
retained(h)  ⟺  pinned(h)                              // some live session pins h
             ∨  implicit_root(h)                        // some live session's program / state-root / log
             ∨  history_root(h)                         // some un-pruned event or retained snapshot names h
             ∨  ∃ b retained : h ∈ refs(b)              // reachable from a retained blob via declared edges

count(h)     =  |pins(h)| + |roots naming h| + |{ b retained : h ∈ refs(b) }|
collectable(h) ⟺ count(h) == 0
```

**Pins and edges are sets, not raw counters** — `pins(h)` is a set of `(session, hash)`, `refs` edges are
a set of `(parent, child)`. This makes double-pinning by the same session, and re-`put`ting identical
bytes (which §8's idempotent-by-content put allows), naturally idempotent, sidestepping the classic
refcount double-count/underflow bug. The effective count is *derived* from the sets, not incremented in
place.

**Two independent retention regimes, deliberately.** Explicit pins follow *reducer lifetime* (drop on
close). Implicit + history roots follow *compaction policy* (§7: conservative, keep raw history; prune
only behind a snapshot). A blob a closed session had in its state is therefore **not** purged the instant
the session closes — §7 says the log and state stay "retained and queryable" after a terminal outcome; it
becomes collectable only when compaction prunes the events/snapshots that reference it. What the operator's
"decrement on close" governs precisely is the *explicit pin* regime. Keeping the two regimes separate is
what lets "close releases my holds" and "history stays inspectable" both be true.

## 4. The pin API — direct, auto-scoped host calls

Two new async host calls, siblings of `cas-put`/`cas-get` (§8 — direct calls, not effects, because a
reducer routinely touches the store mid-fold and routing every touch through the async effect model is
needlessly clumsy):

```
cas-pin(hash)    -> ()      // the CALLING session now pins `hash`
cas-unpin(hash)  -> ()      // release this session's pin on `hash` (no-op if not pinned)
```

- **Auto-scoped.** The kernel attributes the pin to the session making the call; a reducer cannot pin on
  behalf of another session. This makes the pin ledger's key `(session, hash)` a fact the kernel authors,
  never spoofable content.
- **Tied to reducer lifetime (the whole point).** When a session reaches any terminal outcome — a
  self-exit `Break`, a `terminate`, or an uncontrolled fold-failure (§7) — **every** pin keyed to that
  session is dropped in one step. The mechanism respects the storage-is-a-reducer boundary: the kernel
  does **not** reach into the ledger's KV (it does not know the ledger's key layout). Instead the kernel
  *delivers the terminal lifecycle event* — which it already does to watchers via the §7 `subscribe` /
  `watch_exit` path — and the **`GcLedger` subscribes to lifecycle events and folds** `closed` /
  `terminated` / `failed` into a bulk delete of its own `pin:*:<session>` keys. So "closure *is* the
  decrement" still holds, but the ledger authors that decrement by folding the event, not a kernel poke
  into its state. No reducer code the *closing* session runs is needed to unpin on the way out; a crashed
  or force-terminated reducer cannot leak pins, because the ledger reacts to the terminal event the kernel
  emits regardless of how the session ended.
- **Idempotent.** `cas-pin` on a hash this session already pins is a no-op; `cas-unpin` on a hash it does
  not pin is a no-op. (Set semantics, §3.)
- **Unpermissioned to read, but pinning is a capability.** Reading stays "the hash is the capability"
  (§8) — unchanged. But *retaining* storage against the collector is a cost a reducer imposes on the
  system, so `cas-pin` is capability-gated where `cas-get` is not (the natural place is the same host-call
  authorization surface the kernel already applies; a middleware can also cap how much a subtree may pin —
  see §11).
- **Pins are per-session resident state.** A session's pin set is part of its recoverable checkpoint
  (§7), so it survives a node restart / replay and is dropped deterministically at the terminal event, not
  at a wall-clock moment.

**Why not a lifecycle *effect* (the runner-up).** An effect version (pin/unpin through the middleware
chain) is more governable but adds ceremony to what is a routine store touch, and — critically — it would
make pinning asynchronous-with-reply where the reducer wants a synchronous "this is now kept" the way
`cas-put` is. We keep the pin a direct call and get governance instead from (a) the capability gate on the
call and (b) middleware that can observe/limit a subtree's pin budget. Recorded here as the rejected
alternative should the governance requirement grow.

## 5. Reference discovery — declared at put time

Reachability needs to know a blob's outbound edges. The store holds opaque bytes and must not parse them
(it would have to understand every blob kind — components, heap nodes, value-codec payloads — coupling the
store to every producer). So **the putter declares the references**, extending `put`:

```rust
// the one addition to the store's write surface
async fn put(&self, bytes: Bytes, refs: &[Hash]) -> Hash;   // refs = hashes this blob points at
```

- The returned hash is still `Hash::of(bytes)` — **`refs` do not affect the content hash.** They are
  out-of-band metadata about the blob's edges, recorded in the `GcLedger` (§6), not mixed into the bytes.
  (Two putters that disagree about a blob's refs is a producer bug; the honest edge set is a property of
  the bytes' meaning, and the natural producers already know it.)
- **Who declares what.** The producers that hold references already know them: a component declares its
  dependency components by hash (§8 "Resolving what a program needs"); the value/heap runtime knows the
  child hashes of a persistent-map node it spills to the store; a reducer putting a record that embeds a
  hash names that hash. Each passes its known children as `refs`.
- **Existing call sites migrate** by passing `&[]` (a leaf blob with no outbound edges) — the common case
  for scalar payloads. This keeps the change mechanical for the many put sites that store leaf values.
- **Rejected alternative — a typed reachability walker** (a component that parses known blob kinds to
  extract hashes). It keeps `put` unchanged but re-introduces exactly the store↔producer coupling §8
  avoids, and it must be taught every new blob kind. Declared-at-put pushes the (small) knowledge to the
  producer that already has it. Recorded as the fallback if a blob kind ever cannot declare its own refs.

## 6. The `GcLedger` reducer

A reducer holding the GC bookkeeping in its own KV state (`kv.rs` surfaces: `put`/`get`/`delete`/`scan` +
`prefix_range`), so the `BlobStore` trait keeps its minimal shape. Its state:

- **edges** — `edge:<parent>:<child>` keys (a set), written when `cas-put(bytes, refs)` records a blob's
  outbound refs. `prefix_range("edge:<parent>:")` streams a blob's children for the cascade (§7 note: the
  store's canonical key order makes this scan replay-deterministic).
- **pins** — `pin:<hash>:<session>` keys (a set). Written on `cas-pin`, deleted on `cas-unpin` and — in
  bulk by session — when the kernel signals a session terminal (§4).
  > **v-platform review catch (surfaced building increment 2).** The hash-first key `pin:<hash>:<session>`
  > makes `pinned(h)` / `|pins(h)|` a clean `prefix_range("pin:<hash>:")` scan — which is exactly what the
  > liveness walk needs (increment 2). But then a session-terminal bulk-drop is **not** a prefix scan (the
  > session is the *suffix*), so "bulk by session prefix" as originally written is not directly expressible.
  > Increment 5's terminal drop needs one of: a second reverse index `pin-by-session:<session>:<hash>` written
  > alongside each pin, or a full pin scan filtered by session at terminal time. Pick per the pin volume when
  > increment 5 lands; increment 2 uses the hash-first key as-is.
- **history/implicit roots** are *derived*, not stored here: they come from the kernel's live-session set
  and the log-store's retained-event/snapshot references at collection time (§7). The ledger asks for them
  rather than duplicating them, so it cannot drift from the truth the kernel and log store hold.

The ledger answers a **`collect`** operation (§7's controlled compaction, extended to blobs — see §8):
enumerate candidate hashes, compute `retained(h)`, and delete the unreachable ones from the store,
cascading (a purged blob's outbound edges are removed and its children re-evaluated). Because the graph is
a DAG, the cascade terminates and needs no cycle detection.

## 7. The one store addition — a privileged delete

Collection has to actually remove bytes, and the store has no deletion today. The minimal, honest
addition:

```rust
async fn delete(&self, hash: Hash) -> bool;   // remove bytes; true if present. GC-only.
```

- **Privileged, not a general reducer call.** Deletion is *not* exposed to ordinary reducers as a host
  call — only the collector (the `GcLedger`/kernel collection path) invokes it, after establishing
  `count(hash) == 0`. Content-addressing means a delete is always safe to *re-put* later (same bytes →
  same hash), so a delete is never destructive to identity, only to residency.
- **Idempotent + content-safe.** `delete` of an absent hash returns `false`; a subsequent `put` of the
  same bytes restores it under the same hash. This is why premature collection is a *liveness* bug (a
  needed blob is temporarily gone until re-put) rather than a *correctness* one — but §9 makes even that
  impossible for a fold that could reference it.

## 8. Collection is controlled, never an eager background sweep

This mirrors §7's rule for compaction verbatim: *"Compaction is controlled, not automatic [...] never an
eager background sweep that quietly erases history."* Blob collection is the same:

- **Runs at a quiescent point between folds**, not concurrently with a running fold. At a collection
  point the root set is fully defined (all live sessions' pins + program/state/log + retained history +
  edges), so any `count(h) == 0` blob is genuinely dead — there is no in-flight fold whose locals could
  still be holding it (a fold that wanted to hold `h` across an event boundary must have pinned it or put
  it in state; that is what §4 is for).
- **Triggered deliberately** — by a retention policy (age/size/tier), an operator action, or coupled to a
  compaction pass — not by a timer racing live work. The default is conservative, matching §7.
- **Batched to be meaningful** — a sweep evaluates many candidates at once, not one-blob-at-a-time on
  every unpin.

## 9. Determinism and replay — the correctness spine

GC must never change a fold's result or break replay (§9: a fold is a pure function of `(event, state)`;
replay re-runs folds). Two guarantees:

1. **A fold only ever `cas-get`s a retained hash.** A fold can reference a hash only if it obtained it —
   from its state (an implicit root → retained), from a `cas-get` of something reachable (reachable →
   retained), from a pin it holds (pinned → retained), or from a `cas-put` it just made. The last case is
   the sharp edge: a freshly-put blob with no inbound edge and no pin would have `count == 0` and be
   collectable immediately. **Rule: `cas-put` implicitly pins the new blob to the putting session** until
   the session either roots it (puts it in state / references it from another retained blob) or unpins it
   or closes. So no fold can observe its own just-put blob vanish.
2. **A retained-history root is never collected while its event is replayable.** Because a fold re-runs on
   replay and re-issues its `cas-get`s, any blob a fold of event `N` dereferences must stay retained as
   long as event `N` is un-pruned. This is exactly the *retained-history root* (§3): GC and compaction are
   **coupled** — a blob referenced by an un-pruned event cannot be collected; pruning event `N` behind a
   snapshot is what releases its history roots, and only then can the newly-unreferenced blobs be swept.
   Compaction's own rule (§7: "do not prune a raw event still needed to deterministically re-apply") is
   what keeps this sound.

Collection itself is not a logged event and does not appear in any fold's inputs, so it cannot perturb
determinism; it only reclaims space behind the retention frontier.

## 10. Increments (each its own commit + gate; top-to-bottom, the way a vertical lands them)

1. **Store: ref-carrying put + privileged delete.** Extend `BlobStore` (`blob_store.rs`) — `put(bytes,
   refs: &[Hash])` and `delete(hash) -> bool`; update `InMemoryBlobStore` and migrate existing call sites
   to pass `&[]`. Unit tests: refs recorded, delete removes + is idempotent, re-put restores. *(Pure store
   layer; Rust unit tests only — no platform behavior driven.)*
2. **`GcLedger` reducer — pins + edges + liveness, no collection yet.** KV-backed `pin:`/`edge:` sets;
   the `retained(h)` / `count(h)` computation over edges + a supplied root set; property tests that
   liveness matches a brute-force reachability oracle on random DAGs.
3. **`collect` — the sweep + cascade.** Given the root set, delete `count == 0` blobs from the store,
   cascade over edges, terminate (DAG). Tests: unreferenced blobs collected, pinned/rooted/reachable blobs
   survive, cascade frees a whole unreferenced subtree, re-put after collect works.
4. **Pin host calls + auto-scoping.** `cas-pin`/`cas-unpin` wired as host calls attributed to the calling
   session; `cas-put`'s implicit self-pin (§9). Capability gate on `cas-pin`.
5. **Lifetime binding.** The `GcLedger` subscribes to lifecycle events (§7 `subscribe` / `watch_exit`)
   and folds a session's `closed` / `terminated` / `failed` into a bulk delete of its own pins
   (`prefix_range("pin:<*>:<session>")` → delete) — the kernel emits the terminal event it already emits,
   the ledger authors the decrement in its own KV (never a kernel write into the ledger). This is the
   operator's headline requirement.
6. **Root-set assembly + controlled collection trigger.** Assemble live-session implicit roots + retained
   -history roots (from the log store) and run `collect` at a controlled point; couple to / gate behind
   compaction (§8/§9).
7. **Conformance coverage.** Behavioral coverage — spawn a session, have it pin a blob, close it, assert
   the blob becomes collectable and a still-referenced one does not — goes in the **conformance suite**
   (`v-platform-itest`), *not* a Rust `#[test]` (operator directive: any test driving behavior through the
   platform is a conformance test).

## 11. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — Pin budget / quota.** *Default:* no hard quota in v1; expose pin *observability* (a session's
  pin count is a resident fact) and let a middleware impose a subtree budget later (§4). Escalate only if
  unbounded pinning is a near-term abuse concern.
- **D2 — When does `collect` actually fire?** *Default:* coupled to a compaction pass + an explicit
  operator/retention trigger; never a background timer (§8). A size/age high-water is a fast follow, not
  v1.
- **D3 — `refs` for existing/foreign blobs already in the store.** *Default:* treat a blob with no
  recorded edges as a leaf (no outbound refs). Correct for the many leaf payloads; a producer that has
  children must declare them. Escalate if any pre-existing non-leaf blob cannot be re-declared.
- **D4 — Cross-tenant / leaked-hash retention.** *Default:* out of scope, tracking §8's own scoping
  boundary ("cross-tenant confidentiality [...] is out of scope here"). A pin only affects residency, not
  readability, so it does not widen the confidentiality surface.
- **D5 — Does a snapshot's `(state-root, program-hash)` auto-root?** *Default:* yes — a retained snapshot
  is a history root (§3), so keeping a snapshot keeps its state/program blobs reachable. Pruning the
  snapshot is what releases them. (This is the §7 snapshot model, not a new concept.)

## 12. Watch-outs (for the implementing vertical)

- **Never collect concurrently with a fold** (§8/§9) — collection is a between-fold, quiescent-point
  operation. A background sweep would make `cas-get` non-deterministic and is the single most dangerous
  way to get this wrong.
- **`cas-put`'s implicit self-pin is load-bearing** (§9) — omitting it lets a fold's fresh put be
  collected before the fold roots it. It must drop when the session roots/unpins/closes, or it becomes a
  leak.
- **Edges are metadata, not content** (§5) — never fold `refs` into the hashed bytes; the content hash
  must stay `Hash::of(bytes)` so dedup and replay are unchanged.
- **Pins and edges are sets** (§3) — derive counts from set membership; do not keep a mutable integer
  counter (the underflow/double-count trap).
- **GC↔compaction coupling is a correctness invariant, not an optimization** (§9) — a blob referenced by
  an un-pruned event must not be collected. Land the coupling with the trigger (increment 6), not later.
- **Base62, not hex** for any rendered hash if the ledger ever surfaces a hash in a log/error line — the
  base62 flag-day landed 2026-08-23 (#3091). It is `cdz_contract::base62` (digits `0-9`, `A-Z`, `a-z`; a
  45-char *tagged* hash, `62^45 > 2^264`), and `Hash` lives in `crates/cdz-contract` (`cdz-platform`
  re-exports it). `design/cadenza-platform.md` §8 already reflects base62 (the sole remaining "base64url"
  mention there is the historical rationale for the choice, not a live claim).

## 13. Verification (the gate that protects this)

- **Increments 1–3** are pure store/ledger logic → Rust unit + property tests (`dev-gate` on
  `cdz-platform`): a random-DAG reachability oracle for `retained`/`count`, cascade termination, re-put
  after collect.
- **Increments 4–6** drive behavior through the platform → **conformance suite** (`v-platform-itest`
  harness + Checker): pin/close/collect scenarios asserting a closed session's pins drop and a still-rooted
  blob survives; a controlled `collect` purges only unreferenced blobs; replay after a collect reaches the
  same state. No platform-driving behavior lands as a Rust `#[test]`.
- **The anti-regression invariant:** a blob referenced by any un-pruned event or live root is *never*
  absent from the store — the conformance Checker asserts this over the recorded run.

## 14. Relationship to the rest of the platform

This sits under §8 (the store) and §7 (lifecycle) and touches neither the compiler nor any WIT contract
the compiler knows (it is entirely platform-side). It composes with the log-persistence reducer (§8 — the
retained-history roots come from there) and with compaction (§7 — the coupling in §9). It does not change
the reducer interface, dispatch, or authz beyond adding two capability-gated host calls. The `every-guest-
Cadenza` and `storage-is-a-reducer` norms are preserved: the `GcLedger` is a reducer, and the only kernel-
side additions are the two host calls and the terminal-outcome pin drop the kernel is already positioned
to do.

# Session log ⇄ session state decouple — bounded hot state, host-side log, checkpoint recovery

Owner: `v-agent-harness` (co-authors + builds the KERNEL seam half — it owns the `Session` model +
recovery contract) with `v-agent-harness-host` (builds the log-persistence + checkpoint BACKEND half — it
owns the host `LogSink`/store). Design by `design-session-log-decouple`. Status: **iterated live with the
operator (2026-08-09); the four design forks below were decided by the operator via `AskUserQuestion`.**
Coordinate closely with both implementers: the kernel exposes the seam, the host owns the bytes.

> **Operator spark (verbatim).** *"We need to decouple the session log from the session state. Otherwise
> sessions are going to bloat memory over time and become very expensive. The log is really only for
> recovery and auditing so forcing it to stay in memory is wasteful."*

## The problem, precisely

`cdz-kernel`'s `Session` (`crates/cdz-kernel/src/kernel.rs:53`) holds `log: Vec<Event>` **fully resident**,
*and already* write-throughs every append to a host `store: Box<dyn LogSink>` (`kernel.rs:89`). So the
durable log ALREADY lives host-side (§16c-S1 durable-before-route). The waste is that the kernel *also*
retains the entire `Vec<Event>` for the life of the session — it grows unboundedly (one frame per inbound,
dispatch, result, timer-arm/fire, …) even though steady-state operation never needs the history.

**The key finding that makes this tractable: every steady-state read of `self.log` is served-able from
BOUNDED derived state, not the full history.** Auditing the current reads (`kernel.rs`):

| current read of `self.log`                              | what it actually needs            | bounded replacement                         |
|---------------------------------------------------------|-----------------------------------|---------------------------------------------|
| `tip_hash`, `fold_tip`, `append` (seq/tip)              | the **last** event                | resident `tip: Event` (or `(seq, hash)` + last body) |
| `genesis_hash`, `reducer_hash`, `genesis_provenance`    | the **first** event               | resident `genesis: Event`                   |
| `status_snapshot` in-flight scan, `dispatch_hash_of`, `dispatch_token_of`, `dispatch_family_of`, `time_out_effect` | frames for **open** effect ids | resident **open-obligation table** (below)  |
| `spawned_children`                                      | the `Spawned` edges               | resident `spawned: Vec<Hash>` (append-time) |
| `already_seeded_capabilities`                           | one seed-signature bit            | resident `seeded_capabilities: bool`        |
| `status_snapshot` `Closed`/`Terminated`, `is_terminated`| lifecycle flags                   | resident `closed: bool` + tip body          |
| `snapshot()` (`seq, kv_root, reducer`)                  | seq + kv-root + reducer           | all three already derived/resident          |

The genuinely-unbounded material — **settled** results, folded **Inbound** events, **TimerFired** frames — is
consumed by NOBODY in steady state. It is *purely* recovery + audit material, exactly the operator's
intuition. So the decouple is: **stop retaining `Vec<Event>`; keep a bounded resident derived-state struct;
stream the log back from the host only on recovery.**

## The hard constraint this must not break

The **frozen §16c replay/recovery contract**: the open-effect obligation set + the settled set are DERIVED
FROM THE LOG on recovery (`Session::replay`, `kernel.rs:1690`; §16c-S3 "replay under the version that wrote
it"). If the log is no longer resident, recovery must reconstruct KV + open + settled + `next_effect_id` +
`last_now` + `armed_timers` some other way. That is what the checkpoint model (D1) provides — and every
increment below keeps `replay()` byte-for-byte intact as the from-genesis fallback, so the contract is
*extended*, never rewritten.

## Decisions (operator-selected forks)

### D1. Recovery model — **periodic derived-state checkpoint + tail stream** (operator pick)

The kernel persists a **checkpoint** at quiescent boundaries: a small record capturing the entire derived
hot state at a sequence `N`:

```
Checkpoint {
  seq: u64,                       // the log seq this checkpoint is AS-OF (tip seq at capture)
  kv_root: Hash,                  // content-addresses the KV bytes in the blob store (kv.rs:133)
  next_effect_id: u64,
  last_now: u64,
  open: Vec<OpenObligation>,      // the resident open-obligation table (D2), serialized
  settled: SettledSet,            // watermark + sparse exceptions (D3), serialized
  armed_timers: Vec<(u64,u64)>,   // id → absolute deadline
  spawned: Vec<Hash>,             // the Spawned edges (spawned_children)
  seeded_capabilities: bool,
  closed: bool,
  genesis: Event,                 // log[0] verbatim — cheap, one event, needed for genesis_hash/reducer_hash
}
```

Recovery: **load the latest checkpoint, fetch KV bytes by `kv_root` from the blob store (`Kv::decode`), then
stream ONLY the log tail `seq > N` from the host store and re-fold that tail.** No re-fold from genesis. The
KV is ALREADY content-addressed and restorable — `kv.rs:6` states verbatim *"the root hash is a free
per-event snapshot … checkpointing is a retention choice"* and `kv.rs:69` documents *"store `encode()` in
the blob store keyed by `root_hash()`, and a snapshot is only a real checkpoint if the bytes it addresses
exist."* This design is the retention choice that seam was built for.

- **Where the log persists:** unchanged — the host `LogSink`/store (`log_store.rs`, owned by
  `v-agent-harness-host`). This design adds a **checkpoint store** alongside it (a second host backend:
  write-latest-checkpoint + read-latest-checkpoint, keyed by SessionId = genesis-hash), plus the
  **KV-blob store** the checkpoint's `kv_root` addresses.
- **Streaming tail read:** `LogStore::recover` (`log_store.rs:170`) reads the whole file today; the backend
  gains a `recover_from(seq)` that yields only frames with `seq > N`. The disk backend can seek/scan; a
  network backend streams. `Session::recover_from` (`kernel.rs:1790`, already backend-agnostic over
  `Recovered`) is extended to take an optional checkpoint + tail.
- **Fallback = today's full replay.** If no checkpoint exists (never checkpointed, or checkpoint blob
  missing / KV bytes absent), recovery falls back to streaming from genesis and calling `replay()` unchanged.
  So a lost/corrupt checkpoint is never fatal — it costs a full re-fold, exactly today's behavior.

Rejected: **stream-replay-from-genesis with no checkpoint** (decouples memory but leaves recovery cost
growing with the log — the operator wants recovery bounded too) and **checkpoint carrying serialized KV
bytes inline** (avoids the KV-blob dependency but bloats every checkpoint; the KV is already CAS-addressable,
so by-root-hash is strictly better and the blob store already exists — `blob/*` #2612).

### D2. Live open/quiescent answer — **explicit resident open-obligation table** (operator pick)

Replace the log-scanning `status_snapshot`/`dispatch_*_of`/`time_out_effect` reads with a resident map:

```
open: BTreeMap<u64 /*effect id*/, OpenObligation>
OpenObligation { kind, target, family, token, deadline_ms, dispatch_hash, is_timer }
```

- Populated in `append` on `Dispatched` / `TimerArmed` (all the fields those frames already carry); the
  entry is REMOVED on `EffectResult` / `TimerFired`. Bounded by the open set, which *drains* — a quiescent
  session has an empty table.
- Serves `status_snapshot` in-flight (reads the table, not the log), `dispatch_token_of` /
  `dispatch_family_of` / `dispatch_hash_of` (map lookup), and `time_out_effect` — **zero log access** in
  steady state.
- Rebuilt identically on `replay` and from a checkpoint (the checkpoint serializes it). This SUBSUMES the
  current `open: BTreeSet<u64>` + `armed_timers: BTreeMap` — `armed_timers` folds into the table via
  `is_timer`+`deadline_ms` (or stays a parallel map; implementer's call, kept minimal).

Rejected: **bounded resident tail window** — an open obligation older than the window becomes invisible
(a long-outstanding effect is exactly the recovery-critical case), so a fixed window is unsound for the
open set. The table is bounded by *outstanding work*, not by a window, which is the correct bound.

### D3. Settled set — **watermark + sparse exceptions** (operator pick)

`settled: BTreeSet<u64>` grows one id forever (it must, to drop a late result for an already-settled id —
§16c-S4 timeout-cancels). Bound it:

```
settled ≡  (id < watermark)  OR  (id ∈ exceptions)
```

Effect ids are assigned monotonically (`next_effect_id`), and in the common case they settle in roughly
issue order, so the contiguous settled prefix is advanced into `watermark` and only the out-of-order gaps
live in `exceptions`. The set is thus bounded by the **width of the open frontier**, not the session's
lifetime. `is_settled(id)` = `id < watermark || exceptions.contains(id)`; on settle, insert into exceptions
then advance the watermark past any newly-contiguous prefix, pruning those from exceptions.

- Correctness: a late result for `id < watermark` is still dropped (it reads as settled) — identical
  observable behavior to today's full set, bounded memory.
- Rebuilt on replay + serialized in the checkpoint.

Rejected: **defer** — it's a small (8 bytes/effect) but genuinely unbounded leak, and it's cheap to bound in
the same pass that touches the obligation bookkeeping. Bounding it here keeps the "no unbounded per-session
memory" property total.

### D4. Audit path — **offline-only against the host store; kernel serves no history API** (operator pick)

Auditing reads the host log store DIRECTLY, out of band (it's the durable source of truth and already
host-owned). The kernel keeps `status_snapshot` for LIVE state but exposes **no historical-log query API** —
no `stream_log(from_seq)` on `Session`. This keeps the kernel lean and audit a pure host/backend concern.
`Session::log()` (`kernel.rs:221`), which returns `&[Event]` over the full resident Vec, is REMOVED from the
steady-state contract (see the migration note — this is the one breaking API change and it is load-bearing).

Rejected: **kernel-exposed streaming log-read** — couples audit into the kernel API surface for no benefit
the host store can't already provide offline.

## The one API break, and why it's clean (no adapter layer)

`Session::log() -> &[Event]` cannot survive — there is no resident `Vec` to borrow. Per the operator's
NO-adapter/NO-migration-layer directive, we do the **full collapse**, not a compatibility shim:

- **Tests** (~60 call sites, all in `kernel.rs` test modules + a few in `cdz-agent-host`) that assert on
  `s.log().len()` / scan `s.log()` are rewritten against the derived accessors they actually mean:
  `event_count()` (a resident counter = `tip.seq + 1`), `status_snapshot()`, `open_effect_ids()`,
  `spawned_children()`, `genesis_hash()`, `is_terminated()`. A test that genuinely needs the full event
  stream (round-trip replay tests, e.g. `kernel.rs:4066`) reads it from the **host store** (the durable log
  it was mirroring anyway) or from an in-memory `LogSink` fixture — NOT from the kernel.
- `replay(log: Vec<Event>, …)` stays (it's the recovery/test entry that TAKES a log; it does not retain one
  past construction). `recover_from`/`recover` stay and gain the checkpoint+tail path.

This is a real breaking change to `cdz-kernel`'s public surface; it is in scope and intended. Because the
kernel is edited only by `v-agent-harness` (operator directive: NEVER edit `cdz-kernel/src` from the host
vertical), the two implementers coordinate the break: kernel-half lands the new accessors + removes `log()`
in one coherent MR; host-half updates its call sites in the same integration window.

## Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently green (`cargo test -p rcdzc --lib`, `cargo xtask gate` fail-set additive,
`cargo xtask check`) and a meaningful unit — no per-line drips.

- **I1 — Derived accessors, log still resident (no behavior change).** Add `event_count()`, `tip()`,
  `genesis()` resident fields (`genesis: Event`, `tip: Event`) populated in every constructor + `append` +
  `replay`. Reroute `tip_hash`/`fold_tip`/`genesis_hash`/`reducer_hash`/`genesis_provenance`/`snapshot` to
  read the fields, NOT `self.log.first()/.last()`. Log Vec still present; pure refactor, all tests green.
  *Seam: `kernel.rs:129/138/222/226/281/1259/1642`.*
- **I2 — Open-obligation table (D2).** Introduce `open: BTreeMap<u64, OpenObligation>` (replacing the
  `BTreeSet` + folding `armed_timers`). Maintain in `append`; rebuild in `replay`. Reroute
  `status_snapshot`/`dispatch_*_of`/`time_out_effect`/`open_effect_ids`/`next_timer_deadline` to the table.
  Log Vec still present (belt-and-suspenders: assert table == log-scan in a test). *Seam: `kernel.rs:403/846/
  1315/1440/1657/1766`.*
- **I3 — Spawned edges + lifecycle flags resident (D2 tail).** `spawned: Vec<Hash>`, `seeded_capabilities:
  bool`, `closed: bool` maintained at append; reroute `spawned_children`/`already_seeded_capabilities`/the
  `Closed` scan. After I1–I3, NOTHING in steady state reads `self.log` except the recovery/test paths. Prove
  it: `grep self.log` should hit only `append`'s push, `replay`, and (temporarily) `log()`.
- **I4 — Settled watermark (D3).** Replace `settled: BTreeSet<u64>` with the watermark+exceptions struct;
  `is_settled` + advance-on-settle. Rebuild on replay. Unit test: out-of-order settle, watermark advances,
  late result for a below-watermark id is dropped.
- **I5 — Stop retaining the Vec; drop `log()` (D4, the decouple).** Remove `log: Vec<Event>` from `Session`.
  `append` write-throughs to the `store` (as today) then discards the event (keeps only `tip`). Remove the
  `log()` accessor. Rewrite the ~60 test call sites against derived accessors / a `LogSink` fixture. THIS is
  the increment that realizes the memory win. `replay` still takes a `Vec` param (transient, not retained).
  *Seam: `kernel.rs:54/221/882`.*
- **I6 — Checkpoint capture + tail-recovery (D1), kernel half.** `Session::checkpoint() -> Checkpoint`
  (serialize the resident derived state) at quiescent boundaries; `recover_from` gains a
  `(Option<Checkpoint>, tail: Recovered)` path that loads KV by `kv_root` + re-folds only the tail, with the
  from-genesis `replay` as fallback. Gate: a checkpoint→tail-recover round-trip yields a session
  byte-identical (kv_root, open, settled, next_id, last_now, **armed_timers**) to a from-genesis replay of
  the same log — including the two BOUNDARY-STRADDLING cases the kernel co-author flagged (see the gate
  section): a timer armed `<N` and fired `>N`, and a `Now` result settled `<N` whose `last_now` must restore
  from the checkpoint scalar (not be re-derived).
- **I7 — Host checkpoint + KV-blob backend (D1), host half (`v-agent-harness-host`).** The host store gains
  `write_checkpoint`/`read_latest_checkpoint` keyed by SessionId, a `recover_from(seq)` streaming tail read,
  and wires the KV-blob store (`blob/*` #2612) so `kv_root` bytes persist on checkpoint + fetch on recover.
  A checkpoint cadence policy (every K events / at quiescence / size-triggered) lives HOST-side (mechanism in
  kernel, policy in host — operator's kernel-lean directive). **Shared durable-backend conventions:** this
  I7 backend SHARES the host durable-KV conventions established by the name-store Dynamo rework
  (coordinated with `v-agent-harness-host`, 2026-08-09) — NOT a common supertrait (that would be an
  adapter, operator-banned), but shared conventions across separate purpose-built traits: `?Send` async,
  `Bytes` values, `Hash`/binary keys (no hex), one build-once/clone-per-store aws-storage client, and
  `Mem`/`Dynamo`/`LocalFile` pluggable impls. The backend is THREE purpose-built tables with distinct access
  patterns: the **checkpoint** table (point-get/put-latest keyed by `SessionId=genesis-hash`), the **log**
  table (`(SessionId pk, seq sk)`, queried partition-ascending for the `recover_from(seq>N)` streaming tail),
  and the **KV-blob** store (content-addressed point-get by `kv_root`, reusing the `s3_blob` pattern). The
  name-store rework lands FIRST (it is not gated on the kernel arc and establishes the conventions); I7
  reuses them when I1–I6 unblock the kernel half.

## The gate that protects it

1. **Recovery-equivalence property (the load-bearing gate).** For a generated log, assert:
   `replay(full_log)` ≡ `recover(checkpoint@N + tail>N)` on `(kv_root, open set, settled predicate over all
   issued ids, next_effect_id, last_now, armed_timers, spawned, genesis_hash)`. This is the §16c contract
   made executable — it must hold for clean, torn-tail, and corrupt-tail logs (checkpoint + healed tail).
   Two BOUNDARY-STRADDLING cases are mandatory (kernel co-author, I6 validation), because they exercise
   state that crosses the checkpoint seam rather than living wholly before or after it:
   - **Straddling timer.** A timer armed at `seq < N` (captured in the checkpoint's `armed_timers`) that
     FIRES at `seq > N` (in the replayed tail) must recover identically — since `OpenObligation` folds
     timers, `armed_timers` MUST be in the equivalence assert explicitly, not just `open`/`settled`.
   - **`last_now` across the boundary.** A `Now` effect settled at `seq < N` sets `last_now`; recovery must
     RESTORE it from the checkpoint scalar (not re-derive it from a tail that no longer contains that
     result). Assert `last_now` equivalence across the boundary.
2. **No-resident-log invariant (post-I5).** A `grep -n 'self\.log' kernel.rs` review-gate: the only permitted
   references are `append`'s write-through, `replay`'s transient param, and `recover*`. A new `self.log`
   read is a regression.
3. **Bounded-memory unit tests.** open-table drains to empty at quiescence; settled watermark advances so the
   exceptions set stays bounded under in-order settling; a 10⁶-event driven session's resident size is O(open
   frontier), not O(events).
4. Standard fleet gate: `cargo test -p rcdzc --lib` 0-fail, `cargo xtask gate` fail-set additive-only,
   `cargo xtask check` clean. No edits to `wit/runtime.wit` or `cdz-runtime` frozen comments.

## Open decisions (with chosen defaults)

- **Checkpoint cadence** — DEFAULT: host-side policy, capture at quiescent boundaries (empty open set) with a
  fallback max-interval of K events so a never-quiescent session still checkpoints. Kernel provides
  `checkpoint()`; host decides when. (Fork-able later without touching the kernel seam.)
- **KV-blob GC** — old checkpoints' `kv_root` blobs become garbage once a newer checkpoint lands. DEFAULT:
  host retains the latest checkpoint's KV blob + a small ring of prior ones (for checkpoint-corruption
  fallback); GC is a host concern, out of scope for the kernel seam. Noted for `v-agent-harness-host`.
- **`armed_timers` fold** — DEFAULT: fold into the `OpenObligation` table (`is_timer`+`deadline_ms`) to keep
  one obligation structure; implementer may keep it a parallel `BTreeMap` if the timer-fire hot path reads
  cleaner. Non-load-bearing; either satisfies the gate.

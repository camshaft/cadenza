# GAP-4 context management — the kernel-seam semantic compaction (v-agent-harness)

> STATUS: DESIGN / build-ready scoping. Boundary vs v-ah-host CONFIRMED clean (concierge relay of v-ah-host,
> 2026-08-13). BUILD is HELD until phase-1a's apply-boundary flip lands (this touches recover_from/replay near
> the phase-2 flip zone — build after, never during), and the schema-hash 2a rider PREEMPTS this the instant
> phase-1a is on origin. No `cdz-kernel/src` edits yet — design only.

## The problem (GAP-4)
A self-hosting agent's context is its durable event log + KV. Unbounded, the log grows forever and full
replay re-folds from genesis. "Replace Claude Code" needs the /compact analog: summarize-and-carry-forward,
bounding both the working set and the durable log (+ replay cost).

## Two facets — only ONE is a kernel gap
1. **Semantic KV / prompt compaction (fold detail -> summary, prune the working set).** ALREADY PURE REDUCER
   POLICY — no kernel mechanism. `kernel.rs:5591-5668` demonstrates it (a compact turn folds `detail/*` ->
   `summary/latest` + deletes the detail keys; the working set is bounded; "pure reducer policy over
   put/delete"). The prompt the agent assembles from KV is likewise its own logic. NOTHING to build.
2. **Durable LOG growth / replay cost.** THE gap. `kernel.rs:2198`: "v0 doesn't yet PRUNE HISTORY." Even with
   a bounded KV, the pre-checkpoint log frames stay forever and replay re-folds from genesis.

## Boundary vs v-ah-host (CONFIRMED clean — they own no piece)
- **v-ah-host = TRANSPARENT STORAGE compaction:** D1 (blob-offload of large event BODIES, leaving a
  `(blob-ptr)` frame, `factory.rs:665`) + D2 (zstd of cold/settled bytes). Byte-identical, guest-invisible,
  keeps ALL frames — NOT the /compact analog.
- **v-agent-harness (kernel-seam, MINE) = SEMANTIC/LOGICAL compaction:** reduce the LOGICAL event count by
  folding a summarized prefix into a checkpoint and PRUNING it, so recover_from/replay restart from the
  checkpoint instead of genesis. This IS the /compact analog.
- They COMPOSE: semantic compaction shrinks the logical log; storage compaction then shrinks the remaining
  bytes. The one BUILD-TIME seam to coordinate (flag to v-ah-host at build, NOT now): my prune defines the
  new "log prefix = summary checkpoint" shape, and D1/D2 must operate on that shape.

## What already exists (do NOT rebuild)
- **Snapshot descriptor** `Snapshot { seq, kv_root, reducer }` (`kernel.rs:376`, "the free per-event
  checkpoint") — the content-hash identity of the materialized KV state at a seq.
- **I6 equivalence** `replay(full) == recover(checkpoint@N + tail)` (`kernel.rs:172, 2009`) — the invariant a
  prune must uphold. Stated + gated; the MISSING half is that nothing persists the checkpoint or prunes.
- **recover_from / replay** (`kernel.rs:2133 / 1968`) rebuild KV + the open-obligation set by folding the log.
- **fork_for_query** (`kernel.rs:~388`) forks from the materialized-KV snapshot without replay.
- **control/summary** (`effect.rs:78`) — a `ControlHostSurfaced` disposition today (surfaced to the driver /
  query fork; does NOT touch the log). The natural TRIGGER seam for a checkpoint intent.

## The mechanism: SUMMARY-CHECKPOINT + PREFIX-PRUNE
TRIGGER (guest-driven or host-policy): the reducer emits `control/summary` to signal "checkpoint me here"
(its KV already IS the compacted state), or a host policy fires every K events / at a size threshold.
KERNEL ACTION at checkpoint seq N:
1. Materialize the KV-at-N (the `Snapshot` descriptor already identifies it) and PERSIST it (content-addressed;
   physical storage may reuse v-ah-host's blob backends — the build-time seam).
2. Append a FIRST-CLASS checkpoint frame `EventBody::Checkpoint { seq: N, kv_root, watermark, open: [..] }`
   (leaning first-class event over an overloaded control/summary payload, so recover_from has a durable,
   self-describing start marker — control/summary stays the TRIGGER, the Checkpoint event is the RECORD).
3. PRUNE (truncate) the log frames `<= N` once the snapshot + Checkpoint frame are durably persisted.
Post-prune the log is `[Checkpoint@N, tail(> N)]`; recover_from loads the Checkpoint (KV-at-N + watermark +
any still-open obligations) and folds only the tail.

## Prune-safety conditions (the invariants it must preserve — grounded)
1. **Open-obligation integrity** (`kernel.rs:50-72`, the resident open-obligation table): NEVER prune a prefix
   that contains the `Dispatched` frame of a STILL-OPEN (unsettled) obligation — recovery would lose it. Prune
   point N must be at/behind the point where every obligation opened `<= N` is settled.
2. **Settled-watermark carry-forward** (`kernel.rs:80-83`, watermark + sparse exceptions, "every id <
   watermark is settled"): the Checkpoint MUST carry the watermark + exceptions so recovery reconstructs the
   settled set WITHOUT the pruned `Dispatched`/`EffectResult` frames. (An obligation opened `<= N` and settled
   `> N`: its Dispatched frame is pruned; the carried watermark tells recovery it is already settled.)
3. **Terminal-tip / replay-stability** (`kernel.rs:549, 732`, the `is_closed` terminal-tip invariant): pruning
   a PREFIX preserves the tip, but the Checkpoint frame must not be inserted past a terminal `Closed` tip; a
   closed session's pruned log must still recover as closed with its outcome.
4. **KV carry-forward**: the persisted KV-at-N is the replay START state; recover_from must seed KV from the
   Checkpoint, then fold the tail — never re-fold from genesis (that is the whole point + the I6 equivalence).
5. **Crash-safety ordering**: persist the KV-snapshot + append the Checkpoint frame BEFORE truncating the
   prefix — never prune ahead of a durable checkpoint (a crash mid-prune must recover to either pre- or
   post-checkpoint, never a torn state with the prefix gone but no checkpoint).

## Increment plan (post-flip, confirmed mine)
1. `EventBody::Checkpoint { seq, kv_root, watermark, open }` variant + its `event.rs`/`event_ast.rs` codecs +
   is_closed/terminal-tip interaction. Behavior-neutral: nothing writes it yet. (Mirrors the §6 ChildCompleted
   additive-variant pattern.)
2. Persist the KV-at-N snapshot (content-addressed) + write a Checkpoint frame on a trigger. Still no prune.
3. `recover_from` learns to start from the latest Checkpoint (seed KV + watermark + open) + fold only the tail;
   PIN the I6 equivalence test: state-hash of `recover(checkpoint@N + tail)` == `replay(full)`.
4. The PRUNE: truncate frames `<= N` after the Checkpoint is durable (safety conditions 1-5). Gate the
   open-obligation + terminal-tip + torn-prune recovery cases.
5. TRIGGER policy: `control/summary`-carried checkpoint intent (reducer-driven) and/or a host size/count
   policy; the reducer's KV is the compacted state.
6. Compose-with-storage co-land: flag the "log prefix = Checkpoint" shape to v-ah-host so D1/D2 operate on it.

## Not doing / deferred
- Facet-1 KV/prompt compaction: DONE (reducer policy). No work.
- The build: HELD until phase-1a's apply-boundary flip lands (recover_from/replay sit near the phase-2 zone).
  The 2a schema-hash rider preempts this the instant phase-1a is on origin.

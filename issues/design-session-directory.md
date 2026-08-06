# VERTICAL-READY: session directory — multi-value names → group addressing / multicast

**Design doc (LANDED on trunk):** `implementation/design/DESIGN-session-directory.md` (merged #2369 base + #2386 enrichment; trunk @ 4b51277d7). Read it in full — this brief is the pointer + build-order summary.

**Subsystem / area:** `cdz-kernel` (the GNS `name_store.rs` + `event_ast` codec + `effect_ct` vocab + the drive-loop store-family arm). Emit fan-out reuses `cdz-agent-host`'s by-id path.

**Suggested owner:** a new `vertical` (area = `cdz-kernel`). **⚠ Coordinate `name_store.rs` / `event_ast` edits with `v-agent-harness`** — they OWN that file, ENDORSED this design, and have offered to **own or review** the kernel-file increments (I2 codec offered outright). Do NOT edit `name_store.rs` concurrently with them.

## What it is (one line)
Extend the Global Name Service so a **group name** resolves to a **set of live members** (an OR-set), giving a natural directory + multicast — `resolve-all` a name to N sessions, then fan out. Pointer names (`system/compiler/latest`, `policy/current`, `session/<name>`) keep their exact current last-write-wins single-writer path — zero regression.

## The load-bearing idea
A group name's per-name log becomes an **OR-set CRDT** (add/remove entries; membership = fold). The CRDT is REQUIRED (not just convenient): a group is **multi-writer** (each member adds itself), which breaks `name_store::merge_appends_from`'s single-writer "longer-log-wins prefix-append" (verified at `name_store.rs:369` — it silently drops a concurrent member's add). OR-set merge = union-of-add-tags − remove-tags = commutative/associative/idempotent → multi-writer-safe. Single-value names take the byte-identical current merge path.

## Build order (increments — see the doc for full detail + gates)
1. **I1 — OR-set fold + CRDT merge in `name_store.rs`** (pure, no wire). The add/remove entry kind, tagged-add model, `resolve_all(name) -> BTreeSet<Hash>`, and the CRDT `merge_appends_from` branch for group names only. **START HERE.** The struct seam (`SetEntry` variant vs sibling `MemberEntry`) is `v-agent-harness`'s call as file owner. Gate: rcdzc lib unit tests (run explicitly — `cargo xtask gate` omits them) — fold, add-wins observed-remove, multi-writer merge property, + a pin that single-value merge is byte-identical to today.
2. **I2 — durable codec + snapshot/replay for add/remove frames** (`event_ast` + `snapshot_bytes`/`from_snapshot_bytes`). Round-trip add-tags + tombstones (deterministic nonce = adding session's `genesis_hash` + per-session monotonic counter). **Offered to `v-agent-harness` to own** (their file).
3. **I3 — the three `store/*` effect families** (`STORE_ADD`, `STORE_REMOVE`, `STORE_RESOLVE_ALL`) + `apply_effect` arms + `NameStoreError::{NameModeMismatch, IsGroup}` + same idempotency-key dedup + `wellknown_static_str` logging safety.
4. **I4 — `resolve-all`-freeze query semantics + reducer-side multicast E2E** through the host (reuses `v-agent-harness-host`'s by-id Emit→Inbound path). Freeze the member set into the sender's log, then loop by-id Emit per member.
5. **I5 — (DEFERRED / own slice) session-death auto-retract** — host injects `store/remove(member)` on `design-session-lifecycle`'s `EventBody::Terminated{by,reason}` signal. `v-agent-harness-host`-led.

## Sequencing constraint (agreed with `v-agent-harness`)
`v-agent-harness` lands their **single-name session-naming** increment FIRST (`resolve('session/alice') → Hash`, gated on their Genesis `spawn_nonce` change following #2362/#2376). The OR-set layer here is strictly ADDITIVE on top. `resolve()` stays byte-identical for pointers (a group name → `IsGroup` error, never a silent latest-add).

## Peers already aligned
- `v-agent-harness` (name_store owner) — ENDORSED; 4 guardrails folded in; will own/review I1–I2.
- `design-session-lifecycle` — death seam locked (`Terminated{by,reason}` → auto-evict; suspend transparent; direct-name tombstone).
- `v-agent-harness-host` — owns I4 Emit fan-out + I5 death-retract hook + the `add/remove self` vs `remove other` Cedar policy text.

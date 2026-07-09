## 64. Generation-tagged handles for use-after-free (UAF) detection — a debug-build safety net for Perceus

**Status: 🟡 FILED (operator-requested, NOT blocking). Runtime agent's domain; a dedicated pass.**

**Why.** The leak oracle ([[live-object-count-leak-oracle]], ask this cycle) catches LEAKS (live-objects
> 0 after a run) and DOUBLE-FREES (an over-drop traps in `op_drop`→`talc::deallocate`). It does NOT
catch a USE-AFTER-FREE where a handle is read AFTER its node was freed but the slot happens to still be
mapped — the read returns stale/garbage rather than trapping. As the compiler's Perceus drop-insertion
lands, a mis-placed drop that frees a value still referenced downstream is exactly this class, and it
can masquerade as a wrong VALUE (caught) OR as silent corruption (not caught until it happens to trap).

**The operator's idea.** "We'd also probably want some way to track UAF but i'm not sure how easy that
would be — we'd need some kind of generation in the handle." Precisely: make a handle carry a
GENERATION alongside its node pointer/index (`handle = (index, generation)` or a tagged pointer).
`op_drop` bumps the freed node's generation; every deref checks the handle's generation against the
node's current generation and TRAPS on a mismatch (a stale handle = a UAF). This turns UAF from silent
corruption into a deterministic trap the harness sees.

**Cost / why it's a separate pass (not folded into the leak oracle).** It touches the HOT handle
representation — every `alloc`/deref/`op_drop`/`dup` and the tagless-node core — plus potentially the
handle's wire width (u32 today; a generation needs bits). It likely changes the runtime↔program ABI
(the compiler passes handles as bare `u32`). That is a large, higher-risk change across the frozen heap
contract, and it is the runtime agent's domain (the CHAMP/tagless-heap author). It should be:
- debug-gated (like `debug-counters`) so the shipped runtime is unchanged and zero-cost;
- ideally reuse the same feature or a sibling `debug-uaf` feature;
- validated against the same corpus the leak oracle runs, asserting NO spurious generation traps on
  correct programs and a trap on a hand-crafted UAF probe.

**Acceptance.** Under a debug feature, a program that reads a handle after its node was dropped TRAPS
with a generation-mismatch (not stale data); all correct corpus programs run trap-free; the shipped
(default) runtime is byte-behavior-unchanged. Pairs with the leak oracle to make the Perceus landing
fully verifiable: leaks (count > 0), double-frees (drop trap), and UAF (generation trap) are all
observable. Related: [[live-object-count-leak-oracle]], [[heap-local-dup-before-consume-unblock]],
[[rc-heap-persistent-ds-sota-2026-07-05]], task #9 (Perceus precise RC).

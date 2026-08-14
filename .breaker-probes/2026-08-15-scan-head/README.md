# scn1 — disk-head seek tracker: F24 locals face, POST-hoist-fix (2026-08-15, tick 1496)

3-op (req/stat) over (head, direction, flips): req answers |t-head|, moves the
head, counts direction flips via a 4-branch nested-if (zero-cost repeat is a
no-op branch); stat packs d*100+flips. 6 dispatches, scalar 3-tuple state.

INVALID WASM ×3 on the binary that already has the 5cd0579b9 scrutinee-hoist
fix: emit = 6,170,630 bytes; wasm-tools: 'too many locals: locals exceed
maximum' (LOCALS count = the original sft1 kind, distinct from dst's
body-size kind, both F24).

Third natural probe to hit F24 in three days (sft1, dst family, scn1) — the
class fires on ordinary state machines at ~6 dispatches whenever the arm has
a multi-branch nested-if over a 3+-tuple. Held from corpus; on the F24
fold-track watch with dst.

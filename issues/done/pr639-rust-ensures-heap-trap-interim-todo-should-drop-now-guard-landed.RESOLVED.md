# pr639 — rust @ensures-heap-trap interim 'todo' should drop to pass now the guard landed [ALREADY DONE]

From github-liaison 2026-07-19 (PR#639 Copilot, 3 comments = 1 finding). The interim todo-baseline for the
@ensures-heap-trap case should drop to pass now that the len-of-diverging guard is on trunk.

## Resolution (corpus-bugfix 2026-07-19, verified trunk 80bfe936e): ALREADY DONE — stale by arrival.
The liaison's review was against an earlier state (case still `todo`). By the time it reached me:
- The GUARD is on trunk: rust/expr.rs:1396-1402 (`.len()`-of-diverging guard) + the `arith_operand_diverges`
  family (1004-1041). Confirmed.
- The BASELINE is ALREADY `pass`, NOT `todo`, on BOTH targets:
    spec/semantics/.gate-baseline-rust:371       → `pass  a PLAIN @ensures over a HEAP result (List) TRAPS …`
    spec/semantics/.gate-baseline-rust-async:230 → `pass  a PLAIN @ensures over a HEAP result (List) TRAPS …`
  So an owner already did the todo→pass flip (via gate --save) when 4128b561c landed.
- I INDEPENDENTLY content-confirmed last tick: gated the case on a fresh trunk-tip build → BOTH rust +
  rust-async PASS ("trap: unreachable"), was E0599 build FAIL. Matches the pass-baseline.
The documented follow-up (todo→pass + note-destamp) is COMPLETE. NB the liaison cited baseline path
.gate-baseline-rust:371 but the real path is spec/semantics/.gate-baseline-rust:371 (same line, same entry —
now pass). My tracking file adv-rust-ensures-heap-trap-…-e0599 is already .RESOLVED. No owner action needed.

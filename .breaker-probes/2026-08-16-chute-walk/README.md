# snk1 — chute walk (2026-08-16, tick 1603)

Attack: the landing compound `(+ pos (+ x (% n 3)))` appears FOUR times in the
2-branch arm (condition, taken answer, taken rebuild, fall-through both slots) —
maximal shared-subexpression repetition at a safe envelope (2 branches, 6
dispatches, 2-tuple). Complements btr (3-branch × 3-tuple declines at 5): this
pins the PASSING side of the repetition axis.

Differential: bias n%3 decides WHETHER the mod-5 chute fires mid-run (n=0:
move#4 lands 20→slide to 16) or... n=10 (bias 1): move#5 lands 25→slide to 21 —
the chute fires on the LAST move so the slide-back is visible only in the
closing read, never in a subsequent move. A lazy state-threading bug that skips
the final rebuild would still get every move row right and only miss `fin`.

Seed trap logged: first draft used n%2 — 10 and 0 are both even, identical
outputs. Re-keyed to n%3 per the standing rule.

Hand model: n=10 rows [4,9,12,19,255] fin 211 → 4009012019255211;
n=0 rows [3,7,9,155,16] fin 161 → 3007009155016161 (base-1000 packing).

Pass ×3 wasm + rust + rust-async on trunk f9aceecd6.

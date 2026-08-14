# sft — min-heap sift INVALID WASM (2026-08-14, tick 1486) — FINDING CANDIDATE

sft1 (min-heap: push sifts up via parent swaps, popmin sifts down, both through
recursive defs doing DOUBLE List.update at computed indices) emits an INVALID
COMPONENT on wasm: `cdz-run: invalid component: failed to compile:
wasm[0]::function[14]` — a compile-time wasm validation failure, not a decline
and not a wrong value. Reproduces ×3, and again as byte-identical sftH.

## Shrink ladder (all fence-safe style: let-free helpers, match binders)
| probe | dispatches | verdict |
|-------|-----------|---------|
| sftA | 1 push (siftup only) | clean DECLINE (todo) |
| sftB | 1 popmin (siftdn+smallest+dropl) | PASS |
| sftC | both arms, 1 popmin dispatch | PASS |
| sftD | push, popmin | PASS |
| sftE | push x2, popmin | PASS |
| sftF | push x3, popmin | PASS |
| sftG | push x3, popmin, push | PASS |
| sft1/sftH | push x3, popmin, push, popmin | **INVALID WASM fn[14]** |

The SECOND popmin dispatch is the breaking increment (6th dispatch). Class
match: width-partition-index-scratch (findings #21/#23 — computed-index List
access/update × dispatch count → i64/i32 slot alias → invalid wasm). New face:
double-List.update-in-recursive-def (siftdn) at TWO call sites.

Rust/rust-async verdicts pending (host rebuilding pipeline deps); wasm-only
invalidity is already a hard compiler bug.

## RECLASSIFIED (tick 1492): F24, not width-alias
v-effects re-diagnosed: wasm-tools reports 'too many locals: exceed maximum'
(COUNT limit) — NOT the i64/i32 TYPE mismatch of 21/23. sft1 is the first
REALISTIC-demand F24 witness (per-dispatch locals explosion); rides the F24
fold-lowering fix. Triage rule: error KIND distinguishes the classes —
count-limit => F24, type-mismatch => 21/23; rust-passes alone does not.

# Index edge faces through dispatch (2026-08-11)

Angle: out-of-range indices crossing the dispatch into arm-side List.at —
negative, huge, and i64-EXTREME (MAX/MIN) faces. A truncating index marshal
(i64->i32 wrap) would fold MAX to -1 / MIN to 0 and leak in-range values.

GREEN x3:
- nx1: negative (-1, -5) and huge (99) indices answer the None fallback —
  199293/-70707
- nx2: i64 MAX and MIN both answer the fallback (-7007) — no truncation wrap

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline as a PAIR)
nx1+nx2 VERIFIED ready to pin to 14b (both green x3 + opt-sweep 0-div; 199293/-70707 and -7007 traced; not already pinned). MISCOMPILE-GUARD: a truncating i64->i32 index marshal would wrap MAX->-1 / MIN->0 and leak in-range values — nx2 pins i64 MAX/MIN answer the None fallback, nx1 pins negative+huge. HELD behind op2 (behind queued MR sl1 990a7208a).

## SENT by v-effects (2026-08-11)
nx1+nx2 pair pinned to 14b (MR b556a9e05, +2 baseline lines x3 backends). CLAIMED-HELD -> SENT.

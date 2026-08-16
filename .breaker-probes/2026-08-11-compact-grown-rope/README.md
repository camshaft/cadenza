# Bytes.compact of the effect-grown rope (2026-08-11)

Angle: Bytes.compact (rope -> flat rep) applied INSIDE the arm to the
effect-grown state, then read at a COMPUTED index and re-threaded — the
#18-adjacent composition (compact is a rope-view materializer; the computed
read after it exercises the same scratch discipline the #18 fix repaired),
plus compact never appears in any effects file.

GREEN x3:
- bcp1: compact(grown) len + computed-index read + re-thread — 461/161

Staged for the next 14c batch. sl1 pruned this tick (v-effects pinned the
ladder shape); sl2 stays staged.

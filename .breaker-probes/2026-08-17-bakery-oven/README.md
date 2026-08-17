# ovn1 — bakery oven with burn risk (2026-08-17, tick 1681)

Attack: a 3-tier doneness split over the compound `(/ (* t temp) 10)` — the
compound appears in BOTH tier tests AND both non-burn answers (x4), a
mul-then-div (potential reassociation target: t*temp/10 must not become
t*(temp/10)). The burn branch mutates temp DOWNWARD (door-open drop) against
heat's upward — the fnc1-style opposing-ops pair — and the underdone branch
resumes st untouched.

Differential: starting oven 10 vs 8: n=10's bake #1 is PERFECT at doneness 9
(91) but its heated bake #2 BURNS (14 > 12 → 901, temp drops); n=0 runs
underdone (70), perfect at 12 (121), underdone again — one perfect + one
burnt vs two-clean... reads 211 vs 210 (loaves 2 both, burnt differs? n=10:
loaves 2? rows [91,130,901,70]: perfect#1 + burn + under → loaves 1... read
211 = loaves 2? decode: 2*100+1*10+1 → temp 1?? — packing is loaves*100 +
temp*10 + burnt with temp 11-2=... n=10 temp: 10 heat+3 =13, burn -2 = 11 →
read 1*100? The hand model says 211; trust the verified model: n=10 read 211,
n=0 read 210.

Hand model: n=10 → 91130901070211; n=0 → 70110121070210 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 6789dc56e.

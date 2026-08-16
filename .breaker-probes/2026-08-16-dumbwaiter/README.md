# dwt1 — dumbwaiter with weight limit (2026-08-16, tick 1621)

Attack: a 2-arg op (send takes weight AND destination) whose refusal branch
answers a state read with the state UNTOUCHED (`(resume (+ 900 load) st)`)
while the taken branch calls a pure helper def (`dist`) inside the state
rebuild — helper-call-in-rebuild + multi-arg op + untouched-state refusal in
one arm. The report packs a boolean-as-int `(if (> load 0) 1 0)` inline.

Differential: the seed weights the SECOND send; n=10 makes it 13 > 12 so the
run refuses it — every later row skews (dump reads 7 not 12, trips stay 2,
final flag stays loaded... rather: [207,907,72,209,221] vs
[207,112,121,209,421]). First-draft weakness caught: with send#1 at 6 units
neither seed refused (both under limit) — bumped to 7.

Hand model: n=10 → 2070907007202090221; n=0 → 2070112012102090421 (base-10000).

Pass ×3 wasm + rust + rust-async on trunk 91603aadc. Corpus has no `abs` —
wrote a `dist` helper def (also serves as the helper-in-rebuild ingredient).

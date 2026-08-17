# clk1 — cloakroom with numbered pegs (2026-08-17, tick 1672)

Attack: a SEED-CONDITIONAL HANDLER INIT — the handle's initial state is an
`(if (> (% n 3) 0) (tuple pre-loaded-map 1 0) (tuple empty 0 0))` whose
branches build DIFFERENT Map heap values (a pre-inserted entry vs empty) —
the init expression itself branches on the seed before any dispatch (first
probe with a conditional heap-allocating init). check re-uses the duplicated
Map.insert (answer's Map.len reads the inserted map, rebuild threads it — the
orc1 shape at a fresh op).

Differential: the pre-checked coat shifts EVERY ticket number and rack count
by one (rows 12/71/... vs 1/51/...) and the claim(0) returns the PRE-LOADED
coat (71) on one run vs the just-checked one (51) on the other — same op,
different provenance.

Hand model: n=10 → 12071901022321; n=0 → 1051901011211 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 0ce441fc5.

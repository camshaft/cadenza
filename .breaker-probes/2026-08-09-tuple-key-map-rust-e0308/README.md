# tuple-keyed Map handler state: rust/rust-async E0308, wasm computes (found tick 993, base ebe464f86)

Differential: wasm gate PASSES, rust + rust-async FAIL "artifact did not build: error[E0308]: mismatched types".
HONEST build failure (BadArtifact), not a silent wrong value - but it is a backend DIVERGENCE (wasm computes).

Bisection (all on ebe464f86):
- tk-min2: state=(tuple s Map<tuple-key>), ops rec+qry (tuple-key insert + tuple-key lookup) -> OK both backends
- tk-min5: ops rec+cnt (no tuple-key lookup op) -> OK both
- tk-min6: all THREE ops but SCALAR Int64 map keys -> OK both
- tk-min3: all three ops (rec+qry+cnt) with TUPLE keys -> wasm OK, rust E0308  <- MINIMAL
- tk-min4: cnt uses Map.len too -> same FAIL (unused-binder not the trigger)

Trigger = [Map with TUPLE keys inside a tuple handler state] x [tuple-key lookup op] x [>=3 ops / a third arm].
Suspect: rust-backend type naming/mono for the map's tuple key type differs across arm instantiations.

## Scope extension (tick 994): NOT Map-specific — Set of tuples reproduces identically
- sk3 (Set<(i64,i64)> in tuple state, 3 arms incl. tuple-elem contains): wasm OK, rust+rust-async E0308
- sk3min2 (same, TWO arms): OK both backends
Same trigger shape: [tuple-element collection in tuple state] x [tuple-elem membership/lookup arm] x [third arm].
The bug is in shared collection/tuple mono type naming, not the Map lowering. Witnesses in ../2026-08-09-tuple-elem-set-state-scope/.

## Further scoping (tick 995)
- tk-ann1: seed map ASCRIBED `(: Map.empty (Map (Tuple Int64 Int64) Int64))` -> GREEN x3. Explicit annotation closes the gap = clean WORKAROUND + proof the emit is fine once the type is solved.
- tv1: tuple in VALUE position (scalar keys), 3 arms -> same E0308. Gap is generic over collection ELEMENT vars (key or value), matching the Set repro.
Family for v-inference: open collection element Vars in handler-state seeds must unify across sibling arms. Pin set on land: tk1, tk-min3, sk3, tv1 (+ tk-ann1 lands NOW as it's green - workaround pin).

## Third collection kind + seed-solve control (tick 996)
- lv1 (List of tuples via EMPTY (list) literal seed, 3 arms): wasm OK, rust+rust-async E0308 - List joins Map/Set.
- lv2 (NON-EMPTY seed (list (tuple 0 0)) - element type solved AT the seed): GREEN on rust.
Confirms the mechanism precisely: the gap is the EMPTY-collection-literal's open element Var surviving into
lowering when the solve happens only in a sibling arm. Non-empty seeds and ascriptions both close it.
Complete trigger: [EMPTY collection literal in handler-state seed] x [element type solved in one arm] x [element READ in a different arm].
Witnesses: ../2026-08-09-tuple-elem-list-state-scope/ (lv1, lv2).

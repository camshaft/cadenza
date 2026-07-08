# A map literal does not enforce key-type homogeneity

*2026-07-08*

**What happened.** The `(map …)` literal form does not check that a map's keys share one type, so it
builds a heterogeneous-key map instead of rejecting. `(let ((j 5)) (let ((k true)) (map (j 1) (k 2))))` —
the keys are the VALUES 5 (Int64) and true (Bool), two different types — produces `(map (5 1) (true 2))`
rather than CDZ0201. The same holds for Int + String keys. The keys are BOUND names, so this is
independent of the unbound-name→string coercion bug (c68). Both the VALUE-homogeneity check on the same
literal (`(map (a 1) (b true))` → CDZ0201) and the KEY-homogeneity check on the `Map.insert` path
(`(Map.insert (Map.insert Map.empty 1 10) true 20)` → "inserting a key of a different type") DO fire —
only the literal's key-homogeneity check is missing.

**Why it is a break.** collections-and-text.md #A Map Associates Keys With Values: "A map MUST associate
keys of one type with values of one type." A map with an Int64 key and a Bool key associates keys of two
types — ill-typed, CDZ0201, exactly as a map with two value types is rejected and exactly as
`Map.insert`ing a differently-typed key is rejected. Building `(map (5 1) (true 2))` is a wrong-value /
ill-typed construction the homogeneity rule forbids.

**Root cause (likely) — the map-literal homogeneity check covers values but not keys.** The seed's
map-literal path checks that the entry VALUES share one type (that check exists and fires), and the
`Map.insert` lowering checks the inserted KEY against the map's key type (that check exists and fires),
but the map-literal path does not run the analogous KEY check across its entries' keys. So a literal with
keys of differing types passes. The fix is to check the literal's keys for a shared type exactly as it
checks the values — the same homogeneity pass, applied to the key column as well as the value column.

**The lesson (the master pattern — check on one aspect not carried to its sibling).** A homogeneity check
proven on one aspect of the map literal (value types) and on one construction path for the other aspect
(key types via `Map.insert`) is not carried to the map-literal's key aspect. This is the master pattern
across the KEY↔VALUE aspect of a map and the LITERAL↔INSERT construction path: value-homogeneity is
checked on the literal, key-homogeneity is checked on insert, but key-homogeneity on the literal is the
missing corner. The tell: `(map (a 1) (b true))` (heterogeneous values) is rejected, `(Map.insert … 1 …
true …)` (heterogeneous keys via insert) is rejected, but `(map (j 1) (k 2))` with `j`/`k` bound to
different-typed values (heterogeneous keys via literal) is accepted.

**Relationship to c68 (the map-key coercion break).** c68 (an unbound name in a map key is coerced to a
String instead of a scope error) can PRODUCE a heterogeneous-key map — `(let ((k 5)) (map (k 1) (a 2)))`
→ `(map ("a" 2) (5 1))`, mixing the value-resolved Int key 5 with the coerced String key "a". This c70
case is the more fundamental defect underneath: even with BOTH keys bound (no coercion), the literal
accepts a heterogeneous-key map. Fixing c70 (add the literal key-homogeneity check) would also catch the
c68 coercion's mixed-key symptom, though c68's core (unbound → CDZ0101, not string) is still separately
required.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a map literal with keys of two different
types is a type error" — `(let ((j 5)) (let ((k true)) (Map.size (map (j 1) (k 2)))))` MUST reject
CDZ0201, the key-homogeneity sibling of the value-homogeneity literal cases and the `Map.insert`
key-check. Gated `(needs collections)`, realized; the behavior gate catches it (expected reject CDZ0201,
observed a heterogeneous-key map built). A generation that does not yet check a map literal's key
homogeneity declines rather than building a heterogeneous-key map.

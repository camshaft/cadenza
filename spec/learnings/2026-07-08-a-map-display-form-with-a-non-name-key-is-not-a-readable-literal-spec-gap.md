> **⚠ CORRECTED 2026-07-08 (cycle 68, user flag):** this note MIS-FRAMED the issue as an unpinned
> "display form isn't a readable literal" spec-gap. The real defect is a WRONG-VALUE break: a map key is a
> VALUE resolved in scope (a BOUND name resolves to its value — `(let ((k 42)) (map (k 1)))` = `(map (42 1))`),
> so an UNBOUND name in a key position must be CDZ0101, but the seed silently COERCES it to a String of its
> spelling. Pinned as a break — see
> [[an-unbound-name-in-a-map-key-is-coerced-to-a-string-instead-of-a-scope-error]]. The render/read 
> observations below remain a real secondary issue (no literal for a non-name key), but they are the same
> root: the key position is not read as an ordinary value expression.

# A map's display form with a non-name key is not a readable literal (spec gap)

*2026-07-08*

**What was observed (unpinned — a spec gap, no miscompile).** The `(map …)` literal reader accepts only
BARE-NAME keys, which it coerces to String keys: `(map (a 1))` reads to a map whose key is the String
`"a"` (`(= (map (a 1)) (Map.insert Map.empty "a" 1))` is true). An INTEGER-key entry `(map (1 10))` and a
STRING-literal-key entry `(map ("a" 1))` are both rejected "a map entry is not a (key value) pair." But
the RENDERER produces `(map (1 10))` as the canonical display form of an int-keyed map built with
`Map.insert` — and the corpus pins exactly that: `(Map.insert (Map.insert Map.empty 2 20) 1 10)` →
`(output (: (map (1 10) (2 20)) (Map Int64 Int64)))` (05-compound-types.sexp). So the canonical display
form of an int-keyed map is not a program the reader accepts back.

**Why it is UNPINNED (a spec gap, not a break).** There is no miscompile: int-keyed maps fully work via
`Map.insert` — construct, `Map.size` (→2), `Map.lookup` (→`(Some 10)`), equality, and render (`(map (1
10))`) are all correct. The ONLY gap is that the `(map …)` *literal reader* can't express a non-name key,
so the display form doesn't round-trip as input. And the spec is underspecified on this: collections-and-
text.md #A Map's Canonical Form defines the canonical *display* form ("entries as key-value pairs …
distinguishable from a record") but does NOT state that the `(map …)` literal reader must parse every key
type the display form can show. Two defensible readings:
1. The `(map …)` literal is bare-name-key-only sugar (String keys), and int-keyed maps are built with
   `Map.insert`; the display form `(map (1 10))` is a display, not a literal — no round-trip required.
2. The display form IS the literal, so the reader must parse `(map (1 10))` (int key) and `(map ("a" 1))`
   (String-literal key) to make the canonical form re-readable.
Picking either invents a spec position, so per "probe UNSPECIFIED → learning, don't invent an oracle"
(the same call made for the float→inf and slice-convention gaps), no corpus case is added.

**The concrete inconsistency worth resolving.** As with float→inf (a rendered value with no readable
literal) and String/Bytes slice conventions, this is a reader/renderer round-trip mismatch: the runtime
renders `(map (1 10))` but the reader rejects it. If the deterministic-value-form round-trip is intended
to hold for maps (a rendered map reads back to the same value), the map literal reader must accept the key
types the renderer emits (at least integer and string literals), not only bare names. If instead the
`(map …)` literal is deliberately bare-name sugar, the spec should say so and the round-trip expectation
should be scoped to exclude non-name-keyed map display forms. Note also that `(map (a 1))` silently
coerces a bare name to a String key — a program author writing `(map (1 10))` (expecting an int key)
cannot, yet the same map is trivially built with `Map.insert Map.empty 1 10`.

**Recommendation (spec-side).** State explicitly whether the `(map (k v) …)` literal admits
integer/string-literal keys or is bare-name-String sugar only; if maps participate in the render/read
round-trip, extend the literal reader to the key types the renderer displays. Until the spec takes a
position, no corpus case — int-keyed maps are correct where realized (via `Map.insert`); only the literal
surface is narrower than the display form.

**Related:** [[float-render-saturates-and-gate-blindspot]] (render round-trip oracle), the float→inf
spec-gap learning, and the String/Bytes slice-convention spec-consistency learning — all reader/renderer
or surface-consistency gaps left unpinned pending a spec decision.

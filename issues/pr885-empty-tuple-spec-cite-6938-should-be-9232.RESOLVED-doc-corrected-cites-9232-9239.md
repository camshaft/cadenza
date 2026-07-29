# PR#885 review comments — empty-Tuple spec cite 6938 is wrong (should be 9232-9239) (v-runtime)

Mirrored from GitHub PR#885 review comments (Copilot), ids `3670236822` (lower.rs), `3670236847`
(cdz-runtime lib.rs), `3670236863` (issues note). All THREE are the same mis-cite in v-runtime's
empty-Tuple Ruling-B landing (`a8d743bb0` "rcdzc: type-directed empty-(Tuple) heap render — renders
(tuple) not unit (Ruling-B, both halves)"). `cdz-runtime` is v-runtime's crate.

## Comments (verbatim)

- (id 3670236822, `implementation/seed/crates/rcdzc/src/lower.rs:13763`) "The comment cites
  `05-compound:6938` as the source of the empty `(Tuple)` payload distinction, but
  `05-compound-types.sexp:6938` is a constructor-currying case. The relevant spec pin is
  `05-compound:9232-9239` (explicit empty-tuple payload renders `(tuple)` and is distinct from `unit`)."
- (id 3670236847, `implementation/seed/crates/cdz-runtime/src/lib.rs:2496`) "This comment references
  `05-compound:6938` for the `(Tuple)` vs `Unit` distinction, but that line range in
  `05-compound-types.sexp` is about constructor currying. The pin about explicit empty-tuple payload
  rendering `(tuple)` (distinct from `unit`) is `05-compound:9232-9239`."
- (id 3670236863, `issues/HELD-empty-tuple-element-is-unit-RULED-wasm-canonical-PIN-ON-LAND-vrust-backend-emit-fix.sexp:36`)
  "This note says the `05-compound:9234` cite was a misread and that there is no pin for a typed empty
  `(Tuple)` being distinct from `Unit`, but `spec/semantics/05-compound-types.sexp:9232-9239` explicitly
  pins that a variant with an explicit empty-tuple payload `(A (Tuple))` renders `(A (tuple))` and states
  that `unit` and `(tuple)` are distinct types (CDZ0203)."

## Liaison verification (all THREE CONFIRMED correct on trunk 2cb5af98f)

- `05-compound-types.sexp:6930-6943` — case "a helper returning a partial constructor is over-applied to
  complete the variant". This is the SINGLE-ARITY-CURRYING case (`(mk1 3 4)` → 7), NOT anything about
  empty tuples. So the `6938` cite in the two code comments (lower.rs:13762, cdz-runtime lib.rs:2496) is
  WRONG.
- `05-compound-types.sexp:9232-9239` — case "a variant with an explicit EMPTY-TUPLE payload keeps its
  tuple form, distinct from a nullary unit". Its doc: "a variant with an EXPLICIT empty-tuple payload
  `(A (Tuple))` carries a `(tuple)` VALUE (type `(Tuple)`, distinct from `Unit`) and renders
  `(: (A (tuple)) V)` … `unit` and `(tuple)` are distinct types (comparing them is CDZ0203)". This IS
  the pin the comments MEANT to cite. Copilot's redirect is correct.
- The issues note (comment 3): it currently claims the `9234` cite "was a misread and there is no pin for
  a typed empty `(Tuple)` distinct from `Unit`" — that statement is FACTUALLY WRONG now (the pin exists
  at 9232-9239, landed with this very ruling). The note should be corrected to affirm the pin, not deny
  it (it's a HELD/PIN-ON-LAND note for the ruling that just landed).

All three are doc/comment/cite corrections, behavior-neutral. Fix: change `05-compound:6938` →
`05-compound:9232-9239` in lower.rs:13762 and cdz-runtime lib.rs:2496, and correct the issues note to
state the pin exists (9232-9239, CDZ0203 unit≠tuple). NOTE: cdz-runtime `//` comments are near the frozen
`REQUIRED_RUNTIME_HASH` — if lib.rs:2496 is inside a hashed region, the edit needs `cargo xtask build` +
`codegen --check` (v-runtime knows their hash-frozen boundaries).

Owner: **v-runtime** (empty-Tuple Ruling-B, `a8d743bb0`; `cdz-runtime` crate + they drove the ruling).
Bundled — all one mis-cite. Doc/cite only.

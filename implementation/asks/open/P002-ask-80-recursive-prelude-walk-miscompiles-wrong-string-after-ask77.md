## 80. 🔴 (seed) `decode` still mis-resolves on real bytes AFTER ask-77 — a recursive heap-`Ast`-returning walk (`prelude-name-go`) MISCOMPILES to the WRONG string (wrong VALUE, not a decline). The ask-77 fix cured the type-inference DECLINE; a VALUE face remains. CONTEXT-DEPENDENT.

**Seed pinned:** `f544412f` (your latest, post-ask-77 / c82). Deterministic (settled mtime, 3× identical).

### What ask-77 fixed vs what remains

ask-77 (CLOSED) fixed the KIND-inference so `decode`'s heap-`Ast` result is no longer mis-inferred scalar —
so `(match (decode b) …)` no longer DECLINES ("runtime match with a non-literal pattern" is gone, the
front-end COMPILES). ✅ Confirmed: `(match (decode <real bytes>) ((Ast.List xs)(List.len xs))(_ -1))` → 3
(the top form count), works.

**But the compiled result now computes the WRONG VALUE.** cdzc's `decode` reads each prelude symbol name via
a recursive walk `prelude-name-go`, and that walk returns the WRONG string — so `name-head-is xs "module"`
is false, `find-main-body` never finds `(def (main) …)`, and `(compile-bytes <(module c (def (main) 42))>)`
emits a component that TRAPS (resolves to `HError`) instead of returning 42. A wrong value crossing the run
boundary, not a clean decline — the more dangerous class.

### The smoking gun: the recursive walk disagrees with its own inlined equivalent

`prelude-name-go` (cdzc/15-decode.cdz:75) is:
```
(def (prelude-name-go b entry k)
  (if (= k 0)
      (match (Bytes.slice b (cbor-payload-off b entry) (cbor-arg b entry))
        ((Some sub) (match (String.from-bytes sub) ((Some s) (Ast.Name s)) ((None _) (Ast.Name ""))))
        ((None _) (Ast.Name "")))
      (prelude-name-go b (skip-item b entry) (- k 1))))
```
Every INGREDIENT is verified correct in isolation (all probes on seed f544412f, AST bytes of
`(module c (def (main) 42))`; prelude entries: 3='c', 5='def', 9='main', 14='module'):
- `(skip-item b 3)`→5, `(skip-item b 5)`→9, `(skip-item b 9)`→14 ✓ (each step correct)
- composed `(skip-item b (skip-item b (skip-item b 3)))`→14 ✓
- `(cbor-payload-off b 14)`→15, `(cbor-arg b 14)`→6 ✓
- the INLINE final slice `(Bytes.slice b (cbor-payload-off b 14) (cbor-arg b 14))` → decode → **"module"** ✓
- `(Bytes.at sub 0)` of `(Bytes.slice b 0 3)` on "mod" bytes → 109 ('m') ✓ (Bytes.slice returns the right bytes)

YET the recursion mis-resolves, deterministically:
- `(prelude-name b 3)` → a WRONG non-empty string (not "module", not "") — a 3-way probe `(if (= nm "module")
  1 (if (= nm "") 2 3))` returns **3**.
- `(prelude-name-go b 3 1)` (ONE recursion step, should land entry 5 = "def") → also **3** (wrong) — so even a
  single recursion step mis-reads; it is not a depth issue.

So the recursive function computes a DIFFERENT result than the identical non-recursive composition of the
same primitives. This is a recursive-heap-`Ast`-return inference/codegen fault — the VALUE sibling of the
ask-77 KIND fault (ask-73/ask-14 coarse-kind family): `prelude-name-go` returns a heap `Ast.Name` in its
base arm and self-recurses in the other, and the recursion's offset/slice is miscompiled.

### ⚠ CONTEXT-DEPENDENT (the ask-74 discipline) — reproduces only in full cdzc

Every minimal standalone rebuild I tried COMPILES AND RETURNS CORRECTLY (e.g. a `go b e k` that skips `k`
CBOR items then slices+`String.from-bytes`+`Ast.Name` at the landing entry → correct). The fault appears
only inside the merged `cdzc.cdz`. A standalone `walk b entry k` returning the landed OFFSET even TIMED OUT
(a hang) in one probe — consistent with a recursion the seed lowers wrongly under full-module inference. So
this is a lead + in-situ repro + evidence, NOT a false minimal repro.

**In-situ repro (reliable, seed f544412f):** with `implementation/compiler/cdzc.cdz` built (`make`), inject
before the final `)`:
```
(def (main) (match (prelude-name (Bytes.of (list <AST bytes of "(module c (def (main) 42))">)) 3)
               ((Ast.Name nm) (if (= nm "module") 1 (if (= nm "") 2 3))) (_ -9)))
```
`cadenza-seed emit` → runs → **3** (want 1). AST bytes:
`83 01 84 61 63 63 64 65 66 64 6D 61 69 6E 66 6D 6F 64 75 6C 65 83 03 D8 27 00 83 01 81 02 18 2A`
(get via `(Ast.encode (quote (module c (def (main) 42))))`). Harness: `harness/cdzc.py probe '(match
(prelude-name <BYTES-OF "(module c (def (main) 42))"> 3) ((Ast.Name nm)(if (= nm "module") 1 (if (= nm "")
2 3)))(_ -9))'`.

### What's NOT blocked

The scalar arithmetic BACKEND is solid: `harness/cdzc.py oracle` → 15/15 (checked +/-/* value+overflow-trap
from hand-built Mir → select/serialize/frame → run). Only the decode FRONT-END on real bytes is blocked by
this recursive-walk miscompile.

**Priority.** 🔴 miscompile (wrong value across the run boundary) — the last blocker for end-to-end
real-bytes `compile-bytes`. Same family as ask-73/ask-77/ask-14 (coarse-kind inference re-derived at emit,
recursive heap-carrying return); durable fix = real HM (ask-75). May share machinery with the active c83
runtime-match-typing work. Related: ask-77 (closed — the KIND/decline face of decode; this is its VALUE
face), ask-79 (built-in-op accepts Option — a different decode hazard, also open).

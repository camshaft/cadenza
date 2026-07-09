## 73. 🔴 (seed) A tuple-returning recursive function that threads an accumulator declines — "runtime sum match without a constructor arm"

**Status: BLOCKING the compiler rewrite's recursive-descent PARSER.** With ask-69 retired (cdzc hand-rolls
its own byte parser instead of `Ast.decode`, per operator direction), the parser threads a `(node, cursor)`
tuple through mutually-recursive `read-node` / `read-array` calls — the canonical recursive-descent shape.
But the seed declines a **tuple-returning recursive function that matches a helper's tuple and recurses with
an updated accumulator**, with **"runtime sum match without a constructor arm"** (native too). The tuple
match is misclassified as a sum match when the tuple comes from a call and the arm recurses.

**Minimal reproducer (stable seed, `emit`; oracle 6):**

```
; DECLINES "runtime sum match without a constructor arm":
(module m
  (def (go n acc)
    (if (= n 0)
        (tuple acc 0)
        (match (pair n) ((tuple v k) (go (- n 1) (+ acc v))))))
  (def (pair n) (tuple n n))
  (def (main) (match (go 3 0) ((tuple a b) a))))
;   go 3 0 → sums 3+2+1 into acc → (tuple 6 0); a = 6
```

**⚡EVEN-MINIMAL repro (2026-07-08, independently reconfirmed):** no accumulator or helper needed — a bare
TAIL-recursive function that returns a tuple declines:
```
; DECLINES "runtime sum match without a constructor arm" (oracle 0):
(module m
  (def (go n) (if (< n 1) (tuple 0 0) (go (- n 1))))
  (def (main) (match (go 3) ((tuple a b) (+ a b)))))
```
Verified: the SCALAR tuple `(tuple 0 0)` declines identically to a heap `(tuple (Ast.Int i) i)` — so it is
NOT about the tuple's contents, purely the **tail-recursive tuple-valued return**. A NON-tail recursive
function that WRAPS its recursive result (`(match (go (- n 1)) ((tuple a b) (tuple (+ a 1) b)))`) COMPILES,
and a non-recursive tuple return COMPILES — so the trigger is specifically *tail recursion + tuple return*
(the recursive call site is inferred as "unknown tuple shape", so a `match`/`tuple.N` on the result fails).

**The precise boundary (verified — these COMPILE, isolating the gap):**
- ✅ `go` recurses DIRECTLY on itself as the match scrutinee (`match (go (- n 1)) ((tuple a b) …)`), tuple
  both branches → compiles.
- ✅ the same shape with a LITERAL tuple scrutinee inline (`match (tuple n n) …`) → compiles.
- ✅ a tuple-match whose arm calls a non-recursive (or even mutually-recursive) helper, when the function's
  branches are SCALAR → compiles.
- ❌ the function returns a TUPLE in every branch AND the recursive branch matches a **helper call's** tuple
  (`(pair n)`) and **recurses** with an accumulator → declines. A `let`+`tuple.0` variant (no match)
  declines identically, so it is NOT the match — it is the tuple-valued recursive-branch return-kind.

So the gap is **return-kind inference for a tuple-valued recursive function whose recursive branch's tuple
comes through a helper call + accumulator update** — the branch isn't recognized as tuple-valued, so the
downstream tuple destructure is misread as a sum match with no constructor arm.

**Why it BLOCKS.** A recursive-descent parser is exactly this shape: `read-array` returns `(Ast.List acc,
pos)` at the base, and in the recursive branch does `(match (read-node b i) ((tuple node nx) (read-array b nx
(- n 1) (List.push acc node))))` — a tuple-returning function matching a helper's `(node, cursor)` tuple and
recursing with an updated accumulator. Every non-trivial cursor-threading walk hits this. The `(Ast, pos)`
form (heap node + int cursor) declines identically to the scalar `(int, int)` form.

**What the seed needs.** Recognize a recursive function's recursive branch as tuple-valued when its base
branch is a tuple and the recursive branch yields a tuple (through a call and/or accumulator), so the tuple
destructure of its result lowers as a tuple match, not a sum match. This is the tuple-return-kind companion
of the already-realized scalar/heap tail-recursive accumulator inference.

Corpus repro added: `spec/semantics/02-binding-and-control.sexp` "a recursive function that threads a tuple
accumulator returns it" (→ 6), currently `todo [runtime sum match without a constructor arm]`.

**Priority.** 🔴 HIGH — it is the keystone of the rewrite's front end (the byte→Hir parser). Verified the
parser's building blocks otherwise work (Bytes.at + arithmetic, runtime Ast/tuple construction, String from a
byte slice, the `(Ast, pos)` cursor tuple), so this return-kind case is the specific blocker. Related: the
tail-recursive heap/scalar accumulator return-kind inference (realized); ask-65 (payload-through-return by
resolve).

**⚡NARROWING (2026-07-08) — the gap is TUPLE-SPECIFIC; the RECORD path already works, so it is the reference
to mirror.** A/B verified with the identical tail-recursive 2-field-pair shape:
```
; DECLINES "runtime sum match without a constructor arm":
(def (go n) (if (< n 1) (tuple 5 6) (go (- n 1))))          ; + (match (go 3) ((tuple a b) (+ a b)))
; COMPILES — same shape as a RECORD:
(def (go n) (if (< n 1) (record (a 5) (b 6)) (go (- n 1))))  ; + (. (go 3) a)
```
And the full accumulator/parser shape returning a `(record (v …) (p …))` (matching a helper's tuple, tail-
recursing) COMPILES, whereas the `tuple` version declines. So tail-recursive **record** return-kind inference
is realized and correct; **tuple** return-kind inference is not — the fix is to make the tuple case infer the
same way the record case already does (a tail-recursive function whose base branch is a tuple must carry that
tuple result-kind back through the recursive call, exactly as it does for a record). NOTE: restructuring the
decoder to thread a record instead of a tuple would be a WORKAROUND (the natural cursor is a `(node, pos)`
tuple) — NOT taken; documented here so the fix is targeted (mirror the record return-kind path for tuples).

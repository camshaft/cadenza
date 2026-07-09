## 71. 🔴 (seed) A top-level VALUE definition `(def name value)` is rejected — "def without a signature"

**Status: BLOCKING the merged `cdzc.cdz` build.** The seed accepts only FUNCTION definitions `(def (f …) …)`
at a module's top level; a VALUE definition `(def name value)` declines **"def without a signature"** (native
too). But a definition is "a value, function, type" (glossary), and each registers its name in the module
(core-semantics.md #A Module Evaluates To A Record Of Its Exports), so a top-level value-def MUST bind its
name for the module's functions to use.

**Minimal reproducers (stable seed, `emit`; native declines both):**

```
; scalar value-def — declines "def without a signature"; MUST bind answer=42 → 42
(module m
  (def answer 42)
  (def (main) answer))

; record value-def + projection — the op.cdz shape; declines; MUST bind tbl and project → 8
(module m
  (def tbl (record (a 7) (b 8)))
  (def (main) (. tbl b)))
```

Corpus repros added (both currently `todo [def without a signature]`):
`spec/semantics/11-modules.sexp` — "a top-level value definition binds a name usable by the program's
functions" (→ 42) and "…binds a record projected by the program's functions" (→ 8).

**Note the existing coverage this extends.** `11-modules.sexp` already has "a module value definition
registers a reachable export field" for a value-def inside a NESTED module in do-position (`(do (module m
(def v 7)) (. m v))`). This ask is the OUTER-program-module case — the top-level def of the program the
compiler is emitting — which is a distinct code path and is the one that declines.

**Why it's load-bearing NOW.** The rewritten compiler `cdzc.cdz` is built (implementation/compiler/Makefile)
by merging `cdzc/*.cdz` submodule files, one of which is the `@generated` opcode table `05-op.cdz` =
`(def op (record (i64-const 0x42) …))` — a top-level record value-def. With it in the merge, `cdzc.cdz`
declines "def without a signature". This is NOT worked around (the op record is a genuine shared table, and
wrapping it as a nullary function `(def (op) …)` would be a contortion) — the merged compiler fails honestly
on this gap until the seed accepts a top-level value-def.

**What the seed needs.** Register a top-level `(def name value)` as a module field bound to `value` (the same
"each definition registers its name and value as a field" rule already realized for NESTED module value-defs
and for function-defs), so `name` resolves in sibling functions and (for a record value) `(. name field)`
projects. A value-def's `value` is an ordinary expression evaluated at bind time (const-folded where it is a
literal/record of literals, as `op`'s table is).

**Priority.** 🔴 HIGH — it blocks the merged `cdzc.cdz` from compiling at all once its generated opcode
table joins the build, and top-level value-defs are how a self-hosted compiler carries its shared data
tables. Related: the nested-module value-def case (already in the corpus), ask-58 (modules-as-records —
this is the same "a definition registers a field" rule at the outer top level).

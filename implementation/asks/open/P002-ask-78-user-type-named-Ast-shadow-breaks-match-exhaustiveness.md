## 78. 🟢 (seed) A user `(type Ast …)` that shadows a built-in type name breaks match-exhaustiveness: construction resolves to the user type, but the exhaustiveness checker consults a DIFFERENT variant set — MINIMAL, STANDALONE repro

**Not a cdzc blocker** (cdzc uses the built-in `Ast` directly, so it's unaffected) — but a clean,
minimally-isolated correctness gap in name resolution / exhaustiveness, found while diagnosing ask-77.
Filed because it reproduces standalone (unlike ask-77), so it's cheap to fix and pin.

**Symptom.** A user sum type whose name collides with a built-in type name (`Ast`) can be CONSTRUCTED, but
an EXHAUSTIVE `match` over its variants is wrongly rejected **"match does not cover every variant of the
sum"** — and NO arm set satisfies it (neither the user's own variant names nor the built-in's).

**Minimal repro (declines):**
```
(module m
  (type Ast (| AInt Int64 | ALst Int64))
  (def (main) (match (Ast.AInt 5) ((Ast.AInt n) n) ((Ast.ALst m) m))))
```
→ `decline: match does not cover every variant of the sum`.

**Control — identical but the type is named `T` (compiles):**
```
(module m
  (type T (| AInt Int64 | ALst Int64))
  (def (main) (match (T.AInt 5) ((T.AInt n) n) ((T.ALst m) m))))
```
→ OK.

**The trigger is the NAME, not the payload/recursion.** Confirmed by bisection:
- payload shape is irrelevant — all-scalar (`| ALst Int64`), `(list Int64)`, `(list Ast)`, direct self-ref,
  `(Tuple Ast Ast)` ALL decline when the type is named `Ast`;
- the SAME declarations named `T`/`Hir` (any non-built-in name) all COMPILE;
- covering the BUILT-IN `Ast`'s 6 variant names (`Int/Str/Name/List/Bool/Float`) ALSO declines — so the
  checker isn't simply using the built-in set either; construction resolves `Ast.AInt` to the USER type
  (it succeeds) while exhaustiveness consults a set that matches NEITHER declaration cleanly.

**Likely root.** A built-in type name isn't shadowed consistently by a user `(type …)` declaration: the
constructor-resolution path picks the user type (nearest binding), but the exhaustiveness/variant-set
lookup for `match` keys off the built-in registration (or a merged/ambiguous set). Per the ratified model
(lexical nearest-binding — `09-functions.sexp`; "a sum type is a record of its constructors,
resolved by the ONE `(. record key)` rule" — `05-compound-types.sexp`; the capitalization-not-semantic
learning), a user declaration should shadow the built-in **uniformly** for construction AND matching, or
the collision should be REJECTED at declaration (CDZ) — but not silently split so that construction sees
one type and `match` sees another.

**Priority.** 🟢 low — doesn't block cdzc, and colliding with a built-in type name is unusual. But it's a
soundness smell (construction and matching disagreeing on what a name denotes) with a one-line repro, worth
either (a) making the user declaration shadow consistently, or (b) rejecting a user `(type …)` that shadows
a built-in type name at declaration. Related: the records-everywhere / one-resolution-rule model; the
capitalized-name-resolution family in memory (resolution must be lexical-scope-first and consistent across
every use site).

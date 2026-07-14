# The Cadenza compiler, written in Cadenza (ML surface)

A from-scratch port of the compiler into Cadenza itself, written in the **ML surface**, in *ideal
form* — the compiler you would write if the language were finished. The Rust reference compiler
(`implementation/seed/crates/rcdzc`) is the structural **guide**; this is not a transliteration but a
re-derivation in idiomatic Cadenza.

This is a deliberate **stress test of the language**. Where Cadenza cannot express something cleanly,
the rule is to **report the issue so it gets fixed** — either a fix landed in the seed `rcdzc`, or a
crisp repro filed — rather than contorting the code around a limitation. Friction found is a
deliverable.

## Toolchain

- Author `.cdz` files (ML surface). When unsure of syntax, generate the canonical form with
  `cdz convert <file>.sexp --from sexpr --to ml` — do not hand-transcribe nested `match`/patterns.
- **`cdz check file.cdz`** is the primary loop: every well-formedness fault as
  `file:line:col: severity [CODE]: message`, exit ≠ 0 on error. `--json` for structured output.
- To exercise the backend: `cdz convert file.cdz --to binary > file.bin && cdz compile file.bin -t wasm
  -o out.wasm` (compile is the full type-check + lowering).

## Project manifest + tests

`Project.cdz` is the project manifest, **written in Cadenza itself** — well-known top-level `def`s the
`cdz` binary reads (a def is the manifest; no new syntax, no per-command flags). A file-list entry may
be a literal name OR a **glob** (`*.cdz`, `src/*.cdz`, `**/x.cdz`):

```
def name    = "compiler-ml"
def modules = ["src/*.cdz"]      // library modules — a wildcard, so a new pass just drops into src/
def tests   = ["src/*.cdz"]      // modules whose @test defs form the suite
def exclude = []                 // files removed from the globs above (a demo/fixture to skip)
```

Tests use the built-in **`@test`** workflow (`TESTING.md`): mark a nullary def `@test`; it PASSES by
returning `unit`, FAILS by trapping (`trap("…")`, or the `assert`/`assert_eq`/`assert_ne` helpers,
carries the message). Run them:

```
cdz test                   # NO arg: search up from the cwd for the nearest Project.cdz (like cargo), run it
cdz test .                 # reads Project.cdz here, runs every @test in the declared `tests` modules
cdz test Project.cdz       # same, naming the manifest directly
cdz test src/ast.cdz       # one file's @test defs
cdz test --filter head     # only tests whose name contains "head"
```

`cdz test` with no argument walks UP from the current directory to the nearest `Project.cdz`. A
`tests`/`modules` glob expands against the manifest dir (path-sorted, deduped, `Project.cdz` never
matched); a matched file that also matches an `exclude` pattern is dropped. `cdz test <dir>` with no
manifest walks every source file under the dir. A `@test` never burdens a normal `cdz compile` (the
test defs are unexported → dead → dropped). Tests live SAME-FILE with the code they test (a cross-file
test cannot yet construct a type whose variant shadows a prelude name — see
`repros/import-prelude-collision`), so each module tests itself.

## Structure (mirrors the rcdzc stages)

Source modules live under `src/`; `Project.cdz`, `README.md`, `TESTING.md`, and `repros/` sit at the
top. Current `src/` modules (each with same-file `@test`s — 73 tests total across 9 modules):

- `src/ast.cdz` — the AST datatype + pure traversals (`node-count`, `head-name`; the `ast.rs`
  analogue). One recursive sum; a node contains its children (no arena — the language has real
  recursive values). The leaf value variants are `Int`/`Str`/`Bool` alongside the `Name` identifier and
  the `List` form — the subset the pipeline observes so far; the richer wire leaves (Float, Char, Bytes,
  Sym, markers) join as passes read them.
- `src/print.cdz` — renders an `Ast` to an s-expr string (the inverse of decode; hand-written itoa).
- `src/ast-eq.cdz` — structural `Ast` equality (for dedup / constant-fold / quote comparison).
- `src/free-vars.cdz` — collect an `Ast`'s `Name`s into a `Set String` (a resolve-lite pass).
- `src/fold.cdz` — a constant-folder: reduces `(op Int Int)` forms (`+`/`-`/`*`) to an `Int`, bottom-up
  (so `(+ (* 2 3) 4)` → `7`); leaves unknown-op / non-constant forms untouched.
- `src/subst.cdz` — substitution (β-reduction / macro-expansion core): replaces each `Name` found in a
  `Map String Ast` environment with its `Ast`, recursively. Stresses a compound-VALUED `Map`.
- `src/check.cdz` — an arity checker: counts call-forms `(Name op arg…)` whose arg count differs from
  `op`'s declared arity in a `Map String Int64` arity table (recursing into every child).
- `src/eval.cdz` — an evaluator: reduces an arithmetic `Ast` (`+`/`-`/`*` over `Int` leaves) to a
  `BigInt` result, recursively (so `1e9 * 1e9` doesn't overflow — it uses arbitrary precision).
- `src/unify.cdz` — the core of Hindley-Milner inference: unification with a substitution over a
  recursive `Ty` (`Var Int64` / `Con String` / `Arrow Ty Ty`). `unify` returns
  `Result (Map Int64 Ty) String` — the Ok carries the UPDATED substitution (a compound `Map` Ok payload
  threaded through the recursion). Exercises occurs-check recursion, transitive var-chain `resolve`,
  arrow decomposition, and an Int64-keyed compound-valued `Map`.

Planned, following the rcdzc pipeline: decode (binary AST → `Ast`) · resolve · infer (Hindley-Milner)
· lower (→ core) · encode/emit. The compiler is fundamentally bytes → bytes.

### Decode — the current front (blocked on two seed issues, see the log)

`decode` reads the canonical binary AST (`implementation/seed/crates/rcdzc/src/codec.rs`: an 8-byte
version header, a leaf table, a struct table, a root id — all LEB128) into the recursive `Ast`. The
idiomatic-Cadenza job is to resolve the *flat wire arena* into a *recursive tree* (following struct
ids), which is the whole point of the "a node contains its children" design. The decode LOGIC is
proven (LEB128 varint reader, big-endian magnitude, per-leaf reader, struct reader, and the arena→tree
`build`/`build-list` all verified in isolation against real `cdz convert --to binary` bytes), but two
seed issues block landing it as a `.cdz` that compiles + runs end to end (both surfaced by decode; see
the log). Until they're resolved, decode reconstructs the full tree STRUCTURE + every Int/Bool leaf
exactly (so `node-count` and shape passes are correct); Name/Str leaf CONTENT is a placeholder.

## Language issues found (stress-test log)

- **FIXED** (seed `rcdzc` db.rs `scan_type_decl`): a `///` doc comment on a `type` declaration was
  mis-parsed — the ML reader attaches the doc as a `(doc …)` form after the type name, and the sum scan
  read it as a bogus `doc` variant (CDZ0201 "declared more than once"). Now the scan skips a leading
  `(doc …)`, mirroring how a `def`'s leading doc is stripped.
- **Note (not a bug):** author nested `match` via `sexpr → ml`; the reader resolves nesting by greedy
  last-arm absorption, so a hand-written inner `match` easily mis-attaches its catch-all to the outer
  match (CDZ0210 non-exhaustive + CDZ0213 unreachable). The printer's own output round-trips correctly.

- **OPEN (seed `rcdzc` — a plain `//` LINE COMMENT HIDES the following top-level form).**
  `repros/reject-line-comment-hides-toplevel-form.cdz`. The reader wraps a leading line comment as
  `(comment "…" <next-form>)` and the compiler's top-level SCAN does not see through the wrapper — so
  the wrapped `def`/`type`/`export`/`@test`/`effect` becomes invisible (`cdz check` → "unbound name
  `comment`" per form + the def's own name unbound). **SHARP BOUNDARY (probed 2026-07-14 — broader than
  first logged, which wrongly said only `@test`/`effect` were hit):** a plain `//` leading ANY top-level
  form breaks; `//` BETWEEN two defs, TRAILING-INLINE on a def, or with a blank line after it all still
  break (the wrapper attaches to the next top-level form regardless). What WORKS: a `///` DOC comment on
  a `def`/`type` (that scan strips the leading `(doc …)`), and a `//` INSIDE a body (not top-level). So
  the gap is precise: the top-level scan strips a leading `(doc …)` but NOT a leading `(comment …)` —
  the fix mirrors the doc-strip (peel a leading `(comment …)` in the same db.rs scan). Impact: every
  top-level comment in a port module must be `///` or live inside a body. The manifest reader peels
  `(comment …)` as a workaround (so `cdz test` reads a commented `Project.cdz`), but `cdz check` on any
  commented module still errors — a real top-level-scan gap.

- **✅ FIXED (seed `rcdzc`@`b2bf850d`): a `br_table`-lowered match (≥4 arms) in OPERAND position dropped
  a RECURSIVE-CALL sibling operand.** Reported by this loop, fixed by a sibling within one iteration
  ("a br_table match arm branches to the match's own join block, not one block past it"). `go(4)` now
  returns `"bb"` (was `"b"`); a 10-arm-match `itoa` renders `"12345"` correctly. `src/print.cdz` reverted
  its if-chain `digit` workaround to a clean 10-arm `match`. Regression witness kept in
  `repros/miscompile-brtable-match-operand-drops-sibling.sexp`. ORIGINAL: in `(String.concat (go …)
  (d …))` with `d` a ≥4-arm match, the match's `br_table` arms each `br` to the function-result label,
  escaping past the `bytes-concat` and discarding the recursive operand (≥4 arms = the br_table
  threshold; verified on integer `+`; only bit a recursive sibling).

- **OPEN (seed `rcdzc` — MISSING op): `List.map` does not exist.** A `List` value has only
  `at`/`len`/`push`/`concat`/`update`/`slice` (`prelude.rs` `list_module`); `(List.map xs f)` →
  CDZ0201 "record has no field `map`". The corpus MENTIONS `(List.map xs f)` but only in a `|>` doc
  comment (09-functions ~2827) — it is never a realized case. A compiler port maps over lists
  constantly (transform every AST child, every arg); the workaround is a hand-written recursive map
  (`(match xs ((list) (list)) ((list h .. t) (List.concat (list (f h)) (rec t))))`), which works but is
  O(n²) via `concat`. `List.map`/`List.filter`/`List.fold` are the obvious missing higher-order list ops.

- **OPEN (seed `rcdzc` — MISCOMPILE, silent trap): a PARAMETERIZED compound-returning export traps.**
  An export that takes a parameter AND returns a compound (tuple / record / sum) compiles clean (`cdz
  check` passes; the component WIT even shows `make: func(p0: s64) -> t`) but TRAPS at run time —
  `cdz-run … --arg 5` → "trap: expected 1 argument(s), got 0" (wasmtime's arity check). The export's
  argument is not delivered to the resource-escape `make`. A NULLARY compound-return export works
  perfectly. Repro `repros/miscompile-parameterized-compound-export-traps.sexp`. ⚠ the seed test
  `a_parameterized_compound_return_export_compiles_via_the_resource_escape` only asserts it COMPILES —
  it never RUNS the component with an arg, so this runtime gap is untested (a false-confidence test).

- **OPEN (seed `rcdzc` — runtime `String.from-bytes` declines):** `String.from-bytes` (and the
  `Ast.decode` self-decode) only compute on a *compile-time-constant* `Bytes`; a runtime byte slice
  DECLINES ("String.from-bytes of a runtime byte sequence is not yet computed (constant Bytes only)",
  `lower.rs::lower_str_from_bytes` ~13051). A decoder reads a `Name`/`Str` leaf's bytes from a runtime
  buffer, so it cannot MATERIALIZE the string content — every real AST is full of `Name` leaves, so
  this blocks a faithful decode. The fix looks tractable and small: a runtime `String` IS the SAME flat
  UTF-8 byte-leaf as a runtime `Bytes` (`lower.rs` ~1664, `String.concat` on runtime strings lowers to
  `bytes-concat` over their byte leaves), so `String.from-bytes` on a runtime `Bytes` is nearly the
  IDENTITY on the byte handle, plus UTF-8 validation for the `Option`. Worth a dedicated increment.

- **OPEN (seed `rcdzc` — backend MISCOMPILE, silent):** a SELF-TAIL-RECURSIVE function that passes a
  TUPLE-PROJECTED SUM-HANDLE (`(. r 0)` where `r : (Tuple W …)` and `W` is a boxed sum) as a loop
  ITERATION ARGUMENT miscompiles — the value is silently wrong (a `match` on it reads 0). `cdz check`
  is CLEAN → a lowering/codegen bug, not a type error. Root-caused to the SELF-TAIL-CALL LOOP TRANSFORM
  (`backend/wasm/select.rs::emit_loop_iteration`, the loop back-edge that evaluates the new arg values
  and stores them into the param slots). Minimal repro (`--arg 0` returns 0, must be 5), in
  `repros/miscompile-tail-loop-projected-sum-arg.sexp`:
  ```
  (do
    (type W (Atom Int64) (Node (List Int64)))
    (def (one (: b Bytes) (: pos Int64))
      (if (= (Option.expect (Bytes.at b pos) "t") 0)
        (tuple ((. W Atom) (Option.expect (Bytes.at b (+ pos 1)) "v")) (+ pos 2))
        (tuple ((. W Atom) 99) (+ pos 2))))
    (def (loop (: b Bytes) (: pos Int64) (: n Int64) (: last W))     ; self-tail-recursive → LOOP
      (if (= n 0) last (let ((r (one b pos))) (loop b (. r 1) (- n 1) (. r 0)))))  ; (. r 0) : W arg
    (def (wval (: s W)) (match s (((. W Atom) li) li) (((. W Node) ids) 0)))
    (def (main (: pos Int64)) (wval (loop b"\x00\x05\x00\x07" pos 1 ((. W Atom) 0))))
    (export main))
  ```
  Two CONTROLS both return 5 (in `repros/`): `miscompile-CONTROL-nontail-recursion-ok.sexp` — make the
  self-call NON-tail (`(+ 0 (loop …))`) so it lowers to an ordinary `Core::Call` instead of the loop
  transform; and `miscompile-CONTROL-direct-sum-arg-ok.sexp` — pass the sum handle DIRECTLY (`(one b
  pos)`) rather than projected out of a tuple. So the trigger is precisely the loop back-edge storing a
  tuple-projected sum handle into a param slot. (`W` needs a compound-payload variant so it is a boxed
  i32 handle; a single-variant `W` is newtype-erased to its inner scalar and compiles fine — the earlier
  bisection that fingered "compound variant + if + tuple" was seeing this same loop-transform path.) A
  SECOND surface of the same defect: two sibling `if`-branches each placing an `if` in one Ast-typed
  tuple slot emits INVALID wasm ("expected i64, found i32") — `repros/miscompile-two-sibling-ifs-
  invalid-wasm.sexp`. This is what blocks `decode` today: `read-leaf`/`read-struct` return
  `(tuple <sum> pos)` and the decode loops thread the projected sum through a self-tail recursion.
  **SHARPER BOUND (2026-07-14):** the essential ingredient is an `if` INSIDE the function that builds the
  `(tuple <boxed-sum> pos)`; the projected sum is then mis-typed by the loop-transform. Tail-recursion →
  silent wrong value; NON-tail recursion → invalid wasm (even when the recursive branch never runs — the
  base-case compose alone fails to validate, so it's the loop-transform ANALYSIS mis-slotting, not the
  path executing). Repro `repros/miscompile-if-tuple-sum-nontail-recursion.sexp`. 🔑 **A BARE RECURSIVE
  SUM (NOT wrapped in a tuple) works perfectly** — a runtime-built recursive `Tree`/`Ast` folds and
  escapes correctly (verified: `mk`/`sumt` over a depth param, and a `(List Ast)` count). So the decode
  design should thread POSITION separately (not `(tuple ast pos)`) — e.g. return the sum bare and track
  the cursor another way — to sidestep this entirely until the loop-transform fix lands.
  🔬 **ROOT-CAUSED (2026-07-14): it is an i32/i64 SLOT-ALIASING bug in the loop-transform emit**
  (`backend/wasm/select.rs`). Minimal reproducer `repros/miscompile-slot-alias-i32i64-loop-tupleproj.sexp`
  (`cdz check` clean → invalid wasm "type mismatch: expected i32, found i64"). In the emitted
  `read-leaves` loop, ONE wasm local (slot 4 in the WAT) is `local.set` at **i64** for the `pos+1`
  arithmetic temp AND used as **i32** for the handle returned by the recursive tuple-returning
  `read-varu` — the loop-transform's scratch allocator reuses a slot across the two widths. The
  jointly-required ingredients (each removed individually → compiles + runs): (1) the loop advances its
  position via a helper that PROJECTS BOTH fields of a recursive tuple-returning call
  (`(+ (. v 1) (. v 0))`), (2) the loop pushes a COMPOUND-payload sum into a `(List …)` accumulator,
  (3) it is a self-tail loop. THRESHOLD-DEPENDENT on total locals — smaller variants of the same shape
  stay under the aliasing threshold and pass, which is why it resists further minimization. This is the
  same defect as the tail-loop-wrong-value and sibling-ifs-invalid-wasm surfaces above — one slot-typing
  root, several faces. It DIRECTLY blocks a `decode` that advances its cursor with a varint-width helper.

**Confirmed WORKING (stress-swept 2026-07-14):** recursive sum types (build + fold, const + runtime),
HOFs (fn args, closures capturing env, curried/partial application, recursive HOF), Map insert/lookup,
Set of/contains, generic `id` at multiple types, nested generic newtypes, `Result`/`Option` match
(incl. nested + Option-of-tuple), match guards (`(guard pat cond)`), let shadowing, `Record.with`/
`extend`/`project`, assoc-list env lookup, BigInt arithmetic, deep tail recursion (5000), mutual
recursion, bit ops, string equality/ordering, div/mod, big match dispatch, `String.to-bytes`, nullary
compound-return escape (tuple/record/recursive-sum/list). The compiler is broadly solid; the gaps above
are the sharp edges.

- **OPEN (seed `rcdzc` — GAP + misleading diagnostic): a polymorphic type annotation with an unbound
  signature type variable is rejected.** `repros/reject-polymorphic-type-annotation.sexp`: `(def (len
  (: l (Lst a))) …)` → CDZ0203 "`Lst` is a type, not a function" + CDZ0101 "unbound name `a`". No form
  binds a signature's type variables, so `(Lst a)` can't be written as a param type. The UNANNOTATED
  `(def (len l) …)` works and monomorphizes at every element type (the idiomatic spelling); a CONCRETE
  `(: l (Lst Int64))` works too — only a type-VARIABLE annotation fails. Two asks: (1) bind a
  signature's type vars so `(: l (Lst a))` resolves; (2) the diagnostic mis-reads `(Lst a)` as a call —
  it should say "a polymorphic annotation needs `a` bound." Impact: generic passes drop the annotation.
- More confirmed WORKING (loop iter 1): `quote` building a built-in `Ast` (metaprogramming); effects +
  handlers (a state counter via `handle E init with | op(a,s) => resume(v,s') in …`); recursive-generic
  `Lst` length + `Lst of Lst` (unannotated) monomorphized at Int AND String; `Map String → user-sum`
  lookup+match; `Set` union/difference/contains/len (String-keyed). ⚠ minor: `Set.len` vs `Map.size`
  (inconsistent op name for the same "count" concept).

- **OPEN (seed `rcdzc` — LIMITATION): a LIST pattern arm allows at most ONE refutable (constructor)
  element.** `repros/reject-list-arm-multiple-ctor-elements.sexp`: `((list (A.I x) (A.N y) c) …)` →
  "a list arm with more than one refutable constructor element is not yet supported (match one tagged
  element per arm)". ONE ctor element + bare binders is fine; TWO+ rejects. This blocks the natural
  fixed-shape destructure a compiler pass wants (`[Name op, Int x, Int y]`). WORKAROUND (used by
  `src/fold.cdz`): bind every element as a plain binder, then nested-`match` each. Ask: N refutable
  elements per list arm. Clean REJECT (not a miscompile) → a Todo, but a common shape.

- **OPEN (seed `rcdzc` — DECLINE + a context-dependent WRONG-VALUE sibling): `String ==` on a value
  returned from `Map.lookup`.** `repros/decline-borrow-ownership-returned-map-string-eq.sexp`. When a
  `String` originates from a `Map.lookup`, is RETURNED from a function, then used as a `String ==`
  operand in the caller → "borrowing op operand has an ownership this backend cannot yet prove" (`cdz
  check` clean, `cdz compile`/`cdz test` decline). Doing the `==` INSIDE the lookup's match arm is fine.
  A related WRONG-VALUE variant (a `String ==` on a value from a MISSED lookup's `None → node` return)
  miscompiles only past a per-module def/test-count THRESHOLD (passes standalone). Surfaced building
  `src/subst.cdz`; both sidestepped by checking a result's SHAPE (`match … ((Ast.Name _) …)`) instead
  of `String ==`-ing an extracted payload.

- **OPEN (seed `rcdzc` — GAP/BUG): quasiquote `unquote` of an already-`Ast` value is rejected.**
  `repros/reject-unquote-of-an-ast-value.sexp`. `(quasiquote (+ (unquote sub) 1))` with `sub : Ast` →
  CDZ0201 "a variant constructor's payload has declared type Int64, but a value of type Ast was
  applied". `(unquote n)` where `n` is a plain `Int64`/literal WORKS (wrapped as `Ast.Int`); only an
  already-`Ast` value fails — `unquote` wraps by the template slot's leaf type instead of splicing an
  Ast node as-is. metaprogramming.md says `,<expr>` inserts its RESULT at that position; when the result
  IS an Ast, that should splice the node. Blocks the canonical AST-building macro (embed a computed
  subtree). Confirmed WORKING otherwise: quote structural `=`, walking a quoted form via own `Ast`
  match, quoted Ast escaping to host, `unquote-splice` of a list, `(unquote <plain-value>)`.

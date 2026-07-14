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
test defs are unexported → dead → dropped).

**`cdz test` FOLLOWS the import closure** (mirrors `cdz check`): a module whose `@test` imports a
sibling type/function links against it and runs — so a test can reuse another module's `Ty` etc. A
directory run runs each file's OWN tests (the entry-file filter keeps a shared imported library's tests
to that library's own run, never double-counted through an importer). Tests still live SAME-FILE with
the code they test (a cross-file test cannot yet construct a type whose variant shadows a *prelude* name
— see `repros/import-prelude-collision`), so each module tests itself, but a test may now freely IMPORT
non-colliding names from a sibling.

## Structure (mirrors the rcdzc stages)

Source modules live under `src/`; `Project.cdz`, `README.md`, `TESTING.md`, and `repros/` sit at the
top. Current `src/` modules (each with same-file `@test`s — 297 tests total across 32 modules):

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

- `src/decode.cdz` — reconstructs a recursive `Ast` from a flat byte buffer at RUN TIME (the stage
  blocked since iteration 1). Now fully viable: BOTH faces of the slot-alias/projected-sum family are
  fixed, so a `(tuple ast pos)` may be threaded through a self-tail loop directly. This decoder still
  threads its cursor SEPARATELY (a node's byte size is recomputed by `nsize`, so `buildc` advances via
  `pos + nsize(…)`) — a valid alternative, no longer a work-around. Reconstructs the full tree STRUCTURE
  + every Int/Bool leaf value EXACTLY (verified: nested lists, deep nesting, Int-value round-trip);
  Name/Str leaf CONTENT is still a placeholder `""` (runtime `String.from-bytes` decline). 10 `@test`s.
- `src/traverse.cdz` — higher-order combinators the prelude lacks: `map-list`/`filter-list`/`fold-list`
  over `Int64` lists, plus Ast traversals `count-where` (count nodes matching a predicate) and
  `int-total` (fold every Int leaf). Exercises closures passed to self-recursive HOFs (each fn param
  arrow-annotated to sidestep the inference gap above). 9 `@test`s (map/filter/fold, a pipeline,
  predicate counts, nested Int total).
- `src/depth.cdz` — pure structural METRICS over an `Ast`: `depth` (max nesting), `leaf-count`,
  `internal-count`. All SCALAR-returning (no heap value crosses a call), so it sidesteps the resolver's
  borrow-ownership block — a deliberately scalar-only pass that RUNS. Useful for cost heuristics /
  recursion-depth guards. 10 `@test`s (incl. a leaf+internal = total consistency check).
- `src/prec.cdz` — operator precedence + the pretty-printer's parenthesization decision: a `Map String
  Int64` precedence table, `needs-parens` (a child of lower precedence needs wrapping), `paren-count`
  (fold that over a tree). Stresses a `Map String Int64` keyed by a matched sum-payload String. 10
  `@test`s pass; the DEEP `paren-count` case is withheld (it hits the two-live-string-payloads
  miscompile — see the log).
- `src/fresh.cdz` — a fresh-id generator on an EFFECT + handler (the port's first use of the effect
  system): `Fresh.next` yields a distinct `Int64` per call, a state-threading handler
  (`resume(s, s + 1)`) supplies 0, 1, 2, …. The HM/compiler "gensym". `id-sum` is a SINGLE self-recursive
  effectful loop (that shape specializes + runs; a mutually-recursive effectful group with the perform
  in a separate branch does NOT yet — see the log). 8 `@test`s (closed-form sums, a custom start, a
  draw-count via a constant-handback handler).
- `src/collect.cdz` — a leaf-collecting analysis returning a RECORD `{ count, vals }` (the number of
  `Num` leaves and their values) built bottom-up in one traversal. Stresses a record carrying a heap
  `List` field, CONSTRUCTED per node and THREADED (concatenated) through the recursion, then projected —
  the record-of-results shape a real analysis returns. Confirmed WORKING (no new bug): record values +
  the heap-list field survive the recursion exactly (a `count == vals-length` invariant test). 9
  `@test`s.
- `src/scope-fv.cdz` — the BINDING-AWARE free-variable analysis of the untyped lambda calculus (the
  scope-respecting complement to `free-vars.cdz`'s flat collector): a `Var` is a singleton `Set`, an
  `App` UNIONS, and a `Lam x` REMOVES its bound var. Stresses `Set String` union + remove threaded
  through recursion. 10 `@test`s incl. shadowing (`λx.λx.x` closed) and a name free-and-bound at once
  (`(x (λx.x))` — outer x free). Confirmed WORKING (no new bug). ⚠ minor API inconsistency: a `Set`'s
  count is `Set.len` but a `Map`'s is `Map.size` (same "count" concept, two names).
- `src/compare.cdz` — a TOTAL ORDER over `Ast`: a three-way `cmp` returning -1/0/1, ordering by node tag
  then by payload (Nums by value, Nodes lexicographically by children — a proper prefix is smaller). The
  canonical ordering a compiler uses to sort / dedup / hash-cons terms. Fully recursive; returns scalars
  (dodges the runtime-String pitfall). 12 `@test`s incl. an antisymmetry invariant (`cmp(a,b) ==
  -cmp(b,a)`) and deep-leaf differences. Confirmed WORKING (no new bug).
- `src/validate.cdz` — a validation pass that PARTITIONS a node list into successes + diagnostics in one
  traversal (not fail-fast), returning both as lists inside a `Report.Mk(List Int64, List String)` sum.
  The error-collection shape a real front-end has. Stresses a SUM variant carrying TWO `List` payloads
  grown in PARALLEL across the recursion, then each projected. 8 `@test`s incl. a partition invariant
  (`good-count + error-count = length`) and order preservation. Confirmed WORKING (no new bug): both
  lists' values survive the recursion exactly.
- `src/apply-ty.cdz` — the function-application step of a type checker: `apply-fun(TFun(a,b), arg)` →
  `Some b` if `arg` structurally equals `a` (recursive `ty-eq` over `TFun`), else `None`; `chain2` threads
  the `Option` across two applications (short-circuit on `None`). Exercises recursive structural type
  equality + Option-chained fallible application. 11 `@test`s. Uses a top-level `tag` helper for nullary
  comparison (working around the nested-nullary-match bug above).
- `src/depth-guard.cdz` — a recursion-limit / stack-safety guard: `check` returns `Result Int64 String` —
  `Ok(max-depth)` if the `Ast` stays within a limit, `Err` the moment a node exceeds it. Stresses a
  `Result` THREADED through recursion with EARLY-RETURN on `Err` (a deep child short-circuits the sibling
  walk). 9 `@test`s incl. widest-branch-decides + short-circuit-on-deep-first-child. Confirmed WORKING.
- `src/interp.cdz` — a tree-walking INTERPRETER for an expression language with LET BINDINGS, evaluated
  under a `Map String Int64` environment (`Let` extends via `Map.insert`, `Var` reads via `Map.lookup`,
  threaded through recursion). The eval half of a REPL/compiler. 10 `@test`s (arith, nested let, correct
  lexical shadowing, rhs-sees-outer-scope, unbound). The `interp-shadow-restores` case is WITHHELD — it
  exposed the `Map.insert`-mutates-shared-recursive-param miscompile above.
- `src/hash.cdz` — a STRUCTURAL HASH over an `Ast` (rolling polynomial hash mixing tag + payload + child
  hashes), the core of hash-consing / CSE. Key property: CONGRUENCE (structurally-equal ASTs hash equal),
  with order/shape/arity sensitivity. All-scalar recursion (dodges the collection-mutation bug). 9
  `@test`s. Confirmed WORKING.
- `src/quote-build.cdz` — METAPROGRAMMING with the built-in `Ast`: `quote(expr)` reifies a program
  fragment into an `Ast` value (the `Int`/`Name`/`List` prelude sum), and `quasiquote` with an Int-valued
  `unquote` splices a computed number into a template. Walks the reified tree (node count, Int-leaf sum)
  and compares quoted forms by structural `==`. 10 `@test`s. (`unquote` of an already-`Ast` value is not
  yet supported — see the log — so `wrap` splices an `Int64`.) Confirmed WORKING.
- `src/list-ops.cdz` — array-style algorithms over an `Int64` list: `get-or` (indexed read with default),
  `index-of`, `count-of`, `contains`, `reverse` — the index-driven traversals (`List.at`/`len`/`push`) a
  compiler runs over a register file / constant pool / positional args. 12 `@test`s. Confirmed WORKING.
  (⚠ minor prelude gap noted: `List` has no `slice` — only `at`/`len`/`push`/`concat`/`update` — so
  `reverse` is index-driven; less impactful than the Map/Set enumeration gap.)
- `src/lex.cdz` — a LEXER: a flat list of ASCII char-codes (`List Int64`) → a `List Token` (multi-digit
  integers, `+ - * /` operators, parens; whitespace skipped, anything else a `Token.Bad`). The first
  front-end stage, and a deliberate stress of two codegen regions: `scan-number` threads a `(value,
  next-index)` TUPLE through self-recursion (the slot-alias family, fixed iter 14 — re-exercised at
  scale) and `lex-from` builds the result by pushing a compound `Token` sum into a single-use tail-loop
  accumulator (the consuming-op region — a single-use accumulator does NOT trip the still-live-binding
  bug). 11 `@test`s. Confirmed WORKING (the tuple-through-recursion + accumulator paths both correct); a
  reversal LOGIC bug I first wrote was caught by the multi-token order tests, which is the framework
  doing its job. Lexes over char-codes not a `String` to sidestep the open runtime-`String.at` equality
  bug (a real text lexer would hit it — which is the point of that finding).
- `src/parse.cdz` — a recursive-descent PRECEDENCE PARSER: a `List Tok` → an arithmetic `Expr` tree
  (`expr = term (('+'|'-') term)*`, `term = factor (('*'|'/') factor)*`, `factor = NUMBER | '(' expr
  ')'`). The port's heaviest MUTUAL-RECURSION stress: `parse-expr ↔ parse-term ↔ parse-factor` each
  return a `(Expr, next-index)` cursor TUPLE, so a `(compound-Ast, Int64)` crosses every mutual-call edge
  (the tuple-through-recursion shape, fixed iter 14, now at mutual scale). To PROVE the tree is shaped by
  precedence (not just parsed), it EVALUATES the result: `1+2*3 == 7` (not 9), `(1+2)*3 == 9`,
  `10-3-2 == 5` (left-assoc), plus a structural `depth` observation. 11 `@test`s. Confirmed WORKING — the
  mutual tuple-cursor threading is correct; no new bug from the parser.
- `src/codegen.cdz` — a STACK-MACHINE BACKEND, and the port's FIRST genuinely MULTI-FILE module in the
  running suite: it `import { Tok, Expr, parse, run } from "parse"` (cross-file), lowers a parsed `Expr`
  to a flat postfix `List Instr`, then EXECUTES it on an integer operand stack. Its correctness contract
  is that the stack backend AGREES with `parse`'s tree-walking `run` for every expression (`exec(compile
  e) == run(tokens)`), verified end-to-end through lex-free token lists. 10 `@test`s. Confirmed WORKING —
  the cross-file link (importing a sum TYPE + its constructors + functions) is correct. 🔑 UNBLOCK: the
  "prelude-collision forces same-file" constraint only bites a type SHADOWING a prelude name (`List`/
  `Bool`); a custom type like `Expr` crosses files fine via `export { Expr.* }` (wildcard ctors) +
  `import { Expr } from "…"`. This retires the port's long-standing single-file limitation for
  non-colliding types.
- `src/peephole.cdz` — a PEEPHOLE OPTIMIZER over the `Instr` stream (imported cross-file from `codegen`,
  a TWO-HOP chain peephole → codegen → parse). It constant-folds a `Push a, Push b, BinOp o` triple to a
  single `Push (a op b)` and ITERATES to a fixpoint, so a fully-constant program collapses to one
  instruction. Stresses multi-element list-window matching + a fixpoint loop; correctness is checked
  against `codegen.exec` (`exec(optimize p) == exec(p)` — optimizing never changes the value, and a
  constant program optimizes to exactly ONE push). 7 `@test`s. Confirmed WORKING — the transitive
  two-hop import + fixpoint iteration are correct. (Finding G below was surfaced while probing the
  optimizer's List ops, not by the optimizer itself.)
- `src/label.cdz` — a NODE-NUMBERING pass driven by a `Fresh` EFFECT (gensym): assign a unique id per
  `Expr` node (imported cross-file from `parse`) via `Fresh.next()`, a `handle Fresh(0)` supplying 0,1,2,…
  The canonical compiler use of an effect (SSA/tyvar/label ids without a hand-threaded counter). Threads
  the effect in the SINGLE-RECURSIVE-SPINE shape the tail-resumptive fold handles correctly — counts nodes
  PURELY first, then draws that many ids in one linear loop (the "flatten, then gensym" structure) — since
  a self-recursive effectful fn with TWO sibling recursive calls in a match arm MISCOMPILES (findings
  below). Checks the drawn count == `count-nodes` and the id sum == the Gauss closed form N*(N-1)/2. 8
  `@test`s. Confirmed WORKING (in the single-spine shape).
- `src/verify.cdz` — a STACK-MACHINE VERIFIER: statically check an `Instr` program (cross-file from
  `codegen`) is stack-safe BEFORE it runs, returning `Result(Int64, String)` — Ok(final depth) or
  Err(underflow / wrong-depth). The "verify the lowered form" stage (wasm's own operand-stack validator in
  miniature): abstract-interpret tracking only the depth, reject anything that would fault. Stresses
  `Result` early-return threaded through a recursive walk (short-circuit on the first Err) + the cross-file
  `Instr` sum. Contract ties it to the VM: every compiled `Expr` verifies as `Ok(1)` (one result), and
  hand-built malformed programs (bare BinOp, one-operand BinOp, two-value stack) are rejected with the
  right reason. 10 `@test`s. Confirmed WORKING.
- `src/unparse.cdz` — a PRETTY-PRINTER for the arithmetic `Expr` (cross-file from `parse`): render a tree
  back to infix source with MINIMAL parenthesization — parens ONLY where precedence or left-associativity
  requires (the inverse of `parse`; the "render IR to source" a compiler needs for errors / a formatter).
  A child is wrapped iff it binds LOOSER than its parent, or ties on the RIGHT of a left-assoc op
  (`10-(3-2)` keeps its parens, `10-3-2` and `1+2*3` don't). Stresses precedence-aware recursion + string
  building; the round-trip through `parse` is checked (`parse "(1+2)*3"` → tree → `unparse` → `"(1+2)*3"`).
  10 `@test`s. Confirmed WORKING. (⚠ hit the `bin`-is-reserved papercut below: a `Bin`-node builder named
  `bin` couldn't be called — renamed to `mkb`.)
- `src/encode.cdz` — the INVERSE of `decode`: serialize an `Ast` to a flat byte buffer at RUN TIME
  (`Ast → Bytes`, via `Bytes.of`/`Bytes.concat` + `UInt8.wrap` over recursively-assembled fragments) —
  runtime byte CONSTRUCTION, the complement to `decode`'s reading. Its `@test`s prove the full ROUND-TRIP
  end to end at run time: `encode` then `decode` preserves the tree STRUCTURE and every Int/Bool value
  exactly (node count + Int-leaf sum survive; verified on flat, deeply-nested, and empty trees). 10
  `@test`s. So the port now runs a real `bytes → Ast → bytes` pipeline (Name/Str content still a
  placeholder pending runtime `String.from-bytes`).

A `resolve` pass (lexical scope-check accumulating unbound-variable diagnostics — a `Set String` scope
threaded down, a `List String` of faults threaded through, `Let` binding + shadowing) is written and
`cdz check`s clean; its logic runs correctly when exported singly, but the FULL module's `@test` suite
DECLINES on the borrow-ownership gap at the test-emit layout (threshold-dependent — 1 test compiles, the
full suite doesn't). Kept as `repros/decline-borrow-scope-resolver-test-emit-threshold.cdz`.

The `infer` pass (HM inference over an expression language, composing `unify`) is written and `cdz
check`s clean but currently lives in `repros/blocked-infer-cross-file-hm-borrow.cdz` — its `@test`s
DECLINE at emit on the `Map.lookup`-returned-heap-value borrow bug (see the log). It was the port's
first genuine cross-file module (importing `unify`), which is what drove the `cdz test` import-following
fix; only the borrow bug — not the linking — keeps it out of the running suite.

Planned, following the rcdzc pipeline: decode (binary AST → `Ast`, NOW RUNNING for structure + scalar
leaves) · resolve · infer (Hindley-Milner) · lower (→ core) · encode/emit. Fundamentally bytes → bytes.

### Decode — RUNNING (structure + scalar leaves), one seed gap remains for string content

`src/decode.cdz` reconstructs a recursive `Ast` from a flat byte buffer at run time, over the port's own
compact self-describing wire form (tag byte + payload; a List is a count then its children). It threads
POSITION SEPARATELY (recompute the cursor via `nsize`, return the value bare) to sidestep the surviving
loop-transform wrong-value bug. The full tree STRUCTURE + every Int/Bool leaf value decode exactly
(10 `@test`s: leaf values, flat/nested/deep lists, empty list, Int-value round-trip). ONE gap remains:
runtime `String.from-bytes` declines, so a Name/Str leaf's CONTENT is a placeholder `""` — the reason a
faithful decoder against the full `rcdzc/src/codec.rs` container (which is dense with `Name` leaves)
still needs that seed fix; the STRUCTURE-and-scalars decode proves the arena→tree design end to end.

## Language issues found (stress-test log)

- **FIXED** (seed `rcdzc` db.rs `scan_type_decl`): a `///` doc comment on a `type` declaration was
  mis-parsed — the ML reader attaches the doc as a `(doc …)` form after the type name, and the sum scan
  read it as a bogus `doc` variant (CDZ0201 "declared more than once"). Now the scan skips a leading
  `(doc …)`, mirroring how a `def`'s leading doc is stripped.
- **Note (not a bug):** author nested `match` via `sexpr → ml`; the reader resolves nesting by greedy
  last-arm absorption, so a hand-written inner `match` easily mis-attaches its catch-all to the outer
  match (CDZ0210 non-exhaustive + CDZ0213 unreachable). The printer's own output round-trips correctly.

- **✅ FIXED (seed `rcdzc`, 2026-07-14): a plain `//`/`///` comment HID the following top-level form.**
  `repros/reject-line-comment-hides-toplevel-form.cdz` now checks CLEAN. The reader wraps a leading line
  comment as `(comment "…" <next-form>)`; the top-level scan didn't see through it, so the wrapped
  `def`/`type`/`export`/`@test`/`effect` was invisible ("unbound name `comment`"). A sibling landed
  `strip_comments` in `Db::load` (`db.rs`@`1a606980`, peels the wrapper before every scan) fixing all
  the single-file placements this loop had catalogued (leading/between-defs/trailing-inline/blank-after,
  `//` and `///` alike). **This iteration extended the fix to the LINK path:** a `//`/`///` comment on
  an `(import …)` was STILL hidden, because `rcdzc::link` scans imports off the RAW arena BEFORE
  `Db::load`'s strip — so a documented import was spliced as an unmodeled top-level form ("`import` … not
  modeled"). Fixed by peeling comments in `compile.rs::link_inputs` (each file's arena, post-decode)
  + `cdz`'s own `declared_import_paths` closure-detector; regression test
  `crates/cdz/tests/check_imports_cli.rs::a_comment_on_an_import_is_seen_through_by_the_link_scan`. So a
  top-level comment (incl. on an import) is now fully supported.

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
  `src/traverse.cdz` now provides these hand-rolled (map/filter/fold + Ast predicate-count/fold).

- **OPEN (seed `rcdzc` — MISCOMPILE, silent wrong value): a consuming persistent-collection op MUTATES a
  STILL-LIVE binding (Map/List/Set).** 🔬 ROOT + MINIMAL form (iter 33)
  `repros/miscompile-consuming-op-mutates-still-live-binding.sexp` — no recursion, no self-call, one
  `let` used twice: `let xs = [7] in (List.len (List.push xs 9)) + (List.len xs)` returns **4**, not 3 —
  the `List.push` consumes/mutates `xs` in place, so the later `List.len xs` reads `[7,9]`. ORDER-
  SENSITIVE (the tell): borrowing BEFORE the consume (`(len xs) + (len (push xs 9))`) returns 3
  (correct). 🔬 WAT-CONFIRMED (iter 34): the body does `call build; local.tee 1; …; vec-push; vec-len;
  local.get 1; vec-len` — the tee'd stack copy shares the cell with slot 1, `vec-push` consumes it at
  rc==1 (mutating the trie IN PLACE), and NO `dup` precedes the consume, so the later `local.get 1`
  observes the mutation. ⚠⚠ SCOPE (iter 34, corrects iters 27–29 — which were BACKWARDS): this is
  EXCLUSIVELY a **`let`-binding-in-body** defect. A plain PARAMETER used identically computes CORRECTLY
  (a param is borrowed-from-caller; the call boundary keeps it live), and passing a let-binding ACROSS a
  call is also correct (the arg is protected — the callee often inlines and re-evaluates it into a fresh
  list). Only a consuming op applied to a let-binding DIRECTLY in the let body, with a later read of the
  same binding, miscompiles. So "only a self-recursive shared param fails / let-bound is fine" was
  exactly reversed. The self-call (`repros/miscompile-selfcall-plus-consuming-op-share-param.sexp`) and
  interpreter (`repros/miscompile-map-insert-mutates-shared-recursive-param.sexp`) repros are INSTANCES:
  their consuming op + surviving read both land in a let/arm BODY over one slot — not a recursion or
  parameter-sharing effect. Reproduces for `Map.insert`/`Set.insert`. Fix locus: the consuming-op emit
  (`Core::ListPush`/`MapInsert`/`SetInsert` in `backend/wasm/select.rs`) must `dup` its heap operand when
  the operand is a `LocalRef`/`Param` still READ AGAIN in the enclosing body (a delicate refcount change —
  over-dup leaks — so a seed-agent job; the repro carries the exact missing dup). Blocks ANY pass building
  a modified collection while still needing the original (a diff, "with one more binding", a before/after)
  — including `src/interp.cdz`'s withheld `interp-shadow-restores`.

- **OPEN (seed `rcdzc` — MISCOMPILE, silent wrong value, iter 37): `List.concat` corrupts its still-live
  LHS where `List.push` does NOT — a DISTINCT face of the still-live-binding family.**
  `repros/miscompile-concat-consumes-still-live-lhs-then-element-read.sexp`.
  `let xs = [5,7] in (List.len (List.concat xs [9])) + (e xs 0)` (where `e xs i = List.at xs i or -1`)
  returns **3**, not 8 — after `concat` consumes `xs`, the later `List.at xs 0` reads as if `xs` is EMPTY.
  🔑 DISCRIMINATOR: the IDENTICAL shape with `List.push` instead of `List.concat` computes CORRECTLY (8) —
  so `push` (`vec-push`) dup-guards its still-live list operand but `concat` (`vec-concat`) does not. SHARP
  (each necessary): the op is `List.concat`; `xs` is a still-live `let` binding (a param is safe); the
  survivor is an ELEMENT read via `List.at` reached AFTER the concat (reading `xs`'s LENGTH after is
  correct; reading `xs[0]` BEFORE the concat is correct — ORDER-SENSITIVE); a SECOND `concat xs …` is also
  correct (the LHS handle is not freed — its element ARRAY reads empty), so it is an rc/dup-timing bug on
  the element array, not a use-after-free. Root: the `Core::ListConcat` emit (backend/wasm/select.rs) omits
  the `dup` of a `LocalRef`/`Param` LHS still read later — same missing-dup defect as the push/insert
  length face (which `push` was patched for, `concat` not). Repro + its push-twin isolate the exact op.

- **OPEN (seed `rcdzc` — EFFECT MISCOMPILE, silent wrong value, iter 38): a self-recursive effectful fn
  with TWO sibling recursive calls in a match arm does not thread the handler state between the siblings.**
  `repros/miscompile-effect-state-not-threaded-across-sibling-recursive-calls.cdz`. `walk (Node Leaf Leaf)`
  under `handle Fresh(0) | next(s) => resume(s, s+1)` (arm: `let a = walk(l) in let b = walk(r) in a + b`)
  should draw id 0 then id 1 → 1; it RETURNS **0** (both leaves draw id 0 — the second sibling call resumes
  from the INITIAL state, not the state the first left). SHARP (each necessary): TWO sibling recursive
  calls in one arm (a SINGLE recursive call threads correctly); a STATE-THREADING handler (a
  constant-handback `resume(1, s)` counts the draws CORRECTLY — 2 — so the draws happen, only the state
  threading is lost); a self-call inside a `match` arm (two calls to a SEPARATE effectful fn thread fine,
  and a single self-recursive draw loop threads fine — `fresh.cdz`). The tell: the BARE `walk(l) + walk(r)`
  arm DECLINES honestly ("not yet reducible by the tail-resumptive fold") but the let-sequenced form
  MISCOMPILES — the fold accepts a shape it cannot correctly thread. Root: the tail-resumptive
  specialization in `effects.rs` (same area as the iter-15/17 mutual-split-branch decline + `#eff3$s0`
  mangled-name leak, but here a SELF-recursive fn, silent). WORKAROUND: keep the effect on a SINGLE
  recursive spine ("flatten pure, then gensym") — `src/label.cdz` does this.

- **OPEN (seed `rcdzc` — EFFECT DECLINE, not a miscompile, iter 38): performing an effect op directly as
  an ELEMENT of a tuple/list/record constructor is not yet reducible by the tail-resumptive fold.**
  `repros/decline-effect-perform-inside-a-compound-constructor.cdz`. `let p = (Fresh.next(), Fresh.next())`
  → "this handler is not yet reducible by the tail-resumptive fold" (no recursion needed; a single perform
  element declines too). CONTRAST — a perform WORKS in an arith operand (`Fresh.next() + Fresh.next()`), a
  call arg (`twice(Fresh.next())`), a sum-ctor payload (`W.Mk(Fresh.next())`), and let-bound before a tail
  self-call. WORKAROUND: prefetch each perform into a `let`, then build the tuple/list/record from the
  bound vars (`let a = Fresh.next() in let b = Fresh.next() in (a, b)` works). Fix locus: extend the
  reducible set in `effects.rs` to a perform in a `Tuple`/`ListNew`/`Record` element (hoist like the
  arith/call/sum cases it already handles). Honest decline (`cdz check` clean, `cdz compile` declines).

- **OPEN (seed `rcdzc` — ML SURFACE GAP, iter 34): a tuple TYPE written `(A, B)` is rejected — must be
  `Tuple(A, B)`.** `repros/reject-ml-tuple-type-paren-comma-spelling.cdz`. `def f(p: (Int64, Int64)) =
  p.0` → CDZ0203 "a parameter's annotation requires a type, but found a non-type": the ML reader lowers a
  paren-comma form in TYPE position to the VALUE constructor head `("tuple" A B)` (the same node a tuple
  LITERAL builds), not the tuple TYPE `(Tuple A B)`. So tuple VALUES/PATTERNS use `(a, b)` but a tuple
  TYPE must use `Tuple(A, B)` — an inconsistency, and the error points at what looks like a type. Ideal:
  accept `(A, B)` in type position (the RHS of a `:` is unambiguously a type annotation) as `Tuple(A,
  B)`. Low-severity (a clean workaround exists); the confusing diagnostic is the real cost.

- **OPEN (seed `rcdzc` — ML SURFACE INCONSISTENCY, iter 35): `Record(field: Type, …)` colon-fields parse
  in a PARAM annotation but NOT in a TYPE DECL.** `repros/reject-ml-record-type-colon-fields-in-type-decl.cdz`.
  `def f(r: Record(a: Int64)) = r.a` is accepted, but the SAME type in `type R = Record(a: Int64)` →
  "expected `,`" at the `:`. The paren-field spelling `Record(a(Int64))` — what the ML printer EMITS
  (`cdz convert … --to ml` renders `Record(x(Int64), y(Int64))`) — parses in BOTH positions, so it is the
  portable form. The two type-position parsers should agree. Separate related gap: no `{ field: Type }`
  brace sugar for an inline record type at all (parsed as a record VALUE). A named record type is a
  newtype over `Record(…)`: `type R = R(Record(a(Int64)))`. Low-severity; parser-consistency + a confusing
  diagnostic. (Companion of the tuple-type gap above — both are paren-comma-vs-annotation surface
  mismatches in type position.)

- **OPEN (seed `rcdzc` — DIAGNOSTIC QUALITY, iter 36; diagnostics-`/loop` territory): a lowercase type-var
  annotation `def id(x: a) = x` gives a bare "unbound name `a`", identical to an unknown uppercase type
  `Foo`.** Cadenza infers generics from UNANNOTATED params (`def id(x) = x` — HM solves the tyvar; there
  is no surface `forall`/tyvar-binder, by design — resolve.rs "`a` is an ordinary parameter, not a
  special type variable"). A programmer from Rust/ML/Haskell reasonably expects a lowercase `a` in a type
  annotation to be a type variable. Ideal diagnostic: recognize a lowercase (esp. single-letter) type
  annotation and hint "generics are inferred — drop the annotation" rather than a generic CDZ0101. Not a
  miscompile — a message-actionability gap. (No repro: it is a message-quality issue, and the diagnostics
  `/loop` owns this class.) 🔑 the WORKING generic spelling: omit the annotation — `def id(x) = x`,
  `def swap(p) = (p.1, p.0)` both compile + monomorphize correctly.

- **OPEN (seed `rcdzc` — ML SURFACE PAPERCUT, iter 40): `bin` and `quote` can be DEFINED but not CALLED as
  function names.** `repros/reject-bin-quote-as-callable-fn-names.cdz`. `bin(…)` is the binary-matching
  construct and `quote(…)` the metaprogramming reifier, captured by the reader at CALL position before
  resolution — so `def bin(x) = x+1` then `bin(5)` → CDZ0201 "a bin segment must be (<kind> <slot>…)"
  (backtick-quoting `` `bin`(5) `` does NOT escape it), and `def quote(x) = x+1` mis-parses the DEFINITION
  itself (`x` reported unused AND unbound — the body is reified). CONTRAST: the other compound-ctor head
  words — `list`/`tuple`/`record`/`map`/`set` — DO work as fn names (define + call + run), so the hijack is
  specific to `bin`/`quote`. Hit organically (a `Bin`-node builder named `bin` in `unparse.cdz`, renamed
  to `mkb`). Ideal: a "`bin`/`quote` is a reserved construct name — rename" diagnostic, and/or let a
  backtick-escaped name resolve to the user binding at call position. Low-severity (rename works); the
  misleading diagnostic is the cost.

- **OPEN (seed `rcdzc` — RESOLVER, both surfaces; RE-DIAGNOSED iter 25): a NULLARY variant DOTTED pattern
  (`Ty.TInt`) in a NESTED match resolves as member ACCESS.**
  `repros/reject-nullary-variant-pattern-in-nested-match.cdz`. `(match x | Ty.TInt => (match x | Ty.TInt
  => …))` → CDZ0201 "member access requires a record". NOT ML-reader-only (my iter-24 note was wrong —
  that test used a differently-wrapped sexpr): the ML reader emits `(. Ty TInt)` for a nullary pattern in
  BOTH the outer and inner arms (identical arena shape), and the S-EXPR surface fails IDENTICALLY. The
  resolver accepts the bare `(. Ty TInt)` as a nullary-ctor pattern at the OUTERMOST match arm but treats
  it as a `Resolved::Member` expression in a NESTED match's arm (likely the inner match's scrutinee type
  not resolving to `Sum` at that point → the scalar-match path lowers the arm pattern as an expr). The
  correctly-WRAPPED nullary pattern `((. Ty TInt))` (a zero-arg ctor application) works nested + outer on
  both surfaces, as does the non-nullary `Ty.TInt(_)`. Two fixes possible: the ML reader wraps a nullary
  ctor pattern, OR the resolver treats a bare `(. T Ctor)` in ANY arm-pattern slot as a nullary ctor.
  WORKAROUND (used by `src/apply-ty.cdz`): a top-level `tag`-helper maps each nullary variant to a scalar,
  so nullary comparison is `tag(x) == tag(y)` and no nested nullary match is needed.

- **OPEN (seed `rcdzc` — MISSING op, spec-backed): a `Map`/`Set` cannot be ENUMERATED.**
  `repros/missing-map-set-enumeration.sexp`. `Map` has `empty`/`insert`/`lookup`/`remove`/`size`/`swap`/
  `take` and `Set` has `of`/`contains`/`insert`/`remove`/`len`/`union`/`intersection`/`difference` — but
  NEITHER has `keys`/`values`/`entries`/`to-list`/`fold` (all → CDZ0201 "no member"). A program can BUILD
  and QUERY a collection but cannot VISIT its contents — so a symbol table can't be walked to emit every
  binding, a free-var set can't be rendered, etc. `collections-and-text.md` §"Map Iteration Is
  Deterministic" describes iteration as a capability (constraining its order), and the canonical form
  already renders the entries, but no PROGRAM op exposes it. 🔑 TRACTABLE: the runtime ALREADY has the
  cursor ops (`map-iter`/`-next`/`-key`/`-val`, `set-iter`/`-next`/`-elem` in `runtime.wit`) used for
  rendering/equality — the gap is purely a front-end `to-list` field + scheme + a `lower`/backend cursor
  loop. No new runtime op needed; a dedicated increment.

- **✅ LEAK FIXED (seed `rcdzc`, 2026-07-14 — landed by THIS loop) → now a clean DECLINE (feature still a
  Todo): a mutually-recursive effectful group where the perform is in a DIFFERENT branch from the mutual
  call.** `repros/decline-mutually-recursive-effectful-split-branch.sexp`. Was: `cdz compile` leaked
  `unbound name ev#eff3$s0` (a specialization mangled name). Now `cdz check`/`cdz compile` both decline
  cleanly: "this handler is not yet reducible by the tail-resumptive fold." FIX
  (`effects.rs::specialize_recursive`): a syntactic guard `perform_and_mutual_call_in_separate_branches`
  declines up front for exactly this shape. NOT a blanket decline — the seed still specializes the
  working same-branch / same-strict-spine mutual shape (`(+ (Ctr.tick) (od …))`); a unit test locks in
  that the decline doesn't leak the internal name, and the gate's mutual-effect corpus case still passes.
  BUILDING the separate-branch specialization (tie the memo knot with per-branch state distribution)
  remains the Todo. A SINGLE self-recursive effectful fn works (see `src/fresh.cdz`).

- **OPEN (seed `rcdzc` — MISCOMPILE, silent wrong value): two live `String` sum-payloads across a
  recursion drop a per-node result.** `repros/miscompile-two-live-string-payloads-across-recursion.sexp`.
  A recursive tree walk where at each node BOTH the node's own `String` key AND its child's key are read
  (two matched `String` payloads live at once), past a >=3-deep recursion, drops one per-node decision.
  A parenthesization count that should be 2 returns 1. The IDENTICAL tree with an Int64 key counts
  correctly (2), and using either key ALONE per node is correct, and a depth-2 tree is correct — so the
  trigger is precisely two overlapping matched-`String` payloads across depth >= 3. Same borrow/ownership-
  of-a-matched-heap-payload family as the `Map.lookup`-return and runtime-`String.at` findings, with the
  depth-threshold sensitivity of the slot-alias family. `src/prec.cdz`'s deep `paren-count` case is
  withheld for this (its shallow cases pass).

- **OPEN (seed `rcdzc` — MISCOMPILE, silent wrong value; ⚠ NARROWED to LOOP-CONTEXT iter 39): runtime
  `String.at` breaks content equality INSIDE A SELF-RECURSIVE LOOP.**
  `repros/miscompile-runtime-string-at-content-equality.sexp`. ⚠ A sibling PARTIALLY FIXED this — every
  STRAIGHT-LINE case now works: `at(runtime i) == "a"` → true, `at(runtime 1) == at(runtime 3)` → true, a
  helper called twice → correct, two sequenced `at`+`==` → correct. The REMAINING failure is LOOP-CONTEXT
  ONLY: `String.at(s,i) == c` inside a self-recursive loop threading `s` as a param returns FALSE every
  iteration (even the FIRST — the char is a valid `Some` of byte-len 1, but `==` yields false). So
  `count-a "banana"` STILL returns 0; minimal survivor `cnt "aa" 0 0` (2-iter loop) → 0 not 2. A
  CONSTANT-index `String.at` folds + compares correctly. This still silently breaks a LOOP-based LEXER
  (which is why `src/lex.cdz` lexes over char-CODES not a String). **ROOT-CAUSED (two
  layers in `backend/wasm/select.rs`):** (1) `Core::ValueEq` `bytes-compact`s a String operand only when
  OWNED, but a `String.at` result is a non-flat `bytes-slice` rope reached via `Option.expect`; (2)
  `heap_operand_ownership` classifies `SumExpect`/`SumPayload`/`Proj` as always `Borrowed` — right for a
  `Map.lookup`, WRONG for a fresh producer like `String.at` (a `bytes-slice` is Owned, `Option.expect`
  transfers it out). The principled fix is layer 2 (a `SumExpect`/`SumPayload` of a producer is Owned) —
  which then also serves the `Map.lookup` borrow-decline family. A naive layer-1-only fix (compact
  borrowed Strings too) makes isolated compares correct but double-frees in a loop; reverted, root-cause
  documented in the repro for a seed agent. Now that the straight-line path is fixed, the residual is the
  LOOP-TRANSFORM emit needing the same slice-compact the straight-line emit got. `String.slice` on a
  runtime string NOW WORKS (iter 39: returns `Option String`, `slice("hello world",0,5)` → Some "hello",
  OOB → None) — the earlier "constant strings only" decline is GONE.

- **OPEN (seed `rcdzc` — HM inference GAP): an unannotated closure passed to a SELF-RECURSIVE HOF fails
  to infer the closure's parameter types.** `repros/reject-inferred-closure-param-through-recursive-hof.
  sexp`. The classic `foldl` written idiomatically — `(def (fold-list f acc xs) … (fold-list f (f h acc)
  t))` with `(fn (x a) (+ a x))` — fails `cdz check`: "a closure's parameter type has no machine
  representation" + CDZ0203 showing the closure typed `(-> Unit (-> Unit Int64))` (the `Unit`/`_` is the
  tell — the param tyvars were never solved). NO sum type needed; a bare `List Int64` reproduces. ROOT:
  at the recursive call `f` is re-passed to a `fold-list` whose own `f` param is not yet solved, so the
  constraint from the closure's USE (`f h acc`) never flows back to the closure; a NON-recursive HOF
  solves it from its single application (works). TWO WORKAROUNDS (each compiles + runs): annotate the
  closure's params, OR annotate the HOF's `f` parameter with the arrow type (`f: Int64 -> Int64 ->
  Int64`). `src/traverse.cdz` uses the arrow-annotation workaround throughout. Impact: `fold`/`map`/
  `filter` over inferred closures — the compiler's bread and butter — need an arrow annotation on the fn
  parameter; the fully-inferred spelling should work.

- **✅ FIXED (seed, by 2026-07-14 — landed by a sibling): a PARAMETERIZED compound-returning export.**
  Was: an export taking a parameter AND returning a compound compiled clean but TRAPPED at run time
  ("expected 1 argument(s), got 0") — the resource-escape `make` didn't forward the export's argument.
  Now `cdz-run … --arg 5` correctly returns the compound (`(: (tuple 5 6) …)`); verified across tuple/
  record/List/BigInt/Result, and run-gated by corpus cases ("a parameterized export returns a runtime
  BigInt/Rational/list computed from its argument"). Witness `repros/fixed-parameterized-compound-
  export.sexp`. ⚠ **PROCESS LESSON:** this loop reported it OPEN for ~14 iterations because the
  per-iteration finding-check rebuilt `cdz` but NOT `cdz-run` — a stale runner kept reproducing the old
  trap. Finding-checks that RUN a component now rebuild `cdz-run` too.

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
  `read-varu` — the loop-transform's scratch allocator reuses a slot across the two widths.
  ✅ **PARTIALLY FIXED (2026-07-14): the INVALID-WASM face is GONE** — a sibling fixed the slot-typing so
  `miscompile-slot-alias-i32i64-loop-tupleproj.sexp`, `miscompile-if-tuple-sum-nontail-recursion.sexp`,
  and `miscompile-two-sibling-ifs-invalid-wasm.sexp` all now COMPILE to valid wasm AND return correct
  values. ✅ **the SILENT WRONG-VALUE face is now ALSO FIXED (2026-07-14, seed rcdzc)** — repro
  `repros/miscompile-tail-loop-projected-sum-wrong-value.sexp` (`main 0` now returns 5). The surviving
  trigger was: (1) a self-tail loop threading a BOXED-SUM handle projected from a tuple (`(. r 0)`) as a
  param, (2) the loop ALSO advances position from the OTHER projection of the SAME tuple (`(. r 1)`) —
  advancing via `(+ pos 1)` instead returned 5, (3) an `if` inside the builder (branch need not be
  taken). **BROADER than the old framing:** the sum need only be a SCALAR-payload boxed sum
  (`Atom Int64 | Zero`) — a compound-payload variant was NOT required. 🔬 **ROOT CAUSE was NOT
  slot-aliasing** (that was the invalid-wasm sibling); it was a PERCEUS use-after-free in
  `backend/wasm/select.rs` `binding_escapes`. `(. r 0)` projects a nested-compound child (the boxed sum)
  OUT of the `let`-bound tuple and threads it into the recursive call; the escape analysis saw only the
  SCALAR sibling projection `(. r 1)` (copies its i64 out), judged `r` fully borrowed, and DROPPED it —
  cascading to FREE the escaped boxed-sum child → garbage 0. FIX: a nested-compound projection ESCAPES
  its operand, so the aggregate is not reclaimed while its extracted child is live. Migrated to the
  corpus (`spec/semantics/10-bytes.sexp`) + a seed unit test. 🔑 **`decode` is now fully UNBLOCKED** —
  both faces of the family are fixed; a decoder may thread a `(node, cursor)` pair with both fields
  projected directly, no work-around needed.

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

- **✅ FIXED (seed `rcdzc` `lower.rs`, 2026-07-14 — landed by THIS loop): a LIST pattern arm may now
  contain SEVERAL refutable (constructor) elements.** Was: `((list (A.I x) (A.N y) c) …)` declined "more
  than one refutable constructor element is not yet supported". FIX: the list-refutable-element desugar
  generalized from ONE ctor position to N — each ctor element gets a fresh binder, all discriminant tests
  are ANDed into the arm guard, and the body re-matches NEST so every ctor payload is in scope. Corpus
  `spec/semantics/05-compound-types.sexp` (two-ctor-element case + second-tag fall-through, gate-verified)
  + updated unit test; witness `repros/fixed-list-arm-multiple-ctor-elements.sexp`. `src/fold.cdz` now
  uses the natural `[Ast.Name(op), Ast.Int(x), Ast.Int(y)]` arm (three ctor elements) — the
  bind-all-then-nested-match workaround is gone.

- **OPEN (seed `rcdzc` — DECLINE, now GENERALIZED): a HEAP value read from `Map.lookup`, RETURNED, then
  CONSUMED in the caller.** "borrowing op operand has an ownership this backend cannot yet prove" (`cdz
  check` clean, `cdz compile`/`cdz test` decline). Two repros:
  `repros/decline-borrow-ownership-returned-map-string-eq.sexp` (the original — `String ==` on the
  returned value) and `repros/decline-borrow-map-lookup-returned-then-matched.cdz` (the GENERALIZATION).
  **SHARP BOUND (bisected 2026-07-14):** the `==` is NOT essential — a plain `match` on the returned
  value triggers it too. The essential ingredients are (a) the value ORIGINATES from `Map.lookup`, (b)
  it is RETURNED across a call boundary (wrapping it in a ctor counts), and (c) a HEAP payload nested in
  it (a `String`, or a sum carrying one) is then read in the caller. A looked-up value with ONLY SCALAR
  payloads returns + inspects fine; consuming it INSIDE the lookup arm (never returning it) is fine; and
  the same extract-and-compare with NO map is fine — so it is specifically a borrowed heap value
  escaping its lookup scope via a return. An explicit `copy()` in the arm does NOT sidestep it. **This
  is the single biggest blocker to running a real pass in the port:** `repros/blocked-infer-cross-file-
  hm-borrow.cdz` (a full HM inference pass) `cdz check`s clean but its tests DECLINE, because its `Ref`
  arm looks a binding up in a `Map String Ty` env and returns it. `src/subst.cdz` sidesteps the original
  narrow form by SHAPE-checking (`match … ((Ast.Name _) …)`) but there is no honest sidestep for an
  env-lookup whose whole purpose is to return the looked-up type. Likely fix locus: the Perceus/borrow
  analysis — a heap value read out of a persistent collection needs an owned (dup'd) handle when it
  escapes via a return. A THIRD, threshold-dependent face: `repros/decline-borrow-scope-resolver-test-
  emit-threshold.cdz` (a scope resolver threading a `Set`+`List` through recursion) `cdz check`s clean
  and runs correctly when exported singly, but its full `@test` suite DECLINES under the `EmitTests`
  layout — 1 test compiles, the whole suite doesn't (aggregate/total-locals sensitivity, like the
  slot-alias bug). So the borrow gap also scales with the test-emit boundary size.

- **OPEN (seed `rcdzc` — a `///` doc comment on an `import` HIDES it; extends the line-comment finding).**
  A `///` doc comment on a `def`/`type`/`module` is stripped by the top-level scan (works), but a `///`
  on an `(import …)` is NOT — the wrapped import becomes invisible ("unbound name `comment`" + the
  imported names unbound). Same root as the `//`-line-comment gap (`repros/reject-line-comment-hides-
  toplevel-form.cdz`): the doc/comment-strip covers `def`/`type`/`module` but not `import`. Workaround:
  put the `import` FIRST (no leading doc), then a `///` module doc on the first `type`/`def`.

- **OPEN (seed `rcdzc` — GAP/BUG): quasiquote `unquote` of an already-`Ast` value is rejected.**
  `repros/reject-unquote-of-an-ast-value.sexp`. `(quasiquote (+ (unquote sub) 1))` with `sub : Ast` →
  CDZ0201 "a variant constructor's payload has declared type Int64, but a value of type Ast was
  applied". `(unquote n)` where `n` is a plain `Int64`/literal WORKS (wrapped as `Ast.Int`); only an
  already-`Ast` value fails — `unquote` wraps by the template slot's leaf type instead of splicing an
  Ast node as-is. metaprogramming.md says `,<expr>` inserts its RESULT at that position; when the result
  IS an Ast, that should splice the node. Blocks the canonical AST-building macro (embed a computed
  subtree). Confirmed WORKING otherwise: quote structural `=`, walking a quoted form via own `Ast`
  match, quoted Ast escaping to host, `unquote-splice` of a list, `(unquote <plain-value>)`.

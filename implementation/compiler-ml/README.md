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
— see the `mlrepro-import-prelude-collision` queue item), so each module tests itself, but a test may now
freely IMPORT non-colliding names from a sibling.

## Structure (mirrors the rcdzc stages)

Source modules live under `src/`; `Project.cdz`, `README.md`, and `TESTING.md` sit at the top. (Language
issues this port finds are filed in the shared queue — see "Language issues found" below — not a private
`repros/` dir.) Every `src/` module carries same-file `@test`s (run the whole suite with `cdz test .`).
Current `src/` modules:

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
  (`(x (λx.x))` — outer x free). Confirmed WORKING (no new bug). (The former `Set.len`/`Map.size` count
  inconsistency is RESOLVED: `Map.size`→`Map.len` in the collection-op naming cutover, so both are `len`.)
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
- `src/cfold.cdz` — a SOURCE-LEVEL CONSTANT-FOLDING pass over a small `Num`/`Add`/`Mul` language: rewrite
  bottom-up, collapsing constant subtrees (`1 + 2*3` → `7`) and applying algebraic identities (`e+0`→`e`,
  `e*1`→`e`, `e*0`→`0`) via smart constructors. The classic pre-codegen shrink — a PURE tree-to-tree
  rewrite (no env, no mutation), the shape the seed folds cleanly. Contract: `eval(fold e) == eval(e)`
  (meaning-preserving) AND `size(fold e) <= size(e)` (never grows) AND idempotent. 10 `@test`s. Confirmed
  WORKING. (A `Map`-env interpreter with SHADOWING is the UNSAFE cousin — it trips the still-live-binding
  miscompile; see `mlrepro-miscompile-env-interpreter-shadow-corrupts-outer-binding.cdz` (queue), so this optimizer
  stays a pure rewrite.)
- `src/ssa.cdz` — an SSA LINEARIZER: flatten the arithmetic `Expr` (cross-file from `parse`) into a
  straight-line list of THREE-ADDRESS instructions (`Lit(dst, v)` / `Op(dst, opcode, ra, rb)`), each
  defining a fresh SSA register — the pre-register-allocation form a real backend lowers to. Mints fresh
  register ids by THREADING A COUNTER through the result (`(reg, next, instrs)` triple), NOT a `Fresh`
  effect: the effect-based gensym over a two-child tree needs two sibling recursive calls under a
  state-threading handler, which the tail-resumptive fold declines (see
  `mlrepro-decline-effect-state-across-sibling-recursive-calls.cdz` (queue)) — pure counter-threading is the
  correct portable design. An `interp` over the instruction list (a `Map Int64 Int64` register file)
  re-executes the SSA form and must agree with `parse`'s tree-walking `run`. 10 `@test`s. Confirmed
  WORKING (registers dense 0..n-1, instr-count == node-count, SSA eval == tree-walk).
- `src/strlex.cdz` — a STRING LEXER: tokenize a real `String` of source text into a `List Token`
  (multi-digit ints, `+ - * /`, parens; whitespace skipped; else `Token.Bad`). The TEXT version of
  `lex.cdz` (which lexes char-CODES to sidestep the once-broken runtime-`String.at` loop), now possible
  because the runtime-`String.at`-content-equality bug is FIXED — scanning `String.at(s,i) == "<c>"` in a
  self-recursive loop works. It exercises exactly the once-broken shape (String.at in a loop, its one-char
  result compared to literals, the string operand threaded through the recursion), so lexing correctly is
  the end-to-end proof of the fix. 10 `@test`s. ⚠ INLINES `String.at`'s match in each loop arm rather than
  a `char-at` helper — a String-returning helper whose result is `==`-compared IN A LOOP declines (finding
  below).
- `src/parse-checked.cdz` — a FALLIBLE recursive-descent parser: the same arithmetic grammar as `parse`
  (which is total), but returning `Result(Expr, ParseError)` so a malformed token stream yields a
  DIAGNOSTIC (what + at which token index) instead of a silently-wrong tree — what a real front end does.
  Reuses `Tok`/`Expr` (cross-file from `parse`). Stresses `Result` threaded through MUTUAL recursion
  (`pexpr ↔ pterm ↔ pfactor` each return `Result(Tuple(Expr, Int64), ParseError)`, short-circuiting to the
  first `Err`). Reports `ExpectedFactor`/`ExpectedRParen` with position; tests cover empty / leading-op /
  missing-`)` / trailing-token / dangling-op errors AND agreement with the total parser on valid input. 12
  `@test`s. Confirmed WORKING. (⚠ hit the dotted-nullary-arm finding below: `Tok.TRP` in the `'(' expr ')'`
  arm declined; bare `TRP` fixes it.)
- `src/calc.cdz` — the FRONT-END CAPSTONE: a complete `String` → value calculator composing THREE modules
  over a 3-hop transitive import chain (calc → `parse-checked` → `parse`). A string lexer builds `parse`'s
  canonical `Tok` directly (so its output feeds `parse-checked` with no bridge), then the fallible parser
  + evaluator produce a value or a positioned error. `calc "2 * (3 + 4) - 1"` → 13; malformed input (``,
  `"1 @ 2"`, `"(1 + 2"`) reports errors via `parse-checked.ok`. Real end-to-end text-in/value-out (a REPL
  / `--eval`). 12 `@test`s. Confirmed WORKING — the deep transitive import + the string→Tok→Expr→value
  flow both correct.
- `src/tycheck.cdz` — a TYPE-CHECKER for a two-type expression language (`Int` + `Bool`, with arithmetic,
  comparison, logic, `Not`, and a conditional). The middle-end pass: assign each node a `Ty` and REJECT an
  ill-typed program (`1 + true`, `if 3 then …`, `if c then 1 else true`) with a positioned `TypeError`
  rather than letting it reach codegen. `check e : Result(Ty, TypeError)` short-circuits to the first
  error; `well-typed`/`checked-eval` gate a reference evaluator on the check. 14 `@test`s (each typing
  rule + its rejection + nested error propagation + a well-typed/ill-typed eval gate). Confirmed WORKING.
  ⚠ Two seed findings hit + worked around here: (1) the mutual-helper `if teq(ct,TBool) then match
  check(t)` factoring declined "no local slot" → inlined into one self-recursive `check`; (2) nested
  nullary matches (`match rt { TInt => … | TBool => … }`, `Result.Ok(TInt)` patterns) drew spurious
  CDZ0306/CDZ0213 warnings → compare types via a scalar `tag` instead. Both in the findings log.
- `src/scopecheck.cdz` — a SCOPE / FREE-VARIABLE resolver over the canonical homoiconic `Ast` (cross-file
  from `ast`): count unbound `Name`s under a `Set String` of bound names, recognizing the `(let ((x v))
  b)` and `(fn (p…) b)` binder forms. 🎉 UNBLOCKED iter 48 — this was withheld (iter 42) because the
  still-live family corrupted the `Set` scope threaded through the recursion (`free-vars (let ((x 5)) (+ x
  x))` → 3 not 1); the sibling compiler-side dup fix landed, so a `Set` scope now survives a recursive
  `Ast` walk and this real resolver runs. Walks each list by INDEX (keeps `node` live via borrowing
  reads). 10 `@test`s (let/fn binding, shadowing, recursive-let, deep nesting). Confirmed WORKING.
- `src/encode.cdz` — the INVERSE of `decode`: serialize an `Ast` to a flat byte buffer at RUN TIME
  (`Ast → Bytes`, via `Bytes.of`/`Bytes.concat` + `UInt8.wrap` over recursively-assembled fragments) —
  runtime byte CONSTRUCTION, the complement to `decode`'s reading. Its `@test`s prove the full ROUND-TRIP
  end to end at run time: `encode` then `decode` preserves the tree STRUCTURE and every Int/Bool value
  exactly (node count + Int-leaf sum survive; verified on flat, deeply-nested, and empty trees). 10
  `@test`s. So the port now runs a real `bytes → Ast → bytes` pipeline (Name/Str content still a
  placeholder pending runtime `String.from-bytes`).
- `src/iter.cdz` — a LAZY, PULL-DRIVEN ITERATOR realizing the `iterators-as-lazy-pull-computations`
  proposal. `next : it -> Option((elem, rest))` — total (`None` at exhaustion), pure (the second value
  is the REST iterator, i.e. the next state — so an iterator is re-steppable/shareable, no "consumed
  iterator" hazard). Producers (`empty`/`from-list`/`range`), lazy transformers
  (`map`/`filter`/`take`/`drop`/`take-while` — build a wrapping step, force nothing), and consumers
  (`fold`/`count`/`sum`/`collect-list`/`find`/`any`/`all` — drive the pull). ENCODING = reified
  (defunctionalized): a sum of step-shapes `next` interprets, NOT a stored `Unit -> …` thunk (that
  DECLINES — see the Unit-thunk finding below), so each stored closure is over the element type, never
  `Unit`. Laziness is real and tested (`take 3` of a million-element `range` pulls exactly 3). 18
  `@test`s. **Monomorphic over `Int64` — a SPIKE:** the generic `Iter(a)` is blocked by two inference
  gaps (`mlrepro-decline-generic-iterator-composed-transformers.cdz` (queue),
  `mlrepro-reject-user-generic-type-var-in-annotation.cdz` (queue)); the any-element-type version is the real
  goal, gated on those.

The λ-calculus front-end family — resolve/scope through evaluation, over the canonical `Ast`, each
threading its state through the idiomatic mutual `_ / _-list` recursion the still-live-binding backend
fixes unblocked:
- `src/alpha.cdz` — ALPHA-EQUIVALENCE: two terms equal up to consistent renaming of BOUND variables
  (threads a renaming `Map String String` through the mutual walk). Distinct from `ast-eq` (structural)
  and `scopecheck` (free-var counting) — `(fn (x) x) ~ (fn (y) y)`, K vs KI distinguished, free vars rigid.
- `src/debruijn.cdz` — NAMELESS (de Bruijn) conversion: each bound var → its de Bruijn index (binders
  between use and binder), each `(fn (x) …)` loses its param name. Makes α-equivalence syntactic
  (α-equal terms convert to identical nameless terms). Threads a `Map String Int64` name→binding-depth.
- `src/beta.cdz` — capture-free BETA-REDUCTION over nameless terms: the `shift`/`subst`/`beta1` index
  calculus reduces `(app (fn () body) arg)` with no capture (the payoff of the nameless form).
- `src/normalize.cdz` — normal-order BETA-NORMALIZATION: drives `beta1` to β-normal form (leftmost-
  outermost redex + a congruence walk under binders/args), fuel-bounded for totality.
- `src/symtab.cdz` — a SYMBOL TABLE with DETERMINISTIC enumeration via the `Map.to-list`/`Set.to-list`
  ops (canonical-order iteration): ordered names, value sum, indexed lookup, a canonical name Set — the
  shape a back end takes to emit declarations / a symbol dump in a reproducible order.

A `resolve` pass (lexical scope-check accumulating unbound-variable diagnostics — a `Set String` scope
threaded down, a `List String` of faults threaded through, `Let` binding + shadowing) is written and
`cdz check`s clean; its logic runs correctly when exported singly, but the FULL module's `@test` suite
DECLINES on the borrow-ownership gap at the test-emit layout (threshold-dependent — 1 test compiles, the
full suite doesn't). Kept as `mlrepro-decline-borrow-scope-resolver-test-emit-threshold.cdz` (queue). A
SCOPE-CHECKER over the CANONICAL `Ast` (`let`/`fn` binders, a `Set String` scope) was withheld at iter 42
for the same still-live-binding reason (its `let` tests lost the extended scope through the recursion) —
🎉 now UNBLOCKED (iter 48, the sibling compiler-side dup fix) and PROMOTED to `src/scopecheck.cdz` (all 10
tests pass).

The `infer` pass (HM inference over an expression language, composing `unify`) is written and `cdz
check`s clean but currently lives in `mlrepro-blocked-infer-cross-file-hm-borrow.cdz` (queue) — its `@test`s
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

## Language issues found

Language issues found by this port live in the shared issue **queue**
(`.claude/fleet/queue/`, archived to `issues/` when resolved) as `mlrepro-*` entries. File a NEW finding
there (write `.claude/fleet/queue/mlrepro-<slug>.<ext>` and send `corpus-bugfix` an `issue`, or the owning
vertical a `note`) — do NOT add it here and do NOT create a file under a private `repros/` directory. One
pipeline for every repro: the queue for open findings, `issues/` for resolved ones.

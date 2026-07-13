---
name: codemod
description: >-
  How to structurally search and rewrite Cadenza programs with the `cdz query` / `rewrite` codemod
  tool, and how to ask the COMPILER for a semantic fact with `cdz type` / `cdz uses` / `cdz check` (all
  in the unified `cdz` binary). Read this whenever the task is finding or transforming code by SHAPE
  rather than text — structural search-and-replace, a rename/peephole/wrap refactor, a multi-rule
  simplifier pass, running a codemod across files/directories (apply in place, diff preview, or JSON),
  structurally diffing two programs (which subtrees changed), finding duplicated subtrees (exact or
  near-clone / anti-unification), counting occurrences of a form, extracting spans of matching nodes,
  or building on the query/Tree matcher API — OR when the task is a SEMANTIC query the shape layer
  can't answer: the type of a definition (`cdz type`), every source location that references a
  name (`cdz uses`, a span-mapped go-to-references), every well-formedness fault (`cdz check`,
  "diagnostics as you type"), a name's definition (`cdz def`, go-to-definition), the bindings
  visible at a point (`cdz scope`, variable scope tracking), or a module's exported interface
  (`cdz exports`). Covers the
  `,x`/`,@xs` pattern language,
  structural guards (`is-literal`/`head-is`/`matches`/`not`), relational context (`inside`/`has`),
  multi-rule sets + traversal strategy, multi-file/`--write`/`--diff`/`--json`, the `diff`
  (structural tree-diff) and `clones` (content-hash duplicate + `--near` anti-unification)
  subcommands, `lint` mode (anti-pattern checker / CI gate), the semantic `type`/`uses` compiler
  queries, the COMBINED `cdz query --where 'type-of(x) = T'` (a structural match filtered by a compiler
  type predicate), the CLI, the library API, and the self-hosted sidecar map.
---

# Structural query & rewrite (codemod) for Cadenza

A codemod here is **structural search-and-replace over the homoiconic AST**, not a text patch.
Because every Cadenza form is `(head child…)` data, a pattern that matches code *is itself code* — a
rewrite rule reads in the shape of what it rewrites. The structural tool lives in `cadenza-syntax`
(the `query` module + the `query`/`rewrite`/`diff`/`lint`/`clones` subcommands). It is **Rung 2** of
`implementation/DESIGN-query-engine.md` (a built-in Rust driver) standing in for the eventual
self-hosted sidecar — see `implementation/PROTOTYPE-codemod.md` for the full write-up.

> **The binary is now `cdz`, not `cdz-syntax`.** The front-end (convert + this codemod) and the
> compiler (`compile`, plus the semantic queries below) were unified into ONE tool, `cdz`, over both
> libraries (`cadenza-syntax` + `rcdzc`); the standalone `cdz-syntax`/`rcdzc` bins were retired. Every
> `cdz-syntax query …` is now `cdz query …` (the subcommands and flags are unchanged — same code). See
> [[cdz-unified-binary-cli]].
>
> **One consequence is a genuinely new capability.** Because `cdz` holds BOTH libraries in one
> process, it also offers the SEMANTIC queries the structural layer deliberately can't — `cdz type`
> (a definition's inferred type) and `cdz uses` (every reference, as `file:line:col`). Those reach
> into the compiler (`rcdzc`), so they are NOT codemod guards; they are a sibling surface documented
> in [§Semantic queries](#semantic-queries-cdz-type--cdz-uses--the-compiler-as-oracle) below. The
> structural pattern language stays purely shape-based (no type/scope guards); the semantic answer is
> a separate command.

## The pattern language (not a new language)

A pattern and a rewrite template are ordinary **s-expression text** with two metavariable sigils the
reader already produces — no grammar is invented:

| Sigil  | Reads as             | Meaning                                              |
|--------|----------------------|------------------------------------------------------|
| `,x`   | `(unquote x)`        | bind **one** node to `x`                             |
| `,@xs` | `(unquote-splicing)` | bind a **run** of zero-or-more sibling nodes to `xs` |
| `,_`   |                      | wildcard: match one node, bind nothing               |
| `,@_`  |                      | wildcard run                                         |

Everything else is a **structural literal** that must match exactly. `(+ ,x 0)` matches an addition
whose second operand is the integer `0`, binding the first operand to `x`.

Rules to know:
- **Consistency (non-linear):** a repeated metavar must bind structurally-equal subtrees — `(+ ,x ,x)`
  matches `(+ a a)` and `(+ (f 1) (f 1))`, not `(+ a b)`. Wildcards `,_` are exempt.
- **Several splices per list,** as long as no two are ADJACENT — a fixed element between them anchors
  each run boundary: `(call ,head ,@mid ,last)`, and the clause-delete idiom `(F ,@before X ,@after)`
  (delete `X` from ANY position of a variadic form). Only directly-adjacent splices (`,@a ,@b`) are
  rejected (nothing divides the run).
- **Unbound template var ⇒ that site is left unchanged** (reject-don't-corrupt).

These are the same quote-pattern shapes (`` `(+ ,x 0) ``) that `spec/semantics/20-structural-editing.sexp`
pins as the self-hosted end state, so a rule written today reads identically later.

**Guards** constrain a metavar structurally: `,(name guard…)` (conjunctive). Guards are
`is-literal`, `is-name`, `is-int`/`is-float`/`is-str`/`is-bool`, `is-atom`/`is-list`,
`(head-is NAME)`, `(matches PAT)`, `(not GUARD)`. E.g. `(+ ,(x is-literal) ,y)`, `(f ,(g (head-is *)))`.
An unknown guard is rejected at compile time. **All guards are purely structural — there are NO
scope/binding or type guards** (`refs`/`defines`/`type-of`) *inside a pattern*; binding/type analysis
is the compiler's job, not the matcher's, to avoid duplicating the resolver. When you need a semantic
fact — "what is the type of this node", "where is this name used" — use the `cdz type` / `cdz uses`
compiler queries ([§Semantic queries](#semantic-queries-cdz-type--cdz-uses--the-compiler-as-oracle)),
which ask `rcdzc` directly rather than re-deriving it in the pattern layer. To COMBINE the two — a
structural selector filtered by a type predicate — use `cdz query PATTERN --where 'type-of(x) = T'`
([§Combined query](#combined-query-cdz-query----where--shape--meaning)); the pattern stays purely
structural, the `--where` clause adds the semantic filter.

## CLI

The binary is `cdz` (at `target/<profile>/cdz`, or `cargo run -p cdz --`). `--from`/`--to` infer from
a FILE extension (`.cdz`/`.ml`→ml, `.sexp`→sexpr, `.bin`→binary); stdin needs an explicit `--from`.
The codemod subcommands (`query`/`rewrite`/`diff`/`lint`/`clones`) are the front-end surface; `cdz`
also has `convert`, `compile`, and the semantic `type`/`uses` queries (below).

```console
# find every additive-identity site; prints "byte START-END: <form>" + "$var = …" bindings
$ printf 'f(a + 0, b * 1)' | cdz query '(+ ,x 0)' --from ml
byte 2-7: (+ a 0)
  $x = a

# just the count
$ printf 'g(x + 0) + (y + 0)' | cdz query '(+ ,e 0)' --from ml --count
2

# rewrite: (+ ,x 0) -> ,x   (result on stdout, "rewrote N site(s)" on stderr)
$ printf 'f(a + 0, b + 0)' | cdz rewrite '(+ ,x 0)' ',x' --from ml --to ml
cdz: rewrote 2 site(s)
f(a, b)

# wrap a call with a splice template
$ printf '(risky a b)' | cdz rewrite '(risky ,@args)' '(log (risky ,@args))' --from sexpr
(log (risky a b))

# delete a clause from ANY position of a variadic form (two splices around a fixed anchor)
$ printf '(case foo (doc "d") (needs bar) (result 1))' \
    | cdz rewrite '(case ,@a (needs ,_) ,@b)' '(case ,@a ,@b)' --from sexpr
(case foo (doc "d") (result 1))

# a guard + a relational constraint
$ printf '(do (+ 1 a) (+ b c))' | cdz query '(+ ,(x is-literal) ,y)' --from sexpr --count
1
$ printf '(do (safe x) (danger (g x)))' | cdz query 'x' --from sexpr --inside '(danger ,@_)'
byte 24-25: x

# a multi-rule peephole set (first match wins), applied in one bottom-up pass
$ printf '(f (+ a 0) (* b 1) (* c 0))' | cdz rewrite --rules peephole.rules --from sexpr
(f a b 0)

# run over a whole directory; --json for machine-readable output
$ cdz query '(+ ,e 0)' src/ --json
[{"file":"src/a.ml","span":{"start":2,"end":7},"matched":"(+ x 0)","bindings":{"e":"x"}}, …]

# preview a rewrite (--diff, file untouched), then apply in place across a dir (--write).
# --write/--diff are FORMATTING-PRESERVING by default: only changed subtrees are spliced at their
# spans, so a hand-formatted file keeps its layout/comments (the diff is minimal, line-by-line).
$ cdz rewrite '(+ ,x 0)' ',x' src/a.ml --diff
$ cdz rewrite '(+ ,x 0)' ',x' src/ --write
# force a canonical whole-file reflow instead (opt out of preserving):
$ cdz rewrite '(+ ,x 0)' ',x' src/a.ml --write --reprint

# STRUCTURAL diff of two programs — which subtrees changed (not text lines)
$ cdz diff before.ml after.ml
1: replace (+ a 0) => a

# LINT: flag anti-patterns; exits non-zero on any `error` (a CI gate)
$ cdz lint src/ --rule '(lint (deprecated ,@_) "avoid" error)'
src/a.ml:2:3: error: avoid

# CLONES: find duplicated subtrees (copy-paste) within/across files
$ cdz clones src/ --min-size 4
clone: 3 occurrences, 4 nodes: (validate config strict)
  src/a.ml:1:11
  src/a.ml:2:11
  src/b.ml:1:11

# NEAR-CLONES: same shape, differing leaves — INFERS the pattern (feed straight into rewrite)
$ cdz clones src/ --near --min-size 3
near-clone: 3 occurrences, 1 hole(s): (scale x ,m0)
  src/a.ml:1:11 …
```

- **Multiple FILEs and directories** are accepted (a DIR is recursed and FILTERED to source
  extensions — READMEs/dotfiles skipped; `--from` overrides the format, not which files are included;
  a named file always honors `--from`); with no FILE, input is stdin. Empty dir warns; in a sweep, one
  bad file is skipped-with-warning (a single named target is a hard error). Human output over several
  files is grouped by `=== file ===`.
- `query` prints matches (span + bindings), `--count` the number (per file + a `total:`), or `--json`
  a flat array `[{file?, span, matched, bindings}]`. No match ⇒ empty, exit 0. Filter by structural
  context: `--inside`/`--has`/`--not-inside`/`--not-has PAT` (repeatable, conjunctive; ancestry/
  containment only — no scope).
- `rewrite PATTERN TEMPLATE` (or `rewrite --rules FILE`) prints the rewritten program to **stdout** and
  the count to **stderr** (stdout stays a clean, pipeable program; the count line is prefixed `cdz:`).
  `--rules FILE` = `(rule PAT TMPL)`
  forms (first match wins); `--top-down` (default bottom-up); `--fixpoint` (bounded). Output modes:
  `--diff` previews a unified diff (file untouched), `--write` applies in place (FILE inputs only,
  changed files only), `--json` emits `{file?, count, rewritten}` (mutually exclusive with `--write`).
  **FORMATTING-PRESERVING by default**: when the output surface matches the input and the input carries
  spans (ml/sexpr), only the changed subtrees are spliced at their source spans — all other bytes
  (whitespace, newlines, comments, hand-alignment) are copied verbatim, so a bulk edit of a
  hand-formatted file gives a **minimal, reviewable diff** instead of a whole-file reflow. A deleted
  list child removes its own line cleanly (no dangling blank line). `--reprint` opts out (canonical
  whole-tree reflow, the old behavior — for deliberate normalization); a cross-surface `--to` always
  reprints. Always **validates as a transaction**: the edited text is re-parsed and checked
  structurally-equal to the rewritten tree; a splice that can't be validated **falls back to a reprint**
  (with a warning), and a result that doesn't round-trip is **rejected** — never a half-applied edit.
- `diff FILE-A FILE-B` is a **structural** (subtree) diff, not a line diff: it reports each changed
  node by path — `PATH: replace OLD => NEW` / `add NEW` / `remove OLD`, or `--json`
  `[{path, kind, old?, new?}]`. Same-head lists recurse positionally (a changed operand is one
  point-change), differing arity aligns by LCS. Use it to review what a rewrite/edit changed to the
  tree, independent of formatting. (Distinct from `rewrite --diff`, which is a line-based unified diff.)
- `lint [FILE|DIR…] --rules FILE | --rule '(lint …)'` flags structural anti-patterns. A rule is
  `(lint PATTERN "message" [severity])`, severity ∈ `error`/`warning`/`info` (default `warning`),
  patterns use the full language (guards/splices). Each match → `FILE:line:col: SEVERITY: message`
  (or `--json`). **Exits non-zero iff any `error`-severity diagnostic fired** — a CI gate; warnings
  don't fail. Semgrep-lite for the AST.
- `clones [FILE|DIR…] [--min-size N]` finds **duplicated subtrees** (copy-paste) within and across
  files — the refactoring signal for "extract a shared def". Each subtree gets a Merkle content hash;
  a clone class is ≥2 structurally-equal subtrees (hash-bucketed, `tree_eq`-verified). `--min-size N`
  (node-count floor, default 3) drops trivial dupes; only maximal clones are reported, ranked
  biggest-first. Output: `clone: N occurrences, M nodes: <exemplar>` + `LABEL:line:col` per site, or
  `--json`. Purely structural (no α-equivalence).
- `clones --near` finds **near-clones** (Type-2): subtrees sharing a shape but differing in leaves.
  It buckets by a shape hash then **anti-unifies** — the inverse of matching — INFERRING the pattern
  with `,mK` holes where sites differ (shared when positions vary together). Output:
  `near-clone: N occurrences, H hole(s): (scale x ,m0)`. That pattern *is* a `rewrite` pattern — it
  re-matches every site — so a near-clone report **feeds straight back into `cdz rewrite`** to
  factor the duplication into one call.
- Because the parser recovers from errors, `query` works over **broken input** too: it warns on stderr
  and still runs the query over the recovered tree.

## Semantic queries (`cdz type` / `cdz uses` / `cdz check` / `cdz def` / `cdz scope` / `cdz exports`) — the compiler as oracle

The codemod above is a **shape** layer: it never resolves a name or infers a type (that would
duplicate the compiler's resolver). When you need a fact only the compiler knows, `cdz` — because it
holds both the front-end AND `rcdzc` in one process — exposes a family of **semantic** queries. They
parse the program keeping its span table, ask the compiler (via its sidecar query engine), and map the
answer back to source. They are TOTAL: an unknown name yields a defined answer, not an error, and a
query answers even for a program that would not fully compile.

```console
# cdz type NAME FILE — the inferred type of a definition, rendered (the same text an annotation uses)
$ cdz type main prog.cdz
Int64
$ cdz type add prog.cdz            # a function renders as its arrow type
(-> Int64 Int64)
$ cdz type ghost prog.cdz          # total: unknown name is a defined answer, not a failure
no such definition `ghost`

# cdz uses NAME FILE — every source location that references a definition/type, as file:line:col
# (a span-mapped go-to-references; the DECLARATION site is excluded — only references)
$ cdz uses helper prog.cdz
prog.cdz:3:12
prog.cdz:4:5

# cdz check FILE — every well-formedness fault, "diagnostics as you type" (no export/run needed).
# Exits non-zero on any error; a clean file prints nothing. An editor's inline squiggles / a CI gate.
$ cdz check prog.cdz
prog.cdz:2:16: error [CDZ0203]: if condition must be Bool, found Int64

# cdz def FILE OFFSET — go-to-definition: the definition of the name at the cursor, as file:line:col.
$ cdz def prog.cdz 49
prog.cdz:1:25

# cdz scope FILE OFFSET — variable scope tracking: every binding visible at the cursor, innermost
# first, as `file:line:col: name : type` (params, let-bindings, match-arm binders).
$ cdz scope prog.cdz 54
prog.cdz:1:41: q : Int64
prog.cdz:1:22: p : Int64

# cdz exports FILE — the module's interface: each exported name and its type, at its definition.
$ cdz exports prog.cdz
prog.cdz:1:17: inc : (-> Int64 Int64)
prog.cdz:1:49: v : Int64
```

> **Hover reads as a presentation, not a raw type** (`type-at`): a grammar keyword shows `keyword def`
> (not `Any`); a DEFINITION shows its signature `name : (-> A B)` (not just the body's return type); a
> reference/use shows the value's type; an untypeable node shows `unknown`; an operator no longer leaks
> its internal `(record …)`.

- **Why these are here and not codemod guards.** `type`/`uses` reach into `rcdzc` (inference,
  resolution); the structural matcher stays dependency-free. Keeping them a **separate command** (not
  a `,(x type-of Int64)` guard) is the deliberate split — shape queries in `query`, semantic facts in
  `type`/`uses`. This realizes `spec/capabilities/tooling-and-lsp.md` §The Compiler Is A Queryable
  Oracle: an agent learns a static fact by *asking*, and the answer equals what a full compile
  determines (it's the same column read; see [[rcdzc-sidecar-request-list-abi]]).
- **`type`** reads the type column (`infer::def_scheme` → `Ty::render_name`). **`uses`** is the
  transpose of the resolution column: every occurrence resolving to the named def/type, in ascending
  order, each mapped to `file:line:col` via the span table this process kept (the cross-process CLI
  could only report raw node ids — this is the in-process win). A name with no references (or none
  such) prints nothing and exits 0.
- **`check`** drives `Query::Diagnostics` — the full fault set (type mismatch, unbound name, duplicate
  def/field, …) read WITHOUT gating on export/emit, so a mid-edit buffer with no `(export …)` still
  reports. Each fault → `file:line:col: severity [CODE]: message`; exits non-zero iff any error-severity
  fault. This is the "as you type" primitive an editor's inline diagnostics (and a CI lint) ride on.
  It also reports WARNINGS (non-error, don't fail the build): a dead computation (CDZ0305) and an
  **unused binding** (CDZ0306) — a `let` binding, `fn`/`def` parameter, or non-exported def that is
  never referenced. Prefix the name with `_` to silence it (`_x`/`_`, as in Rust).
- **`def`** drives `Query::ResolveOf` — the go-to-definition counterpart of `uses`: the reference node
  at the cursor → its defining occurrence (`resolve::resolved_of` → `Ref`/`Lambda`), mapped to
  `file:line:col`. A non-navigable token (a literal, an unbound name) reports no definition.
- **`scope`** drives `Query::ScopeAt` — variable scope tracking: walks the lexical scope from the
  cursor collecting every binding in scope (enclosing `fn`/`def` params, a `let`'s bindings visible at
  the point — sequential, so an initializer sees earlier bindings not itself, a match-arm binder),
  each with its type; INNERMOST first (a shadowed name appears once). What an editor's autocomplete /
  scope panel rides on.
- **`exports`** drives `Query::Exports` — the module's interface: each `(export …)` name paired with
  its def's type (signature), at the def's location. The "what does this module offer" view.
- **Format** is inferred from the file extension (`.cdz`/`.ml`→ml, `.sexp`/`.sexpr`→sexpr), like the
  codemod subcommands.

## Combined query (`cdz query … --where …`) — shape ∧ meaning

The structural `query` finds matches by SHAPE; `--where` filters them by a COMPILER fact. This is the
one query neither the front-end nor the compiler can answer alone — `cdz` runs the structural search,
then types each match's binding node (a batch of `TypeAt`), keeping only matches whose binding relates
to the asked-for type. It needs a single FILE (a compiler query is per unit).

```console
# every call `(foo x)` whose argument x is Int64 — structural (foo ,x) AND semantic type-of(x)=Int64
$ cdz query '(foo ,x)' --where 'type-of(x) = Int64' prog.sexp
prog.sexp:3:30: (foo (: 42 Int64))
  $x = (: 42 Int64)

# the complement, with != ; and --count works too
$ cdz query '(foo ,x)' --where 'type-of(x) != Int64' --count prog.sexp
2
```

- **Predicate grammar** (minimal by design): `type-of(VAR) = TYPE` or `type-of(VAR) != TYPE`. VAR is a
  metavariable the PATTERN binds (`,x` → `x`); TYPE is a rendered type taken verbatim, so a compound
  `(-> Int64 Int64)` / `(List Int64)` works. A malformed predicate is a hard error.
- **Only `cdz` honors `--where`** (it needs the compiler); the pure `cdz-syntax` front-end ignores the
  flag. `--inside`/`--has`/… relational context composes with it. Output is the surviving matches with
  `file:line:col` (or `--count`), same as `query`.
- **Reach:** one FILE for now (a dir sweep would be a per-unit fan-out). Only `type-of` is wired as a
  predicate; other facts (`uses`, effect row) are the natural extension.

## Library API — `cadenza_syntax::query`

Reach for this when scripting a codemod in Rust (a multi-rule pass, a custom report). Everything
operates on an owned homoiconic `Tree` (`Atom | List`), the mirror of the built-in `Ast` sum; convert
at the edges and each node keeps its source `StructId` so a match reports a span.

```rust
use cadenza_syntax::query::{self, Pattern, Template, Tree};

let arena = /* from parser::read_ml / sexpr::read / codec::decode */;
let tree  = Tree::of(&arena);
let pat   = Pattern::compile("(+ ,x 0)")?;
let tmpl  = Template::compile(",x")?;

let hits  = query::search(&pat, &tree, Some(&spans));  // Vec<Match { node, span, bindings }>
let n     = query::count(&pat, &tree);
let out   = query::rewrite(&pat, &tmpl, &tree);          // Rewrite { tree, count }; bottom-up
let sat   = query::rewrite_fixpoint(&pat, &tmpl, &tree, 64);

// relational context (structural only): search filtered by ancestry/containment
use query::Query;
let q     = Query::new().inside(Pattern::compile("(danger ,@_)")?).not_has(Pattern::compile("(ok)")?);
let hits2 = query::search_with(&pat, &q, &tree, Some(&spans));

// multi-rule set + strategy
use query::{Rule, RuleSet, Strategy};
let rules = RuleSet::compile("(rule (+ ,x 0) ,x) (rule (* ,x 1) ,x)")?;   // or RuleSet::new(vec![Rule::new(p, t)])
let out2  = query::rewrite_rules(&rules, &tree, Strategy::BottomUp);       // or Strategy::TopDown
let sat2  = query::rewrite_rules_fixpoint(&rules, &tree, Strategy::BottomUp, 64);

// or the whole driver (what the CLI uses): load a target + project output, with validation
let (target, warnings) = query::driver::load(bytes, Format::Ml)?;   // Target carries a SpanTable (ml + sexpr)
let report  = query::driver::report_matches(&pat, &q, &target);
let outcome = query::driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Ml, 100, false)?;

// FORMATTING-PRESERVING rewrite: splice changed subtrees into the original `src` at their spans
// (layout/comments verbatim); validated, else Err (the CLI falls back to a reprint on Err).
let kept = query::driver::apply_rewrite_preserving(&rules, Strategy::BottomUp, &target, src, Format::Sexpr, false)?;
// low-level: the span-guided minimal-edit engine + the sexpr reader that now records spans
let (arena, spans) = cadenza_syntax::sexpr::read_spanned(src)?;   // s-expr with a SpanTable (was span-free)
let te = query::textedit::rewrite_preserving(src, &old_tree, &new_tree, &span_of, Format::Sexpr); // TextRewrite { output, edits }

// machine-readable output + diff (dependency-free helpers)
let mjson = query::driver::matches_json(&pat, &q, &target, Some("a.ml"));    // [{file?,span,matched,bindings}]
let rjson = query::driver::rewrite_json(Some("a.ml"), outcome.count, &outcome.output);
let before = query::driver::project_target(&target, Format::Ml, 100)?;       // "before" side of a diff
let d = query::diff::unified(&before, &outcome.output, "a/a.ml", "b/a.ml");   // unified LINE diff text

// structural (subtree) diff of two trees
let changes = query::treediff::diff(&tree_a, &tree_b);  // Vec<Change { path: Vec<usize>, kind }>
let human   = query::driver::changes_report(&tree_a, &tree_b);   // "PATH: replace OLD => NEW" …
let cjson   = query::driver::changes_json(&tree_a, &tree_b);     // [{path, kind, old?, new?}]

// structural lint (pattern + message + severity; error-severity fails a run)
use query::lint::{self, LintSet};
let set   = LintSet::compile("(lint (deprecated ,@_) \"avoid\" error)")?;
let diags = lint::run(&set, &tree, Some(&spans));   // Vec<Diagnostic { message, severity, span, matched }>
let gate  = lint::has_error(&diags);                // true → CI should fail

// content hash + clone detection
let h       = query::hash::hash_tree(&tree);        // u64 Merkle hash; == iff tree_eq (fast eq filter)
let classes = query::clones::find_clones(&tree, 3, Some(&spans));   // Vec<CloneClass { exemplar, size, sites }>
// cross-file: query::clones::find_clones_multi(&[Source { tree, spans, file }], min_size)

// anti-unification (inverse of matching) + near-clone detection
let g = query::antiunify::anti_unify(&[&a, &b]);              // Generalization { pattern, holes }
let p = query::antiunify::render_pattern(&g.pattern);        // ",mK"-sugar pattern → feed to Pattern::compile
let near = query::clones::find_near_clones_one(&tree, 3, Some(&spans));  // Vec<NearCloneClass { pattern, size, hole_count, sites }>
```

Multi-file / `--write` / directory-walk plumbing lives in the CLI (the bin), not the library — the
library stays pure. Reach for the driver + `std::fs` if scripting a batch run in Rust.

`search` is top-down (nested matches included). `rewrite` is **bottom-up** — a node is matched against
its already-rewritten children, so a rule that exposes a new match collapses in one pass
(`(+ ,x 0) → ,x` fully reduces `(+ (+ x 0) 0)`).

## What is NOT here (and why)

- **Type/binding guards INSIDE a pattern** (`,(x type-of Int64)`, `,(x refs …)`, typed metavars) — the
  structural matcher stays dependency-free, so a *pattern* never resolves a name or infers a type. The
  type FILTER is available a different way — as the `--where` clause on `cdz query`
  ([§Combined query](#combined-query-cdz-query----where--shape--meaning)): the pattern is pure shape, the
  `--where` adds the semantic predicate, kept as separate layers rather than a guard that reaches into
  the checker. (A `refs`-based `--where` and typed metavars are the natural extension; only `type-of` is
  wired today.)
- **Scope-aware rename** (rename a binding + every reference, respecting shadowing) — `cdz uses` gives
  you the reference set; a validated multi-site rename built on it is not yet a single command.
- **Addressed edits** (`insert`/`replace`/`delete`/`move` by node path/content-id) — the
  `options/structural-interface/content-addressed-nodes.md` layer, above these primitives.
- **Type-checking a rewrite result** — the codemod validates *well-formedness* (re-parse + round-trip),
  not types. A full typed transaction (re-check the edited tree with `rcdzc`) is reachable now that the
  binary links both libraries, but is not yet wired into `rewrite`.

## Gotchas

- **Patterns are the s-expr surface, always** — write `(+ ,x 0)`, not `x + 0`. (The subject can be any
  surface via `--from`; the pattern/template text is s-expr.)
- **`rewrite` writes the program to stdout, the count to stderr** — capture stdout to get a clean
  result; don't grep stdout for "rewrote".
- **A repeated metavar is a constraint, not just a name** (`,x … ,x` demands equal subtrees). Use a
  fresh name or `,_` when you don't want that.
- **`--fixpoint` is bounded** (64 passes) precisely because a rule whose output re-matches its input
  (e.g. `,x → (w ,x)`) would otherwise loop; a bounded, non-fixed result is returned, not an error.
- **`--write`/`--diff` preserve layout by default** — the output keeps the source's exact formatting
  (only changed subtrees are respliced). If you WANT a canonical reflow (normalize a file), pass
  `--reprint`. A cross-surface `--to` (e.g. `.sexp` in, `--to ml`) can't splice into the original text,
  so it always reprints.
- **Two `,@` splices must have a fixed element between them** — `(f ,@a X ,@b)` is fine (delete/anchor
  around `X`), `(f ,@a ,@b)` is rejected (no anchor to divide the run).

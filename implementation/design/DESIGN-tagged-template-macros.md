# Design — tagged-template macros (embedded DSLs), with JSX as the flagship library

**Author:** design pass (design-jsx-syntax agent, interactive with the operator). **Audience:** the
`vertical` picking up embedded-DSL syntax, + future me.
**Status:** proposal / handoff — **nothing landed**. Written in the house style of
`DESIGN-collection-and-binary-patterns-rcdzc.md` and `DESIGN-binding-patterns-rcdzc.md`: it states the
target, the accept/decline boundary, the shared mechanism, the seams with file anchors, the gate, and
the subtleties.

The through-line the operator asked for: **don't bake JSX into the grammar. Ship a generic
tagged-template mechanism** — a sigil captures raw text plus `{expr}` interpolation holes and hands them
to a *compile-time function* that returns an `Ast`, spliced into the program and type-checked like
hand-written code. **JSX is then a library, not a language feature.** Users add their own embedded DSLs
(SQL, regex, GraphQL, CSS, a state-machine notation) the same way — "the user could easily add their
own" — with zero compiler changes, because the DSL parser is ordinary Cadenza code, not a reader
extension.

---

## 1. TL;DR — the win, the insight, the pick

**The win.** Cadenza has quote / quasiquote / `eval` / a built-in `Ast` sum, and a one-tier compile-time
evaluator (`spec/capabilities/metaprogramming.md` §*Compile-Time Evaluation Is One Tier*). What it does
**not** have is a way to write a value in a *foreign concrete syntax* — HTML/JSX, SQL, a regex — and have
it become a Cadenza value at compile time. Today the only embedded surface is the s-expr/ML grammar
itself; a `<div class={c}>{body}</div>` is unspellable. Yet an agent-authored program that emits UI, a
query, or a byte format wants exactly this: write the domain notation, interpolate live values, get a
typed value back.

**The insight (the one that unifies JSX with every other embedded DSL).** A tagged template is
**quasiquote generalized from one hard-wired grammar (Cadenza's own) to an arbitrary grammar supplied by
a library.** Quasiquote captures a *Cadenza* tree with `,`-holes and hands you an `Ast`; a tagged
template captures a *string* with `{…}`-holes and hands a **library function** the raw chunks + the holes
(already lowered to `Ast`), and that function returns an `Ast`. The language's job is only to (a) lex the
literal into chunks + holes, (b) resolve the tag name to a binding, and (c) evaluate that binding at
compile time on the chunks/holes and splice its `Ast` result. Everything domain-specific — the HTML
parser, the SQL parser — is **ordinary Cadenza code the user writes and imports**, evaluated on the
one-tier compile-time evaluator that macros, generics, and const-folding already share.

**The spec constraint that shapes it (read this or you'll design something illegal).**
`metaprogramming.md` §*A Macro Is Dispatched By Binding, Not By Spelling* is explicit: *"The reader MUST
NOT be extensible by a program: syntax MUST grow at the abstract-syntax-tree level through macros rather
than through reader macros, so that the text-to-canonical-representation reader stays outside the
compiler's trusted path."* So we **cannot** let a program extend the parser. The resolution that gives the
operator's "users add their own" while staying legal:

> The reader has **ONE fixed, non-extensible rule**: `tag"…text…{expr}…"` lexes to a **canonical AST
> node** carrying the tag name, the literal string chunks (as `Ast.Str` leaves), and the holes (as
> ordinary `Ast` sub-expressions). The reader never runs user code and never learns a new grammar. The
> *interpretation* of the string is a **binding-dispatched compile-time macro** — an ordinary function —
> operating on that canonical data. The DSL grows at the AST level (a function over `List String` +
> `List Ast`), exactly as the spec demands; the reader stays fixed and outside the trusted path.

**The pick.** Add **one** reader form and **one** expansion rule:
1. **Reader (fixed):** lex `ident "…"` (no space between the identifier and the string, mirroring the
   existing `b"…"` byte-string and `#"…"` symbol lexing) into a `TaggedTemplate { tag, chunks, holes }`
   canonical node. `{expr}` inside the string opens a hole whose contents are lexed/parsed as ordinary
   Cadenza and lowered to `Ast`; `{{`/`}}` (or `\{`) escapes a literal brace.
2. **Expansion (binding-dispatched, compile-time):** during the macro-expansion phase (which already
   *precedes* type-checking, §*Expansion Runs In Phases To A Fixpoint*), resolve `tag` to a binding. It
   must be a function `tag : List String -> List Ast -> Ast`. Evaluate it on the one-tier compile-time
   evaluator with the chunks and holes; splice its returned `Ast` in place of the template node; re-run
   expansion to fixpoint; then type-check the spliced tree normally.

JSX is then a library `jsx : List String -> List Ast -> Ast` whose body parses the HTML-ish chunks and
emits `(Tag (record …attrs) (list …children))` calls — the **plain-function / `h()` desugar** the
operator chose (§4.3). No blessed `View` type; any view library works. The same mechanism gives `sql"…"`,
`re"…"`, `css"…"` for free.

---

## 2. What already exists (read before touching code)

Confirmed against the tree:

- **`Ast` is a real built-in sum**, deconstructible by match, with `Int`/`Name`/`Bool`/`Str`/`List`
  leaves landed (`spec/semantics/12-metaprogramming.sexp`; `Ast.Str` at `:306`–`:372`, the leaf the
  metaprogramming vertical just realized). A macro that returns an `Ast` is returning an ordinary value.
- **Quote / quasiquote / unquote / `eval`** build and run `Ast`
  (`12-metaprogramming.sexp:7`,`:94`,`:487`; `eval (quote (+ 1 2)) => 3` at `:41`). The tagged-template
  macro's *output* is the same `Ast` these produce, so it inherits reification, encode/decode round-trip
  (`:438`), and type-checking of the spliced expression.
- **One-tier compile-time evaluation** is a normative requirement, not a new subsystem
  (`metaprogramming.md` §*Compile-Time Evaluation Is One Tier*: "Macro expansion, generic reduction,
  monomorphization, and constant folding MUST be the same compile-time evaluation mechanism"). The tagged
  macro runs on it. rcdzc's fold/const-eval seam lives in `implementation/compiler-ml/src/cfold.cdz` +
  `fold.cdz` (the ML compiler) and the rcdzc guide's fold path.
- **Dispatch by binding, not spelling** (`metaprogramming.md` §*A Macro Is Dispatched By Binding…*) — so
  there is **no `@macro` attribute**; a tag is a macro *because it sits in tag position and resolves to a
  suitable function*. This is precisely the operator's "we don't need macro attrs — it should just be
  able to const-eval a function."
- **Expansion precedes type-checking and runs to a fixpoint** (§*Expansion Runs In Phases To A Fixpoint*)
  and **is reproducible** (§*Expansion Is Reproducible*) and **pure** (§*Compile-Time Evaluation Is
  Pure*). The tagged-template expansion slots into this existing phase; it adds no new phase.
- **The lexer already carries sigil-glued-to-quote precedent**: `b"…"` byte strings and `#"…"` symbols
  are lexed as an ident-start char glued to a string (`implementation/seed/crates/cadenza-syntax/src/lexer.rs`
  around `:74`–`:90`,`:175`), and the quasiquote sigils (`` ` ``, `,`, `,@`) are already there
  (`lexer.rs` header + `:135`). A tagged template is the **general case** of the `b"…"` rule: any ident
  glued to a string, with holes.
- **`<` is the comparison operator today** (`token.rs:36` `Kind::Lt`, `lexer.rs:164`). The
  tagged-template design **does not touch `<`** — angle brackets live *inside the string*, parsed by the
  JSX library, never by Cadenza's grammar. This sidesteps the JSX-in-grammar ambiguity entirely and is a
  direct dividend of the operator's sigil approach.
- **No `View`/`Node`/`element` type exists** anywhere in the prelude or corpus (greenfield). The chosen
  desugar (plain function call, §4.3) means we do **not** add one — the view model is a library's choice.

---

## 3. The surface — what the operator writes

```
// A tag glued to a string is a tagged template. `jsx` is an ordinary imported function.
let page =
  jsx"<Panel title={heading}>
        <Row>{body}</Row>
        <Row class=\"footer\">{footer}</Row>
      </Panel>"
in render page
```

The reader lexes this into a canonical `TaggedTemplate` node:

```
TaggedTemplate {
  tag    = "jsx"
  chunks = [ "<Panel title=", ">\n        <Row>", "</Row>\n        <Row class=\"footer\">",
             "</Row>\n      </Panel>" ]         // the literal text between holes (n+1 chunks)
  holes  = [ Ast(heading), Ast(body), Ast(footer) ]   // each {…} lowered to an Ast (n holes)
}
```

Expansion resolves `jsx` to its binding and evaluates `jsx chunks holes` at compile time. A `jsx`
library that chose the **plain-function desugar** returns the `Ast` for:

```
Panel (record (title heading))
      (list (Row (record) (list body))
            (Row (record (class "footer")) (list footer)))
```

— ordinary constructor/function calls. `Panel`, `Row` are whatever the user has in scope (a function, a
record constructor, a component). The language blesses **nothing**: no `View` type, no `render`, no
attribute model. A different `jsx` library could emit a blessed `View.Element` tree instead — that is the
library's decision, not the language's.

**Interpolation invariant:** `chunks.len() == holes.len() + 1` always (JS tagged-template shape). A hole
is arbitrary Cadenza already lowered to `Ast`, so `title={user.name}` interpolates a computed value; the
macro decides where holes may legally appear (an attribute value, a child) and rejects the rest with a
compile-time error it raises itself (§8).

---

## 4. The mechanism — three obligations on the language, everything else is library

### 4.1 Lex the literal (reader, fixed, non-extensible)
`ident "…"` with **no intervening whitespace** is a tagged template (mirrors `b"…"`/`#"…"`). The string
body is scanned for holes:
- `{` opens a hole; its contents up to the matching `}` are lexed+parsed as an ordinary Cadenza
  expression (balanced-brace aware, so `{ record {a 1} }` nests). The hole expression is lowered to `Ast`
  the same way a quasiquote `,expr` is.
- `{{` and `}}` are literal braces in a chunk (or `\{`/`\}` — pick one, §9); `\"`, `\n`, etc. keep the
  existing string-escape semantics so the body is a normal string.
- The result is the canonical `TaggedTemplate { tag, chunks: [Str…], holes: [Ast…] }` node. **The reader
  runs no user code and learns no grammar** — it only splits a string on holes. This is the whole of the
  reader's involvement, satisfying "the reader stays outside the trusted path."

**Why glued-ident, not `<…>` in the grammar.** The operator's key move: angle brackets are *inside the
string*, so Cadenza's grammar is untouched and `a < b` stays comparison with zero ambiguity (§2). The
generality (any DSL, not just HTML) also falls out — `sql"SELECT …"`, `re"[a-z]+"` use the identical
reader rule.

### 4.2 Resolve the tag to a binding (expansion, by binding not spelling)
In the expansion phase, a `TaggedTemplate`'s `tag` is resolved in the ordinary name environment. It MUST
resolve to a value of type `List String -> List Ast -> Ast` (a *template macro*). If it doesn't resolve,
or resolves to a non-function / wrong-arity / wrong-type binding, that is a compile-time error at the
template site (§8) — **not** a reader error, because whether a tag is a template macro is a *binding*
fact (§*A Macro Is Dispatched By Binding, Not By Spelling*). This is what lets a user "add their own": you
`def jsx …` (or import it) and the tag works; there is no registration step and no attribute.

### 4.3 Evaluate at compile time and splice (one-tier eval, then fixpoint)
Evaluate `tag chunks holes` on the **existing** one-tier compile-time evaluator (the same path that folds
`(eval (quote …))`, reduces generics, and const-folds — `metaprogramming.md` §*One Tier*; rcdzc
`cfold.cdz`/`fold.cdz`). The evaluator is pure (§*Compile-Time Evaluation Is Pure*), so a template macro
cannot do I/O — good: DSL expansion is deterministic and reproducible (§*Expansion Is Reproducible*).
Splice the returned `Ast` in the template node's position, then **re-run expansion to a fixpoint** (the
returned `Ast` may itself contain tagged templates or macro uses — §*Expansion Runs In Phases To A
Fixpoint*), then type-check the fully-expanded tree. Because expansion precedes type-checking, a
`jsx` that emits an ill-typed call is caught downstream at the spliced call — and a *typed-quote* macro
(§*A Typed Quote Carries The Type…*) can be caught **at the macro** if we later want per-macro typing;
Increment 1 does not require typed quotes.

### 4.4 The desugar target is the library's choice (the operator's pick: plain call)
The flagship `jsx` emits **plain function/constructor calls** (`<Foo a=1>k</Foo>` → `(Foo (record (a 1))
(list k))`): capitalized-or-not, the tag name is just a name resolved at the *spliced* site. Lowercase
`<div>` can map to a call to an in-scope `div` function, or the library can special-case bare-lowercase
tags to a string-tagged builtin — again, the library decides. The language adds no `View` type, no
element/attr/child model, no renderer. This keeps Cadenza neutral (any view lib works) and tiny, and it
means the JSX vertical is *mostly a Cadenza-library-authoring task*, not a compiler task.

---

## 5. Pass-by-pass — where each obligation plugs in

The headline: **the compiler change is small and generic (a reader form + an expansion rule); the JSX
richness is a library.**

### 5.1 `cadenza-syntax` (the reader/printer — the fixed rule)
- **Lexer** (`lexer.rs`): extend the ident-glued-to-string precedent (`b"…"`/`#"…"`, ~`:74`–`:90`,`:175`)
  to *any* ident glued to a `"` → begin a tagged-template lex. Scan the body into chunks + hole spans
  (balanced-brace aware). A hole's inner text is handed back to the normal expression parser.
- **Token/AST** (`token.rs`, `ast.rs`): a `TaggedTemplate` node = `{ tag: Name, chunks: [StrLit],
  holes: [Expr] }`. It is an ordinary expression node (appears anywhere an expression can) — this is what
  makes `<…>` "embedded in ML" per the operator's surface choice.
- **Printer** (`printer.rs`): render `TaggedTemplate` back to `tag"…{…}…"`, escaping braces, so the
  surface round-trips through the arena (the constitution's round-trip discipline — every surface
  projects losslessly). **Watch the printer trap:** a form that round-trips to garbage is a second
  spelling and must be rejected/migrated, not taught to the printer
  ([[garbage-render-means-not-canonical-fix-the-source]]).
- **Codec** (`codec.rs`): the node is ordinary arena data (a tag leaf + a chunk list + a hole subtree
  list); no special encoding, and `decode` stays total ([[codec-decode-must-reject-non-tree-arenas]]).
- This is `v-syntax` territory (the ML front-end crate) — coordinate; the reader form is theirs to land.

### 5.2 The expander (the binding-dispatched rule)
- The macro-expansion phase gains a case: on a `TaggedTemplate`, resolve `tag`; require
  `List String -> List Ast -> Ast`; evaluate on the one-tier evaluator; splice; fixpoint. In rcdzc this
  sits where compile-time evaluation already runs (`cfold.cdz`/`fold.cdz` + the guide's fold path); in the
  Cadenza-ML compiler (`implementation/compiler-ml/`) it is the analogous expansion pass. **No new IR
  rung** — the spliced result is ordinary `Ast`; a `TaggedTemplate` node exists only *before* expansion
  and never survives into type-checking (like a macro use).
- **Hygiene** (§*Macros Are Hygienic*): names a template macro *introduces* (e.g. a helper the JSX
  expansion binds) must not capture at the use site and vice-versa, resolved by scope-set. Increment 1
  can restrict template macros to emit only fully-qualified / hole-and-chunk-derived names (no fresh
  binders) to sidestep hygiene, and lift that restriction when the general macro hygiene work lands.

### 5.3 The JSX library (Cadenza code, not compiler code)
- `jsx : List String -> List Ast -> Ast` — a hand-written recursive-descent HTML-ish parser *in
  Cadenza*, weaving `holes` in at the positions its parse reaches. It emits `(Tag (record attrs)
  (list children))` `Ast`. This is the **stress test** the operator's active workstream wants
  ([[port-compiler-to-cadenza-ml]]): a real, non-trivial Cadenza program (a parser!) that must compile
  and run at compile time. Report/fix language gaps it hits; don't work around them.
- Lives under a library path (e.g. `implementation/compiler-ml/` examples or a `std`-ish location the
  vertical picks), imported with ML import syntax (NOT the s-expr `(export (. T *))` which renders
  garbage — [[garbage-render-means-not-canonical-fix-the-source]]).

---

## 6. Worked example — the `jsx` library shape (illustrative)

```
// jsx : List String -> List Ast -> Ast   (ordinary Cadenza; runs at compile time)
def (jsx chunks holes)
  let toks = lex-markup chunks holes in    // interleave literal text + hole markers
  let tree = parse-element toks in         // recursive descent over <tag attr=.. >..</tag>
  emit tree                                // Element{tag,attrs,kids} -> Ast of (tag (record ..) (list ..))

// emit maps a parsed element to plain-call Ast — the operator's desugar:
def (emit el)
  match el
    (Element tag attrs kids ->
       Ast.app (Ast.name tag)
               [ Ast.record (map emit-attr attrs)
               , Ast.list   (map emit-child kids) ])
    (Hole ast -> ast)                      // a {expr} child splices its Ast straight through
    (Text  s  -> Ast.app (Ast.name "text") [Ast.str s])
```

The point is not this exact code — it's that **all of it is Cadenza**, so the DSL is user-owned and a new
DSL (`sql`, `re`, `css`) is a new function with the same signature, no compiler change.

---

## 7. Why this is the right generalization (and what it is NOT)

- **It is quasiquote with a pluggable grammar.** Quasiquote = fixed Cadenza grammar + `,` holes → `Ast`.
  Tagged template = library grammar + `{}` holes → `Ast`. Same output, same evaluator, same phase.
- **It is NOT a reader macro.** The reader learns no grammar and runs no user code; it only splits a
  string on holes into canonical data. The grammar of the *DSL* lives in a compile-time function over
  that data. This is the exact line `metaprogramming.md` draws to keep the reader untrusted.
- **It is NOT a new blessed type.** No `View`. The desugar target is the library's `Ast` output; the
  language stays neutral.
- **It subsumes JSX as one instance.** The agent's name is `design-jsx-syntax`, but JSX is the *flagship
  demo* of a mechanism that is strictly more valuable: any embedded DSL, added by any user, with no
  compiler change — which is what the operator actually asked for.

---

## 8. Diagnostics

All raised at the **template site** during expansion (before type-check), so spans point at the source:
- **Tag doesn't resolve / wrong type** — `tag` is unbound, or not `List String -> List Ast -> Ast`:
  a coded error ("a tagged-template tag must be a compile-time function `List String -> List Ast ->
  Ast`"). New code (reserve one; mirror the CDZ02xx scheme).
- **Malformed hole** — an unbalanced `{`/`}`, an empty `{}`, a hole whose inner text doesn't parse as a
  Cadenza expression: a reader/lex error with the brace span.
- **Macro-raised errors** — the JSX (or any) library detects a bad `<div` / mismatched close tag / a hole
  in an illegal position and **raises its own compile-time error** (the macro is ordinary code; it can
  `trap`/emit a diagnostic `Ast`). The language surfaces it at the template site. This is the payoff of
  "the DSL is a function": *its* error messages are as good as its author makes them, and the compiler
  needs no HTML knowledge.
- **Non-termination / impurity** — the evaluator is pure and metered by the compile-time tier; a macro
  that loops is bounded the same way generic reduction is (no new policy).

## 9. Open decisions (with a chosen default)

1. **Hole delimiter.** `{expr}` (JS/JSX-familiar, but collides with record/block braces inside the DSL
   text) vs `${expr}` (JS template-literal, unambiguous) vs reuse quasiquote `,expr`.
   **Default: `{expr}` with `{{`/`}}` escapes** (matches the operator's JSX sketch); revisit if a DSL's
   own syntax makes `{}` painful (`${}` is the fallback).
2. **Lowercase-tag convention.** Does the *language* care about `<div>` vs `<Foo>`? **Default: no** — the
   language passes the raw tag name through; the *library* decides (bare-lowercase → `(div …)` call or a
   string-tagged builtin). Keeps the language neutral.
3. **Multi-argument macro signature.** `List String -> List Ast -> Ast` (JS-tagged shape, chosen) vs a
   single `TemplateParts` record vs raw single `String` (no structured holes). **Default: the two-list
   signature** — the operator explicitly chose "raw strings + spliced Ast holes."
4. **Typed quotes for macros** (§*A Typed Quote Carries The Type…*): catch an ill-typed macro *at the
   macro* rather than at the spliced site. **Default: defer to a later increment** — Increment 1 checks
   the spliced tree downstream, which is sound if noisier.
5. **Where the JSX library lives** (std-ish path vs example). **Default: the vertical picks**, guided by
   the compiler-in-Cadenza layout ([[port-compiler-to-cadenza-ml]]).
6. **Sigil vs glued-ident** to enter template mode. **Default: glued-ident `tag"…"`** (reuses the
   `b"…"`/`#"…"` lex, no new sigil); a `#tag"…"` variant is a trivial alt if glued-ident proves
   ambiguous with some existing ident-quote form.

## 10. Increment plan (leverage- and dependency-ordered)

0. **Spec-first.** There is no capability sentence for tagged templates. Add a normative §*A Tagged
   Template Is A Binding-Dispatched Compile-Time Macro Over Literal Chunks And Holes* to
   `spec/capabilities/metaprogramming.md`, explicitly deriving it from §*A Macro Is Dispatched By Binding*
   and §*The Reader MUST NOT Be Extensible* (it is the reconciliation of the two). Add corpus cases to a
   new `spec/semantics/NN-tagged-templates.sexp` (a trivial `id"…"` echo macro, a hole-splicing macro, a
   malformed-tag reject, a fixpoint case where a macro emits another template). Register the capability
   gate in `options/realized-capability-set/`.
1. **The reader form (fixed rule).** Lex `tag"…{…}…"` → `TaggedTemplate { tag, chunks, holes }`; print it
   back; codec/round-trip; no-panic fuzz. `v-syntax` owns this. Gate: the round-trip corpus + the syntax
   crate's total-decode/no-panic tests. *No expansion yet — the node just parses and prints.*
2. **The expansion rule (binding dispatch + one-tier eval + fixpoint).** Resolve the tag, require the
   `List String -> List Ast -> Ast` type, evaluate on the compile-time tier, splice, fixpoint, then
   type-check. Prove it with the **identity macro** `def (id chunks holes) …` that reassembles a plain
   string, and a **hole-splicing macro** that interpolates one `{x}` — smallest surface, no JSX yet. Gate:
   the new corpus cases pass end-to-end (a value executes on wasmtime).
3. **Diagnostics (§8).** Unbound/wrong-type tag; malformed hole; macro-raised error surfaced at the site.
   Reject-tests in the corpus.
4. **The JSX library (Cadenza code).** Write `jsx : List String -> List Ast -> Ast` — the HTML-ish parser
   in Cadenza emitting plain-call `Ast`. This is the flagship demo AND a real compiler-in-Cadenza stress
   test ([[port-compiler-to-cadenza-ml]]): report/fix every language gap the parser hits. Gate: a
   `jsx"…"` example compiles and renders (a small `render` in the example) to the expected value.
5. **A second DSL to prove generality** (e.g. `re"…"` → a compiled matcher `Ast`, or `sql"…"` → a query
   value). Confirms "users add their own" with zero compiler change beyond Increments 1–3. Optional but
   high-signal.

## 11. Subtleties an implementer must get right

- **The reader must not learn the DSL grammar.** It splits a string on holes and hands back canonical
  data — nothing more. Any temptation to special-case `<`/HTML in the lexer is the illegal reader-macro
  path (§*The Reader MUST NOT Be Extensible*). All grammar lives in the compile-time function.
- **`chunks.len() == holes.len() + 1`, always.** A template with no holes is one chunk + zero holes; a
  leading/trailing hole yields an empty edge chunk. The macro relies on this invariant to interleave.
- **Expansion precedes type-checking and runs to a fixpoint.** A spliced `Ast` may contain more templates
  or macro uses; expand until stable *before* typing (§*Expansion Runs In Phases To A Fixpoint*). Don't
  type a half-expanded tree.
- **Compile-time eval is pure and reproducible.** A template macro cannot do I/O and must be
  deterministic (§*Compile-Time Evaluation Is Pure* / *Expansion Is Reproducible*) — the same program
  expands identically on every conforming compiler. No clock/random in a macro.
- **Hygiene** (§*Macros Are Hygienic*). If a macro introduces a binder, resolve by scope-set, not
  spelling. Increment 1 sidesteps by emitting only hole/chunk-derived and use-site-resolved names; lift
  when general macro hygiene lands.
- **Dispatch is by binding, never by spelling** (§*A Macro Is Dispatched By Binding*). `jsx` is a macro
  because it resolves to a `List String -> List Ast -> Ast` in tag position — not because it's spelled
  "jsx". Two modules can bind `jsx` to different parsers; each use resolves in its own scope.
- **`<` stays comparison.** Angle brackets live inside the template string; the base grammar is untouched
  (the whole reason for the sigil/glued-ident approach). Do not add an angle-bracket expression form.
- **Round-trip or reject.** The printer must render `TaggedTemplate` back to a form that re-reads to the
  same arena; a garbage render means the node isn't canonical — fix the source, don't teach the printer a
  second spelling ([[garbage-render-means-not-canonical-fix-the-source]]).

## 12. Ladder placement & related

Sits squarely in the **metaprogramming** vertical's territory (it is a macro form) with a **syntax**
vertical dependency (the reader/printer form, Increment 1). It is orthogonal to the compiler's type/effect
work: it adds one reader node and one expansion rule, both feeding the *existing* one-tier compile-time
evaluator; the spliced output is ordinary `Ast` that the normal pipeline types and lowers. The JSX
library (Increment 4) doubles as a compiler-in-Cadenza stress test ([[port-compiler-to-cadenza-ml]]).

Related: `spec/capabilities/metaprogramming.md` (§*A Macro Is Dispatched By Binding, Not By Spelling* —
the reader-non-extensibility line this design reconciles; §*Compile-Time Evaluation Is One Tier*;
§*Expansion Runs In Phases To A Fixpoint*; §*Macros Are Hygienic*); `spec/semantics/12-metaprogramming.sexp`
(quote/quasiquote/eval/`Ast` — the machinery this layers on; `Ast.Str` at `:306`);
`implementation/seed/crates/cadenza-syntax/src/lexer.rs` (the `b"…"`/`#"…"` glued-ident-to-string
precedent the tagged-template lex extends); `implementation/compiler-ml/src/{cfold,fold}.cdz` (the
compile-time evaluation seam expansion runs on); `DESIGN-collection-and-binary-patterns-rcdzc.md` (the
sibling "one mechanism, incremental categories" design this mirrors).
```

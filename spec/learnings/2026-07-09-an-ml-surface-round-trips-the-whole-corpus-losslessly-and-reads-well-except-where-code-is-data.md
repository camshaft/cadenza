# An ML surface round-trips the whole corpus losslessly and reads well — except exactly where code is data

*2026-07-09*

**What happened.** A spike (`implementation/seed/crates/ml-spike/`, outside `spec/` and the gate) built an
ML-flavored **printer** (`Ast → ML text`) and **reader** (`ML text → Ast`) against the *real* `rcdzc`
`Ast` and s-expression reader, then validated `read_ml(print_ml(read_sexpr(case))) == read_sexpr(case)`
over every `(input …)` form in the executable-semantics corpus. Result: **925 / 925 forms round-trip to
the byte-identical AST**, across all twenty `spec/semantics/*.sexp` files — literals, binding, compound
types, the numeric model, functions, bytes, modules, metaprogramming, strings, effects and handlers,
rows and open sums, binary matching, symbols, units, sets, and structural editing. This is the empirical
answer to the question [[2026-07-09-the-paren-problem-is-a-decoding-problem-and-the-ai-native-win-is-semantic-context-at-the-edit-point]]
left open: *can an ML surface losslessly represent every AST the corpus exercises?* Yes, with no genuine
representability wall.

Three findings from building it are the durable content:

1. **The lossless floor is a generic call-form bijection; ergonomics layer on top.** The whole surface
   rests on `(Name a b c) ⟺ Name(a, b, c)` — any name-headed list round-trips as an ML call *for free*,
   so effect / op / host / handle / record / map / module / def / constructor forms needed **zero**
   bespoke code. Only the forms that *read better* with dedicated surface got it: infix operators with a
   precedence table (Pratt-parsed, printed with **minimal** parentheses — `(+ 1 (* 2 3))` prints
   `1 + 2 * 3` and reads back correctly, not by over-parenthesizing), member access `a.b`, `let`, `if`,
   `fn`, and `match`. The lesson: a projection does not need a grammar rule per construct — it needs one
   lossless default (the call form) plus ergonomic surface *only* where a construct is common enough to
   earn it.

2. **The one lossless primitive that made it work is a re-lex-identity name escape.** The first full run
   was 729/925; nearly every failure traced to a symbolic atom printed bare and then re-lexed as
   something else — a sum-type declaration's `|`, a type arrow `->`, a `:` annotation head, a reserved
   word used as a member key (`.in`). One rule (`emit_name`) closed almost all of them: **a name prints
   bare only if it re-lexes to exactly itself, otherwise it is backtick-quoted** (`` `|` ``, `` `->` ``,
   `` `.in` ``). This is the printer-side dual of the reader being total: losslessness through a surface
   is a property you *engineer* with one escape hatch, not one you get by matching each construct.

3. **`match` round-trips through the generic call form, but the corpus matches structurally — a
   surface convenience surfaced a semantic question the language had not settled.** The corpus's real
   match arms are pattern-position *structure*: constructor patterns (`(Some n)`,
   `(Node.NAdd (tuple a b))`), quote patterns (`(Ast.List elems)`), literals (`(0 "zero")`), and
   `_`/`else` — and value conditions go through `if`/`else`, never a match arm (the corpus's own
   sign-classification is `(if (< n 0) (Sign.Neg unit) (if (= n 0) …))`, with `match` used only to
   destructure the resulting `Sign`). The spike printed each arm through the ordinary expression
   printer, which round-trips *any* arm head — including a boolean-predicate head — losslessly; that is
   what initially made "no separate pattern grammar" look like a free win. It was not: a `match` whose
   arm head can be an arbitrary boolean predicate is a `cond` in disguise, and it silently demotes
   type-directed exhaustiveness (did you cover the scrutinee's variants?) to "is there an `else`?" The
   round-trip does not care; the *language* does. The resolved design (matching the corpus, not the
   spike's permissive surface) is **structural patterns only** — a match arm head is a constructor,
   literal, binding, or wildcard, checked against the scrutinee's type for exhaustiveness — with a
   `pattern if guard` refinement as a *separate, optional, pure* concern that does not count toward
   coverage. So a real ML surface needs a genuine pattern grammar (a distinct grammatical category),
   which reads *better* than the generic call form (`Some(v) => v` rather than an application), not the
   "one node" collapse. The lesson: a lossless round-trip proves a surface can *represent* a tree, not
   that the tree is *well-formed under the intended semantics* — a permissive printer will faithfully
   round-trip a construct the language means to reject, so surface validation must not be mistaken for
   semantic validation.

**Why.** The result is not that ML is a *neutral* surface — it is that ML is a *true-friend* surface for
Cadenza's semantics (strict, pure, immutable, expression-oriented, HM-typed, ADTs with exhaustive
match), so the surface that reads idiomatically is the one that also primes the model toward the
behavior the language actually has. But the spike also found the sharp edge, and it is precisely where
the surface stops being ordinary code and starts being *data about code*:

- **Nested / quoted-template metaprogramming still favors s-expr.** In-ML quasiquote sigils
  (`` `{ ,x + 10 } ``, `,{ 1 + 1 }`, `,@xs`) close the gap for the shallow, common case — splicing a
  value into a template reads *as well as or better than* the s-expr, because the template body is real
  infix ML. But a compound unquote needs brace noise (`,{ 1 + 1 }` vs s-expr's brace-free `,(+ 1 1)`),
  and a quote-of-quasiquote is genuinely worse: `` quote(`{ `+`(,x) }) `` overloads the backtick as both
  quasiquote *and* name-escape and forces the inert operator into call form, where the s-expr
  `(quote `` `(+ ,x) ``)` is unambiguous and shorter.
- **Structural editing and symbolic-atoms-as-data pay the escape tax.** When an agent manipulates the
  tree *as* data (the `20-structural-editing` cases) or a declaration carries an operator glyph as a
  value (a sum-type declaration's `` `|` ``), the s-expr's uniform `(head child…)` shape is the natural
  target and ML's construct-specific surfaces are a projection to decode first.

This is the same tension the research pass predicted from first principles, now demonstrated in code: the
one domain homoiconicity exists to serve — code as data — is the one domain a construct-shaped ML surface
serves worse than the uniform s-expr. It is not a defect to fix; it is a property of the two surfaces,
and it says the two are *complementary projections*, not competitors.

One authoring cost is worth recording because it is a real gotcha, not a bug: allowing kebab-case
identifiers (`byte-at`, `cbor-info`, pervasive in the corpus) forces the rule **a `-` is part of an
identifier iff a word character is on both sides, otherwise it is subtraction** — so a human must write
`x - 1`, never `x-1`, for subtraction. The rule held across all 925 cases with no collision, but it is a
whitespace-sensitive lexer rule inside an otherwise whitespace-insensitive grammar, and it is the price
of the corpus's naming convention.

**The requirement it drove.** No normative requirement — this is a `code-shape` finding, and `code-shape`
is a declared *choice*, not a requirement (`options/code-shape/homoiconic-decoupled-display.md`), so it
touches no frozen contract and does not gate. What it drives is a **strengthening of that choice's
evidence**: the decoupled-display bet was argued a priori (one binary-AST store, many lossless surface
projections); the spike shows empirically that an **ML display is a viable *primary* display, not merely
a possible one** — 925/925 lossless including effects, handlers, sum declarations, binary matching, units,
and metaprogramming — and that the ergonomic surface reads as idiomatic strict-pure ML for the constructs
agents touch most. It also localizes, in code, exactly which constructs argue for keeping the
s-expression syntax as a first-class **co-surface** (metaprogramming, structural editing, and any
position where an operator glyph is data rather than an operator). The fold is into
`homoiconic-decoupled-display.md`'s choice text as a recorded finding, naming the spike here in the
learning per the prior-art exception and keeping proper names out of the choice text. The genuinely-open
design questions a real front-end must still answer — an ergonomic quote surface that does not overload
the backtick, and whether kebab-case survives contact with human authors — are recorded, not resolved.

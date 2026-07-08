# A plain quote evaluates a quasiquote's unquote nested inside it

*2026-07-08*

**What happened.** A plain `(quote …)` is supposed to produce the AST of its body *without
evaluating any of it*. But when a quasiquote is nested inside the quote and that quasiquote contains an
`,<expr>` (unquote), the seed **evaluates the unquote** and embeds the result, dropping the unquote
marker — as if the quoted quasiquote were an active one.

- `(quote `(+ ,(+ 2 3)))` yields the AST for `(quasiquote (+ 5))` — the `,(+ 2 3)` was evaluated to 5
  and the `unquote` node discarded. The correct value is the AST of the template verbatim:
  `(quasiquote (+ (unquote (+ 2 3))))`, with the `(+ 2 3)` and the `unquote` both present as inert
  structure.
- `(quote `(+ ,x))` with `x` bound to 1 yields the AST for `(quasiquote (+ 1))` — same defect with a
  runtime name: the name `x` is resolved to its value instead of appearing as `(unquote x)`.
- **Observable wrong value:** `(let ((x 1)) (let ((y 1)) (= (quote `(+ ,x)) (quote `(+ ,y)))))` yields
  **`true`**; the correct answer is **`false`**. The two quoted templates mention *different* names
  (`x` vs `y`), so their inert ASTs differ. The seed evaluates both nested unquotes (x→1, y→1),
  collapsing both to the AST of `(+ 1)`, and the two compare equal.
- **Wrong rejection, same root:** `(quote `(+ ,undefined-name))` is *rejected* ("unbound name") —
  the seed tried to evaluate the nested unquote and hit the unbound name. The inert-data reading
  compiles it fine (the quoted structure just mentions the name `undefined-name`).

**Why it is a break.** metaprogramming.md #Quote Produces An AST Value: "`(quote <expr>)` MUST
evaluate to an AST sum type value representing the structure of `<expr>`, **without evaluating
`<expr>` itself**." This is unconditional — whatever `<expr>` contains. A quasiquote nested inside a
plain quote is ordinary structure; the plain quote does not establish a quasiquote-active context (that
context is established by an *evaluated* quasiquote, and this one is quoted, not evaluated). So the
`,x` inside it is inert data — the AST mentions the name `x`, not x's value. Selective evaluation
(§Quasiquote Constructs AST With Selective Evaluation) applies to a quasiquote that is *itself
evaluated*, not to one sitting inside a quote. The positive control confirms the wrong value is real,
not an equality artifact: `(quote (+ x 1))` ≠ `(quote (+ y 1))` → correctly **false** (plain-quote
name-distinguishing equality works); only the quote-of-quasiquote path collapses the names.

**Root cause (evaluation-side dual of the already-fixed rejection bug).** This is the exact companion
of the fixed "an unquote nested inside a plain quote is a syntax error, not an active unquote"
(`(quote (g ,x))` must reject CDZ0401 — memory [[plain-quote-rejects-nested-unquote]]). That fix taught
`check_tree` to *reject* a stray unquote directly under a plain quote (level 0). But an unquote one
level deeper — under a *quasiquote* under the quote — is not stray (the quasiquote raises the level to
1), so it is inert data that must be preserved verbatim, NOT evaluated. The seed's quote-evaluation
path (the code that turns a quoted body into an `Ast.*` value) still runs the quasiquote's
selective-evaluation machinery on a quasiquote it merely *quotes*: it should build the `unquote` node
as inert structure, but instead it evaluates the operand and embeds the result. The construction
(build-the-AST) side never learned what the check side learned — a plain quote's body is inert *all
the way down*, so a quasiquote inside it, and any unquote inside that quasiquote, are structure to be
represented, not evaluated. The tell: swap `(quote (+ ,x))` (correctly CDZ0401, stray unquote) for
`(quote `(+ ,x))` (one quasiquote deeper) and correct-reject becomes wrong-evaluate.

**The lesson (a "don't evaluate this body" rule must hold all the way down, not just at the head).**
The fixed rejection bug taught that a checker's "prune this form's body" is a silent accept of anything
ill-formed inside it. The evaluation-side analogue: quote's "represent, don't evaluate" must apply to
the *entire* subtree, including a nested quasiquote and its unquotes. A quasiquote only becomes
"active" (selective-evaluation on) when the quasiquote is itself *evaluated*; a quasiquote that is
merely quoted is inert like everything else under the quote. The seed treats "is a quasiquote node" as
"is active," ignoring whether an enclosing quote has frozen it into data. Same family as the master
pattern: a rule proven on one nesting (quote directly over unquote → reject) must carry to the sibling
nesting (quote over quasiquote over unquote → represent inert, don't evaluate). Both are "a plain quote
evaluates nothing in its body."

**Fix direction (gitignored seed).** In the quote-body → AST construction path, track quasiquote
nesting the same way `unquote_outside_quasiquote` does for the check side: a plain quote starts the
body at "quote depth" with quasiquote level 0; a nested `quasiquote` raises the active level, an
`unquote` lowers it — but crucially, when the whole body is being *quoted* (not evaluated), the
selective-evaluation of an unquote must be suppressed and the `unquote`/`quasiquote` nodes emitted as
inert `Ast.*` structure. Equivalently: only evaluate an unquote when its enclosing quasiquote is on the
*evaluated* path, never when that quasiquote is itself inside a quote. Regression guards: a top-level
*evaluated* quasiquote with an unquote must still evaluate it (`(let ((x 1)) `(f ,x))` → embeds 1);
`(quote (g ,x))` must still reject CDZ0401; `(quote (+ 1 2))` still yields the plain AST.

**Corpus case added.** `spec/semantics/12-metaprogramming.sexp` §"a plain quote does not evaluate a
quasiquote's unquote nested inside it" — `(let ((x 1)) (let ((y 1)) (= (quote `(+ ,x)) (quote `(+ ,y)))))`
MUST yield `false`. Realized (ungated — quote is realized), the behavior gate catches it (expected
false, observed true). Placed right after the "quote produces an AST value without evaluating" case it
strengthens.

Related: [[plain-quote-rejects-nested-unquote]] (the rejection-side dual, already fixed);
[[quote-vs-ast-constructor-equality-miscompiles]] (the quote→Ast bridge); metaprogramming.md #Quote
Produces An AST Value; #Quasiquote Constructs AST With Selective Evaluation.

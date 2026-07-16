; Metaprogramming — quote/quasiquote and the AST as a sum type. Witnesses metaprogramming.md.
; Quote produces an AST value without evaluating; quasiquote allows selective evaluation for
; construction. The AST is a sum type deconstructible by pattern matching, so the compiler
; operates on AST values natively rather than using string-tagged reflection. Eval (executing
; AST as code) is optional for macros/REPL, not needed by the core compiler.

(case "quote produces an AST value without evaluating"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value: (quote <expr>) returns an
           AST sum type value representing <expr>'s structure, without evaluating <expr>.
           (quote (+ 1 2)) produces an AST value, not 3.")
  (input  (quote (+ 1 2)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2))) Ast)))

; --- A plain quote does not evaluate a nested unquote --------------------------------------------
; metaprogramming.md #Quote Produces An AST Value: `(quote <expr>)` produces the AST of <expr>
; "WITHOUT evaluating <expr> itself" — UNCONDITIONALLY, whatever <expr> contains. A quasiquote
; nested inside a plain quote is INERT: `(quote `(+ ,x))` is the AST of the template `(+ ,x)`, in
; which the `,x` (unquote) is ordinary structure — the plain quote does not put it in a
; quasiquote-active context (that context is established by an EVALUATED quasiquote, and this
; quasiquote is quoted, not evaluated). So the `,x` MUST NOT be evaluated: the quoted structure
; mentions the NAME `x`, not x's value. This is the EVALUATION-side dual of "an unquote nested
; inside a plain quote is a syntax error, not an active unquote" (a bare `(quote (g ,x))` rejects
; CDZ0003): here the unquote is one level deeper — under a quasiquote under the quote — so it is
; inert data rather than a stray unquote, but the same principle holds: a plain quote evaluates
; NOTHING in its body. Discriminator that does not depend on the inert node's exact spelling: bind
; two DISTINCT names to the SAME value and quote each — the quoted templates mention different
; names (`x` vs `y`), so they are NOT structurally equal. A compiler that evaluates the nested
; unquote collapses both to the AST of `(+ 1)` and wrongly answers `true`.
(case "a plain quote does not evaluate a quasiquote's unquote nested inside it"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (\"without evaluating <expr>\").
           `(quote `(+ ,x))` and `(quote `(+ ,y))` with x and y both bound to 1 quote two templates
           that mention DIFFERENT names (`x` vs `y`); a plain quote does not evaluate the nested
           `,x`/`,y`, so the two quoted structures differ and `=` is FALSE. A compiler that evaluates
           the nested unquote (embedding x's value 1 and y's value 1) collapses both to the AST of
           `(+ 1)` and wrongly answers true — it treated the quoted quasiquote as an active one,
           evaluating inside a plain quote. Companion (rejection side) below: a bare stray unquote
           under a plain quote is CDZ0003.")
  (input  (let ((x 1)) (let ((y 1)) (= (quote `(+ ,x)) (quote `(+ ,y))))))
  (output (: false Bool)))

(case "eval is optional for macros and interactive use"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional For Macros And Interactive Use: (eval <ast>)
           executes AST as code, optional for macros/REPL. Seed provides it; static generations need
           not. (eval (quote (+ 1 2))) produces 3.")
  (input  (eval (quote (+ 1 2))))
  (output (: 3 Int64)))

(case "eval of a quasiquote splicing a compile-time-known value"
  (doc    "The core macro idiom (metaprogramming.md #Eval Is Optional / #Quasiquote Constructs AST With
           Selective Evaluation): eval a quasiquoted form whose unquote splices a compile-time-known VALUE,
           not just a bare literal. `(let ((x 3)) (eval `(+ ,x 4)))` reconstructs `(+ x 4)` and folds to 7.
           The eval desugar reconstructs `(eval AST)` to the source the AST denotes; an active unquote lifts
           its live operand into `(Ast.Int <e>)`, so reconstruction unwraps that back to `<e>` — a let-bound
           name, a def-const, or a computed constant, all resolving in the eval's enclosing scope. (A bare-
           LITERAL splice `(unquote 3)` and a plain `(quote …)` already worked; a NON-literal unquote once
           left the eval un-desugared, so its head `eval` reported a misleading 'unbound name eval'.) The
           reconstructed source must reach the enclosing `let`, so the desugar blanks the dead reified-
           argument wrappers, leaving the spliced `x` node parented at the eval position. Expected 7.")
  (input  (do
            (def (main) (let ((x 3)) (eval (quasiquote (+ (unquote x) 4)))))
            (export main)))
  (output (: 7 Int64)))

; The eval-of-quasiquote macro idiom composes with the FLOAT and STRING leaves, and `print` renders a
; quoted float re-readably — pinning that the eval/print paths handle the leaves this vertical realized,
; not only integers/names. A float unquote lifts + reconstructs + folds like an integer one; a string
; splices through ordinary String ops; and `print` of a quoted float carries a `.` so it re-reads.

(case "eval of a quasiquote-built form with a float unquote folds"
  (doc    "The float companion of the eval-splice idiom: `(let ((x 2.5)) (eval `(+ ,x 1.5)))` lifts the
           float `x` into the reconstructed `(+ x 1.5)` and folds to 4.0 — the active-unquote float lift
           (Ast.Float) composes with `eval`'s source reconstruction exactly as the integer case does.")
  (input  (do
            (def (main) (let ((x 2.5)) (eval (quasiquote (+ (unquote x) 1.5)))))
            (export main)))
  (output (: 4.0 Float64)))

(case "eval of a quasiquote splicing a string works through String ops"
  (doc    "The string companion: `(let ((s \"hi\")) (String.byte-len (eval `(String.concat ,s \"x\"))))`
           splices the runtime string `s` into the reconstructed `(String.concat s \"x\")`, evaluates it to
           `\"hix\"`, and reads its length 3. Pins that a string unquote reconstructs + folds through
           ordinary String operations in the eval'd source.")
  (input  (do
            (def (main) (let ((s "hi")) (String.byte-len (eval (quasiquote (String.concat (unquote s) "x"))))))
            (export main)))
  (output (: 3 Int64)))

(case "print of a quote containing a float renders re-readably"
  (doc    "`print : Ast → String` renders a quoted compound containing a float as its canonical re-readable
           s-expression: `(quote (f 1.5))` prints `\"(f 1.5)\"` — the `Ast.Float` leaf renders with a `.` so
           the text re-reads as a float (not an integer). Pins that `print` handles the float leaf inside a
           compound, the companion of the leaf-level print/read round-trip cases.")
  (input  (= (print (quote (f 1.5))) "(f 1.5)"))
  (output (: true Bool)))

; `print`'s EXACT canonical rendering — not just its round-trip. The `read(print v) == v` cases pin the
; printer/reader as INVERSES, but a round-trip normalizes, so it does NOT pin the exact text `print` emits
; (spacing between elements, nested parenthesization, the empty-list form). These assert the literal string,
; catching a printer that changed spacing/nesting yet still round-tripped: a deep compound with a nested
; list and a quoted string renders `(f (g 1) "s")` (one space between elements, inner parens, the Str leaf
; quoted), and an empty list renders `()`.

(case "print renders a nested compound with a string leaf as its exact canonical text"
  (doc    "`print` of `(quote (f (g 1) \"s\"))` is exactly `\"(f (g 1) \\\"s\\\")\"`: elements space-
           separated, the nested list `(g 1)` parenthesized in place, and the `Ast.Str` leaf rendered as a
           QUOTED literal (distinct from the bare name `f`). Pins the exact rendering of nesting + spacing +
           string-quoting in one string — a printer that dropped a space or a paren would still round-trip
           but flip this literal-text assertion.")
  (input  (= (print (quote (f (g 1) "s"))) "(f (g 1) \"s\")"))
  (output (: true Bool)))

(case "print renders an empty Ast.List as the empty-parens form"
  (doc    "`print (Ast.List (list))` is exactly `\"()\"` — the zero-element list rendering (open then close
           with nothing between). Pins the empty-list edge of the printer, which the non-empty compound
           cases never reach.")
  (input  (= (print (Ast.List (list))) "()"))
  (output (: true Bool)))

(case "print renders a single-element Ast.List as one parenthesized element"
  (doc    "`print (quote (f))` is exactly `\"(f)\"` — the ONE-element (arity-1) list: open paren, the single
           element, close paren, no inter-element space. Completes the list-arity rendering coverage — 0
           elements → `()`, 1 → `(f)`, 2+ → the nested/compound cases above. Pins that the space-separator
           logic (only BETWEEN elements) emits none for a lone element.")
  (input  (= (print (quote (f))) "(f)"))
  (output (: true Bool)))

(case "the AST is a sum type deconstructible by pattern matching"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (2nd sentence): the AST is a
           sum type with variants for each syntactic form. Pattern matching over (quote 42) binds
           the integer payload, demonstrating AST variants are proper sum types. Because the AST is
           an ORDINARY sum (type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type),
           its match is subject to the same exhaustiveness rule any sum match is (#A Match Is
           Exhaustive Against The Sum Type's Variant Set), so a match that inspects one form carries a
           catch-all `_` arm for the others.")
  (input  (match (quote 42)
            ((Ast.Int n) n)
            (_ 0)))
  (output (: 42 Int64)))

(case "pattern matching over AST distinguishes forms"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value: the compiler pattern-matches
           over AST sums to distinguish syntactic forms. Matching (quote (+ 1 2)) as an Ast.List
           allows inspecting its structure recursively. The AST is an ordinary sum, so the match
           covers the remaining variants with a catch-all `_` arm (#A Match Is Exhaustive Against The
           Sum Type's Variant Set).")
  (input  (match (quote (+ 1 2))
            ((Ast.List elems) (List.len elems))
            (_                0)))
  (output (: 3 Int64)))

(case "eval on malformed AST traps"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional: eval on malformed AST traps. An Ast.List
           with no elements is malformed (no operator), so eval traps. The eval desugar reconstructs
           the source an `Ast.*` construction denotes; an empty `Ast.List` has no operator to
           reconstruct, so `eval_ast::reconstruct` rewrites it to an explicit `(trap \"malformed AST\")`
           — a diverging halt, not a value. The trap's canonical KIND is `unreachable`, the SAME on
           every backend: an explicit `trap` lowers to wasm's `unreachable` instruction and to a Rust
           `panic!` whose reason classifies as `unreachable` (a message-less halt — the trap_kind grader
           classifies the actual reason, and `Core::Trap` carries no string through either backend, so
           the observable kind is `unreachable`, matching the explicit-`trap` lowering pinned by the
           runtime expect-on-absent case in 02-binding-and-control.sexp).")
  (input  (eval (Ast.List (list))))
  (trap   "unreachable"))

(case "quoting an empty compound produces an empty Ast.List"
  (doc    "`(quote ())` reifies the empty compound `()` to an EMPTY `Ast.List` — the reifier maps a
           parenthesized form to `Ast.List` of its reified elements, and zero elements give an empty list
           (NOT a reify error, and NOT a leaf). `List.len` of its elements is 0. The source-level companion
           of the constructor-built `(Ast.List (list))`: this is the very value the eval-malformed case
           above traps on, so it pins where that empty list COMES FROM — a quoted empty compound is a
           well-formed (if operator-less) Ast, distinct from a leaf or a rejected form.")
  (input  (match (quote ())
            ((Ast.List es) (List.len es))
            (_             -1)))
  (output (: 0 Int64)))

(case "quasiquote constructs AST with selective evaluation"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           `<template> quotes like quote, but ,<expr> evaluates <expr> normally and inserts result
           into the AST being constructed. `(+ ,x 10) with x=2 produces AST for (+ 2 10), not (+ x 10).
           This is construction, not eval — ,x evaluates the variable x, not an AST.")
  (input  (let ((x 2)) `(+ ,x 10)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

(case "unquote in quasiquote evaluates normally and embeds"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,<expr> evaluates <expr> normally (not as AST) and embeds the result.
           `(+ ,(+ 1 1) 10) evaluates (+ 1 1) to 2, constructs AST with that value.")
  (input  `(+ ,(+ 1 1) 10))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

; --- An active unquote lifts its operand by the operand's VALUE KIND ------------------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation: an active `,<expr>` evaluates
; <expr> and INSERTS ITS RESULT into the AST being constructed. The inserted node is the `Ast.*` leaf
; that VALUE denotes — an integer becomes `Ast.Int`, a boolean `Ast.Bool`, a string `Ast.Str` — the same
; leaf `quote` of that literal produces (so `` `(f ,true) `` embeds the SAME `(Ast.Bool true)` node as
; `(quote (f true))`). Now that the `Ast` sum carries the boolean and string forms, an active unquote of
; a boolean/string literal lifts to the matching leaf rather than declining. (A RUNTIME operand — a name
; or a computed expression — still lifts as `Ast.Int` this increment: its type is not known at reify
; time, so a non-Int runtime operand declines rather than miscompiling; the inferred-type lift of a
; runtime operand is a later increment.)

(case "an active unquote of a boolean literal lifts to an Ast.Bool node"
  (doc    "`` `(f ,true) `` embeds the boolean literal `true` as the `Ast.Bool` leaf its value denotes —
           the same node `(quote (f true))` builds — so it equals `(Ast.List (list (Ast.Name \"f\")
           (Ast.Bool true)))` (metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           the unquote inserts its result). The boolean companion of the integer embed case above.")
  (input  (= (quasiquote (f (unquote true)))
             (Ast.List (list (Ast.Name "f") (Ast.Bool true)))))
  (output (: true Bool)))

(case "an active unquote of a string literal lifts to an Ast.Str node"
  (doc    "`` `(f ,\"x\") `` embeds the string literal `\"x\"` as the `Ast.Str` leaf — the same node
           `(quote (f \"x\"))` builds — so it equals `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"x\")))`.
           The string companion; pins that the active-unquote lift dispatches on the operand's value kind
           (a string literal → `Ast.Str`, not the `Ast.Int` the integer/runtime path uses).")
  (input  (= (quasiquote (f (unquote "x")))
             (Ast.List (list (Ast.Name "f") (Ast.Str "x")))))
  (output (: true Bool)))

(case "an active-unquoted boolean literal equals the quoted form"
  (doc    "The unquote-vs-quote agreement for the boolean form: `` `(f ,true) `` and `(quote (f true))`
           build the SAME `Ast` value (both `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`), so they
           are structurally equal (core-semantics.md #Equality Is Structural). An active unquote of a
           literal produces the same node quote of that literal does.")
  (input  (= (quasiquote (f (unquote true))) (quote (f true))))
  (output (: true Bool)))

; --- An active unquote of a RUNTIME operand lifts by the operand's inferred type ------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation: ",<expr> MUST evaluate <expr>
; normally and INSERT ITS RESULT into the AST at that position." When <expr> is a RUNTIME value (a bound
; name, a parameter, a computed expression) its type is not known at reify time, so it lifts by its
; INFERRED type at lowering: a value that is ALREADY an `Ast` is spliced AS-IS (identity — the compiler's
; own subtree-embedding idiom); a scalar is wrapped in the matching leaf (Int64 → `Ast.Int`, Bool →
; `Ast.Bool`, String → `Ast.Str`). This is the runtime companion of the literal-operand cases above,
; where the leaf kind is known structurally. (Previously a runtime operand was wrapped `Ast.Int`
; unconditionally, type-erroring a non-Int operand and — crucially — an already-`Ast` operand against
; `Ast.Int`'s Int64 payload.)

(case "an active unquote of an Ast-valued expression splices the subtree as identity"
  (doc    "The canonical AST-building macro: `(def (wrap sub) `(+ ,sub 1))` embeds a COMPUTED sub-AST into
           a template. When the unquoted value is ALREADY an `Ast`, \"insert its result\" splices that node
           AS-IS — NOT re-wrapped in `Ast.Int` (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation). `(wrap (Ast.Int 9))` builds `(+ 9 1)` — a 3-element `Ast.List` — so `List.len` is 3.
           Pins the identity lift the compiler/macro layer needs; previously this type-errored (CDZ0201,
           Ast against Ast.Int's Int64 payload).")
  (input  (do
            (def (wrap (: sub Ast)) (quasiquote (+ (unquote sub) 1)))
            (def (main) (match (wrap (Ast.Int 9)) ((Ast.List es) (List.len es)) (_ -1)))
            (export main)))
  (output (: 3 Int64)))

; --- The boundary of the OPTIONAL eval surface: eval does not see through an Ast-VALUE splice --------
; `eval`'s desugar (eval_ast::reconstruct) reconstructs SOURCE statically by walking the reified template.
; A splice whose operand is an ordinary VALUE (a literal, a bound scalar, a computed expression) reconstructs
; to that operand as source and folds — the working macro idiom above (`(eval `(+ ,x 4))` → 7, and a runtime
; `,n` → 7). But a splice whose operand is ITSELF an `Ast` value — `(unquote (quote (* 2 3)))`, or a
; let-bound quoted subtree — reconstructs to `(+ <Ast-value> 1)`: an `Ast` in a numeric position, which is
; the ordinary type error CDZ0201. Evaluating THAT would require the desugar to RECURSIVELY INTERPRET a
; runtime `Ast` subtree as code — a nested RUNTIME eval, precisely the "execute an arbitrary runtime AST"
; capability metaprogramming.md marks OPTIONAL (the seed ships a compile-time-FOLD eval, not a runtime
; interpreter). So this is a SOUND decline, not a bug: the CONSTRUCTION splices the subtree fine (the case
; above), and eval of the HAND-BUILT equal tree works — it is only the static source-reconstruction that
; does not see through a spliced Ast value. Pinned (breaker-found, ruled a deliberate limit) so the boundary
; can't silently flip: the value-splice keeps working, the Ast-value-splice keeps declining.

(case "eval of a template splicing a runtime VALUE folds — the working side of the boundary"
  (doc    "The positive face: a splice whose operand is an ordinary runtime value reconstructs as source and
           folds. `(main n) = (eval `(+ ,n 1))` with runtime n=6 → 7 — the operand `n` is spliced as source,
           not as an `Ast` node, so the reconstructed `(+ n 1)` evaluates. Contrast the Ast-value-splice
           decline below.")
  (input  (do
            (def (main (: n Int64)) (eval (quasiquote (+ (unquote n) 1))))
            (export main)))
  (call   main (: 6 Int64))
  (output (: 7 Int64)))

(case "eval does not see through a splice whose operand is itself an Ast value"
  (doc    "The boundary of the optional eval surface: `(eval `(+ ,(quote (* 2 3)) 1))` DECLINES. The
           unquote operand `(quote (* 2 3))` is itself an `Ast` value, so eval's static source
           reconstruction produces `(+ <Ast-value> 1)` — an `Ast` in a numeric position (CDZ0201).
           Evaluating it would need a nested RUNTIME AST interpreter (metaprogramming.md marks runtime eval
           OPTIONAL; the seed folds at compile time). Sound decline, NOT a bug — the construction splices the
           subtree fine (case above) and eval of the hand-built equal tree works; only the static
           reconstruction does not see through a spliced Ast value. Breaker-found; ruled a deliberate limit.")
  (input  (eval (quasiquote (+ (unquote (quote (* 2 3))) 1))))
  (declines))

; The boundary is specifically the EVAL/execution surface — not the spliced Ast value, which is a
; perfectly well-formed tree. The SAME template that `eval` declines above is handled by the NON-executing
; interchange paths: `print` renders it and `Ast.encode`/`Ast.decode` round-trip it. These pin that an
; Ast-value-spliced template is a valid AST (it is only RUNNING it as code that hits the optional-runtime-
; eval line), so a future reader does not mistake the eval decline for a malformed template.

(case "print renders a template that splices an Ast value — the non-executing path works"
  (doc    "The same template `eval` declines above prints fine: `(quasiquote (+ ,(quote (* 2 3)) 1))` splices
           the quoted subtree `(* 2 3)` at its position, and `print` renders the whole as `\"(+ (* 2 3) 1)\"`.
           Pins that the Ast-value splice builds a WELL-FORMED tree (the eval limit is the execution surface,
           not the construction) — `print` reads through the spliced subtree with no decline.")
  (input  (= (print (quasiquote (+ (unquote (quote (* 2 3))) 1))) "(+ (* 2 3) 1)"))
  (output (: true Bool)))

(case "an Ast-value-spliced template round-trips through encode and decode"
  (doc    "The byte-path companion: the template `eval` declines encodes and decodes back equal. `Ast.encode`/
           `Ast.decode` treat the spliced `(* 2 3)` subtree as ordinary nested AST structure, so the
           bijection holds over it — confirming the spliced value is a valid AST the interchange paths handle,
           and only the eval/execution surface has the (optional-runtime-eval) limit.")
  (input  (match (Ast.decode (Ast.encode (quasiquote (+ (unquote (quote (* 2 3))) 1))))
            ((Ok a)  (= a (quasiquote (+ (unquote (quote (* 2 3))) 1))))
            ((Err _) false)))
  (output (: true Bool)))

(case "an active unquote of a let-bound boolean lifts to Ast.Bool by inferred type"
  (doc    "A RUNTIME operand (a let-bound name) lifts by its inferred type: `b : Bool` → `Ast.Bool`.
           `(let ((b true)) `(f ,b))` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`. Pins the
           runtime-Bool lift (the literal case is above; this exercises the inferred-type path at lower).")
  (input  (let ((b true))
            (= (quasiquote (f (unquote b)))
               (Ast.List (list (Ast.Name "f") (Ast.Bool true))))))
  (output (: true Bool)))

(case "an active unquote of a let-bound string lifts to Ast.Str by inferred type"
  (doc    "The runtime-String companion: `s : String` → `Ast.Str`. `(let ((s \"hi\")) `(f ,s))` builds
           `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"hi\")))`. Pins the runtime-String inferred-type lift.")
  (input  (let ((s "hi"))
            (= (quasiquote (f (unquote s)))
               (Ast.List (list (Ast.Name "f") (Ast.Str "hi"))))))
  (output (: true Bool)))

(case "an active unquote of a let-bound integer still lifts to Ast.Int"
  (doc    "Regression guard: a runtime Int64 operand still lifts to `Ast.Int` (the original active-unquote
           behavior, now via the inferred-type path). `(let ((n 42)) `(op-const ,n))` builds
           `(Ast.List (list (Ast.Name \"op-const\") (Ast.Int 42)))`.")
  (input  (let ((n 42))
            (= (quasiquote (op-const (unquote n)))
               (Ast.List (list (Ast.Name "op-const") (Ast.Int 42))))))
  (output (: true Bool)))

(case "an active unquote of a computed boolean expression lifts to Ast.Bool"
  (doc    "A non-leaf (computed) runtime operand lifts by its inferred type too: `(= 1 1) : Bool` →
           `Ast.Bool`. `` `(f ,(= 1 1)) `` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`.
           Pins that the inferred-type lift covers a computed expression, not only a bound name.")
  (input  (= (quasiquote (f (unquote (= 1 1))))
             (Ast.List (list (Ast.Name "f") (Ast.Bool true)))))
  (output (: true Bool)))

; An unquote MUST EVALUATE its expression (metaprogramming.md #Quasiquote Constructs AST With Selective
; Evaluation: ",<expr> … MUST evaluate <expr> normally and insert its result"). If that expression cannot
; be evaluated because it references an UNBOUND name, that is the ordinary unbound-name error
; (core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding MUST be a
; compile-time error", unconditional) — NOT an occasion to fall back to quoting the expression as inert
; AST. `` `(a ,(+ b 1)) `` with `b` unbound must be rejected CDZ0101, exactly as the bare `(+ b 1)` is,
; because the unquote evaluates `(+ b 1)` and `b` has no binding. A compiler that, when an unquote's
; expression fails to const-evaluate, silently QUOTES it (yielding `(Ast.List (Ast.Name "+") (Ast.Name
; "b") (Ast.Int 1))`) turns a scope error into a valid AST value — the unquote stopped evaluating and
; became a second quote, contradicting the selective-EVALUATION rule.

(case "an unquote of an expression with an unbound name is rejected, not quoted"
  (doc    "`` `(a ,(+ b 1)) `` unquotes `(+ b 1)`, which references the unbound name `b` — the unquote
           MUST evaluate its expression (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation), so this is the ordinary unbound-name error (CDZ0101, core-semantics.md #Binding
           Is Lexical — unconditional), exactly as the bare `(+ b 1)` is. Pins that an unquote whose
           expression cannot be evaluated is rejected, NOT silently quoted as inert AST: a compiler that
           falls back to quoting the un-evaluable expression (yielding an `(Ast.List …)` for `(+ b 1)`)
           turns the selective-evaluation unquote into a second quote and swallows the scope error. With
           `b` bound (`(let ((b 5)) `(a ,(+ b 1)))`) the unquote evaluates to 6; unbound, it is CDZ0101.")
  (input  `(a ,(+ b 1)))
  (error  CDZ0101))

; An AST built by quasiquote-with-unquote is an ordinary AST VALUE: it must be structurally EQUAL to
; the same AST built any other way, and encode to the same bytes (core-semantics.md #Equality Is
; Structural; the AST is an ordinary sum type — type-system.md #The Abstract Syntax Tree Type Is An
; Ordinary Sum Type). So `` `(f ,x) `` with x=1 equals `` `(f ,1) `` and `(quote (f 1))` — all three
; are `(Ast.List (Ast.Name "f") (Ast.Int 1))`. An unquote that embeds a RUNTIME (let-bound) value
; must build the same `(Ast.Int 1)` node a const fold produces, so structural equality and encoding
; see the two as identical. This is the compiler's own idiom: it builds instruction ASTs by
; quasiquoting runtime values, then compares/encodes them.

(case "an AST from quasiquoting a runtime value equals the same AST built by quote"
  (doc    "`` `(f ,x) `` with x bound to 1 builds `(Ast.List (Ast.Name \"f\") (Ast.Int 1))`, the same
           AST `(quote (f 1))` builds — so they are structurally equal (core-semantics.md #Equality Is
           Structural). An unquote that embeds a runtime value produces the same node as a const fold,
           so the two compare equal. MUST be true.")
  (input  (let ((x 1)) (= `(f ,x) (quote (f 1)))))
  (output (: true Bool)))

(case "quasiquotes unquoting a runtime variable and a literal build equal ASTs"
  (doc    "The companion isolating the runtime-vs-const embedding: `` `(f ,x) `` (x=1, a runtime local)
           and `` `(f ,1) `` (a literal) build the same AST and MUST be equal — the runtime-unquoted
           node is structurally identical to the const-unquoted one.")
  (input  (let ((x 1)) (= `(f ,x) `(f ,1))))
  (output (: true Bool)))

; --- A quoted AST equals the same AST built by applying the Ast.* constructors -------------
; `quote` produces an AST SUM value (metaprogramming.md #Quote Produces An AST Value: "MUST evaluate
; to an AST sum type value"; type-system.md #The Abstract Syntax Tree Is An Ordinary Sum Type). The
; AST's variants are the ordinary constructors `Ast.Int` / `Ast.Name` / `Ast.List` / …, so a value
; `quote` builds and the SAME value built by applying those constructors are ONE sum value — the
; corpus already records `(quote (+ 1 2))`'s value form AS `(Ast.List (list (Ast.Name "+") (Ast.Int 1)
; (Ast.Int 2)))` (the first case in this file) and matches `(quote 42)` against `(Ast.Int n)` binding
; n=42. Structural equality (core-semantics.md #Equality Is Structural) must therefore see the two as
; equal: `(= (quote 42) (Ast.Int 42))` is true, exactly as `(= (quote (f 1)) (quote (f 1)))` and
; `(= (Ast.Int 42) (Ast.Int 42))` are. Pattern matching already normalizes the two — `(match (quote
; 42) ((Ast.Int n) n) …)` binds 42 — so equality must agree, or the AST is a sum value under `match`
; but a distinct thing under `=`, splitting the one value form the encoding is a bijection over
; (ast-encoding.md #The Encoding Is A Bijection With One Canonical Byte Form).

(case "a quoted integer equals the same node built by the Ast.Int constructor"
  (doc    "`(quote 42)` is the AST sum value `(Ast.Int 42)` (metaprogramming.md #Quote Produces An AST
           Value — quote evaluates to an AST SUM value). The corpus records quote outputs in exactly
           this constructor form and matches `(quote 42)` as `(Ast.Int n)` binding 42, so the two
           denote ONE sum value and structural equality MUST be true. A representation that stores a
           quote result differently from an applied Ast.* constructor — comparing them unequal — splits
           the single AST value form the encoding bijection is defined over. MUST be true.")
  (input  (= (quote 42) (Ast.Int 42)))
  (output (: true Bool)))

(case "a quoted name equals the same node built by the Ast.Name constructor"
  (doc    "The Name companion: `(quote foo)` is `(Ast.Name \"foo\")` — a quoted bare name is the
           Ast.Name sum value carrying the name as a String payload (metaprogramming.md #Quote Produces
           An AST Value). `(= (quote foo) (Ast.Name \"foo\"))` MUST be true, exactly as the Int case.
           Pins that the quote-vs-constructor equality holds for the leaf name node too.")
  (input  (= (quote foo) (Ast.Name "foo")))
  (output (: true Bool)))

; --- The Ast.Bool leaf variant --------------------------------------------------------------------
; The built-in `Ast` is an ordinary sum type with "a variant per syntactic form (an integer, a float, a
; string, a BOOLEAN, a name, and a list of child nodes)" (type-system.md #The Abstract Syntax Tree Type
; Is An Ordinary Sum Type). A BOOLEAN literal is one such form, so `(quote true)` is the `Ast` sum value
; `(Ast.Bool true)` — the boolean companion of `(quote 42)`=`(Ast.Int 42)` and `(quote foo)`=`(Ast.Name
; "foo")`. It carries a `Bool` payload (a single-arity variant constructor whose argument is type-checked,
; like every other `Ast.*`), it destructures by pattern match binding that payload, it round-trips through
; `Ast.encode`/`Ast.decode` and `print`/`read`, and `eval` executes it (a boolean form evaluates to itself).

(case "a quoted boolean equals the same node built by the Ast.Bool constructor"
  (doc    "The boolean companion of the Int/Name equality cases: `(quote true)` is the `Ast` sum value
           `(Ast.Bool true)` (metaprogramming.md #Quote Produces An AST Value; type-system.md #The Abstract
           Syntax Tree Type Is An Ordinary Sum Type — a boolean is a syntactic form). `(= (quote true)
           (Ast.Bool true))` MUST be true (core-semantics.md #Equality Is Structural), exactly as
           `(= (quote 42) (Ast.Int 42))` is — the quote result and the constructor-built node are ONE value.")
  (input  (= (quote true) (Ast.Bool true)))
  (output (: true Bool)))

(case "a match binds an Ast.Bool payload"
  (doc    "The `Ast` sum is deconstructible by pattern matching like any other sum (type-system.md #The
           Abstract Syntax Tree Type Is An Ordinary Sum Type), so a match over `(quote false)` binds the
           `Ast.Bool` payload. The arm returns the bound boolean; the catch-all covers the other variants
           (the match is exhaustive against the sum's variant set). Yields false.")
  (input  (match (quote false)
            ((Ast.Bool b) b)
            (_            true)))
  (output (: false Bool)))

(case "a built-in Ast.Bool constructor applied to a wrong-type payload is a type error"
  (doc    "`Ast.Bool`'s payload type is Bool (a variant per syntactic form — type-system.md #The Abstract
           Syntax Tree Type Is An Ordinary Sum Type), so `(Ast.Bool 5)` applies it to an Int64 — a type
           mismatch the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")` (a String where Int64
           is declared) is. Pins that the built-in `Ast.Bool` constructor type-checks its declared payload
           like any user sum variant.")
  (input  (Ast.Bool 5))
  (error  CDZ0201))

(case "a quoted compound form containing a boolean reifies with an Ast.Bool element"
  (doc    "A boolean nested inside a quoted compound reifies as an `Ast.Bool` element, exactly as an
           integer reifies as `Ast.Int`. `(quote (f true))` is `(Ast.List (list (Ast.Name \"f\") (Ast.Bool
           true)))`, so comparing it against that hand-built node MUST be true — the leaf reification is
           structural and covers the boolean form.")
  (input  (= (quote (f true)) (Ast.List (list (Ast.Name "f") (Ast.Bool true)))))
  (output (: true Bool)))

(case "eval of a quoted boolean executes it to the boolean value"
  (doc    "eval executes an AST value as code (metaprogramming.md #Eval Is Optional For Macros And
           Interactive Use); a boolean form evaluates to itself, so `(eval (quote true))` runs to true.
           The boolean companion of `(eval (quote (+ 1 2)))`=3 — `eval` reconstructs the source the
           `Ast.Bool` denotes (the `true` literal) and folds it through the ordinary path.")
  (input  (do (def (main) (eval (quote true))) (export main)))
  (output (: true Bool)))

(case "encoding and decoding an Ast.Bool round-trips to an equal value"
  (doc    "`(Ast.Bool true)` is an AST value; encoding then decoding it MUST yield an equal AST
           (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t), exactly as the Int/Name/
           List round-trips do. `Ast.decode : Bytes → Result<Ast, _>` is total, so the round-trip matches
           the `Ok` arm and equates its payload.")
  (input  (match (Ast.decode (Ast.encode (Ast.Bool true)))
            ((Ok a)  (= a (Ast.Bool true)))
            ((Err _) false)))
  (output (: true Bool)))

(case "print of an Ast.Bool renders the bare word and read inverts it"
  (doc    "`print : Ast → String` renders an `Ast.Bool` as the bare word `true`/`false` — the canonical
           re-readable spelling — and `read : String → Ast` parses it back, so `read(print v) == v`
           (compiler-pipeline.md — the printer and reader are inverse over the AST value). A boolean word
           is unambiguously a boolean literal (never a name), so the round-trip is exact.")
  (input  (= (read (print (Ast.Bool false))) (Ast.Bool false)))
  (output (: true Bool)))

; --- The Ast.Str leaf variant ---------------------------------------------------------------------
; A STRING is one of the syntactic forms the `Ast` sum carries (type-system.md #The Abstract Syntax Tree
; Type Is An Ordinary Sum Type: "an integer, a float, a STRING, a boolean, a name, and a list"), so
; `(quote "hi")` is the `Ast` value `(Ast.Str "hi")`. `Ast.Str` is DISTINCT from `Ast.Name` even though
; both carry a String payload: `Ast.Str` is a string LITERAL (a value), `Ast.Name` is an identifier
; REFERENCE — so `(quote "foo")` (the string) and `(quote foo)` (the name) are different `Ast` values.
; The leaf constructs, destructures by match, round-trips through `Ast.encode`/`Ast.decode` and
; `print`/`read` (the printed `"…"` uses the closed escape set and reads back exactly), and `eval`
; executes it (a string form evaluates to itself).

(case "a quoted string equals the same node built by the Ast.Str constructor"
  (doc    "`(quote \"hi\")` is the `Ast` sum value `(Ast.Str \"hi\")` (metaprogramming.md #Quote Produces
           An AST Value; type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type — a string
           is a syntactic form). `(= (quote \"hi\") (Ast.Str \"hi\"))` MUST be true (core-semantics.md
           #Equality Is Structural), the string companion of the Int/Bool/Name equality cases.")
  (input  (= (quote "hi") (Ast.Str "hi")))
  (output (: true Bool)))

(case "a quoted string is distinct from the same text quoted as a name"
  (doc    "`Ast.Str` (a string LITERAL) and `Ast.Name` (an identifier reference) are different variants
           even though both carry a String payload. `(quote \"foo\")` is `(Ast.Str \"foo\")`, NOT
           `(Ast.Name \"foo\")`, so comparing them is FALSE — the reifier maps a string literal and a bare
           name to distinct forms. Pins that a string is not collapsed to a name (they are separate
           syntactic forms).")
  (input  (= (quote "foo") (Ast.Name "foo")))
  (output (: false Bool)))

(case "a match binds an Ast.Str payload"
  (doc    "The `Ast` sum is deconstructible by pattern matching (type-system.md #The Abstract Syntax Tree
           Type Is An Ordinary Sum Type), so a match over `(quote \"hey\")` binds the `Ast.Str` payload —
           the String literal — and `String.byte-len` of it is 3. The catch-all covers the other variants.")
  (input  (match (quote "hey")
            ((Ast.Str s) (String.byte-len s))
            (_           0)))
  (output (: 3 Int64)))

(case "a built-in Ast.Str constructor applied to a wrong-type payload is a type error"
  (doc    "`Ast.Str`'s payload type is String, so `(Ast.Str 5)` applies it to an Int64 — a type mismatch
           the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")` and `(Ast.Bool 5)` are. Pins
           that the built-in `Ast.Str` constructor type-checks its declared payload like any sum variant.")
  (input  (Ast.Str 5))
  (error  CDZ0201))

(case "a quoted compound form containing a string reifies with an Ast.Str element"
  (doc    "A string nested inside a quoted compound reifies as an `Ast.Str` element. `(quote (f \"x\"))` is
           `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"x\")))` — the head `f` is a name, the argument
           `\"x\"` a string literal — so comparing it against that hand-built node MUST be true. Pins that
           the string leaf reifies structurally inside a list, distinct from the head name.")
  (input  (= (quote (f "x")) (Ast.List (list (Ast.Name "f") (Ast.Str "x")))))
  (output (: true Bool)))

(case "eval of a quoted string executes it to the string value"
  (doc    "eval executes an AST value as code (metaprogramming.md #Eval Is Optional For Macros And
           Interactive Use); a string form evaluates to itself, so `(eval (quote \"abcd\"))` runs to the
           string `\"abcd\"` — `String.byte-len` of it is 4. The string companion of `(eval (quote true))`
           — `eval` reconstructs the source the `Ast.Str` denotes (the string literal) and folds it.")
  (input  (do (def (main) (String.byte-len (eval (quote "abcd")))) (export main)))
  (output (: 4 Int64)))

(case "encoding and decoding an Ast.Str round-trips to an equal value"
  (doc    "`(Ast.Str \"hi\")` is an AST value; encoding then decoding it MUST yield an equal AST
           (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t), exactly as the Int/Bool/
           Name/List round-trips do. `Ast.decode` is total, so the round-trip matches the `Ok` arm.")
  (input  (match (Ast.decode (Ast.encode (Ast.Str "hi")))
            ((Ok a)  (= a (Ast.Str "hi")))
            ((Err _) false)))
  (output (: true Bool)))

; --- The Name text round-trip is scoped to grammatically-valid identifiers; the byte codec is total ---
; `print` renders an `Ast.Name` as its bare word, and `read` classifies a bare token by the language's
; number/identifier boundary: a DIGIT-LED token is a NUMBER (spec/learnings — a digit-led token is a
; number, never an identifier). So an `Ast.Name` whose spelling is digit-led (`"1.5"`, `"123"`) — a name
; that CANNOT arise from parsing real source, since no valid identifier starts with a digit — prints as
; that numeric text and reads back as `Ast.Float`/`Ast.Int`, not the original `Name`. This is the correct
; grammar behavior, not a bug: the TEXT round-trip `read(print v) == v` holds for well-formed names (a valid
; identifier). The BYTE codec is total over ANY name string — its tag delimits the payload — so a digit-led
; name still round-trips through `encode`/`decode`. These pin the boundary so it can't silently change and
; so the two interchange paths' differing domains are explicit. (Found bug-hunting; the printer docstring
; was corrected from an unconditional round-trip claim to this scoped one.)

(case "the byte codec round-trips a digit-led Ast.Name that the text path would reclassify"
  (doc    "`Ast.encode`/`Ast.decode` is total over any `Name` string: `(Ast.Name \"1.5\")` — a name spelled
           like a float — round-trips to an EQUAL `Ast.Name` through the byte path, because the Name tag
           delimits its payload (no re-lexing). Contrast the text path below, which reclassifies it. Pins
           that the codec's domain is every name, digit-led or not.")
  (input  (match (Ast.decode (Ast.encode (Ast.Name "1.5")))
            ((Ok a)  (= a (Ast.Name "1.5")))
            ((Err _) false)))
  (output (: true Bool)))

(case "print then read of a digit-led Ast.Name reclassifies it per the number/identifier boundary"
  (doc    "The TEXT round-trip is scoped to grammatically-valid identifiers. `print (Ast.Name \"1.5\")`
           renders the bare word `1.5`, and `read` classifies a digit-led token as a NUMBER (the language's
           number/identifier boundary — no valid identifier is digit-led), so it comes back as an
           `Ast.Float`, not the original `Ast.Name`. This is correct grammar behavior, NOT a round-trip bug:
           `Ast.Name \"1.5\"` is a name that could never be parsed from source. Pins the boundary (matched
           via the Float arm) so a future printer/reader change is a deliberate decision, not an accident.")
  (input  (match (read (print (Ast.Name "1.5")))
            ((Ast.Float _) 1)
            ((Ast.Name _)  2)
            (_             0)))
  (output (: 1 Int64)))

; The keyword companion of the digit-led boundary: `true`/`false` are BOOLEAN literals in the grammar, not
; identifiers, so the same text-round-trip scoping applies. `print (Ast.Name "true")` emits the bare word
; `true`, which `read` classifies as `Ast.Bool` (the reader's keyword arm) — not the original `Ast.Name`.
; Like a digit-led name, `Ast.Name "true"` cannot arise from parsing real source (the lexer yields `true`
; as a boolean, never a name). The byte codec is total over it. (These correct the reader's comment that
; claimed "a name can never collide" — a HAND-CONSTRUCTED keyword/numeric-spelled name can, and the text
; round-trip is scoped to grammatically-valid identifiers accordingly.)

(case "the byte codec round-trips a keyword-spelled Ast.Name that the text path would reclassify"
  (doc    "`Ast.encode`/`Ast.decode` is total over a name spelled like a keyword: `(Ast.Name \"true\")`
           round-trips to an EQUAL `Ast.Name` through the byte path (its tag delimits the payload, no
           re-lexing). The keyword companion of the digit-led byte-codec case; contrast the text path below.")
  (input  (match (Ast.decode (Ast.encode (Ast.Name "true")))
            ((Ok a)  (= a (Ast.Name "true")))
            ((Err _) false)))
  (output (: true Bool)))

(case "print then read of a keyword-spelled Ast.Name reclassifies it as the boolean literal"
  (doc    "`print (Ast.Name \"true\")` renders the bare word `true`, which `read` classifies as `Ast.Bool`
           (the reader's keyword arm — `true`/`false` are boolean literals, not identifiers), NOT the
           original `Ast.Name`. Like the digit-led case, `Ast.Name \"true\"` can't arise from source (the
           lexer never yields a name spelled `true`). Correct grammar behavior, not a bug — the text
           round-trip is scoped to grammatically-valid identifiers. Matched via the Bool arm.")
  (input  (match (read (print (Ast.Name "true")))
            ((Ast.Bool _) 1)
            ((Ast.Name _) 2)
            (_            0)))
  (output (: 1 Int64)))

(case "print of an Ast.Str renders a quoted literal with escapes and read inverts it"
  (doc    "`print : Ast → String` renders an `Ast.Str` as a `\"…\"` literal, escaping the closed set
           (`\\n \\t \\r \\\\ \\\"`) — the canonical re-readable spelling — and `read : String → Ast`
           parses it back, so `read(print v) == v` (compiler-pipeline.md — printer and reader are inverse).
           The payload here holds an embedded quote and newline (`a\"b\\nc`), so this pins the escape
           round-trip, not just plain text — distinct from `Ast.Name`, which prints the bare word.")
  (input  (= (read (print (Ast.Str "a\"b\nc"))) (Ast.Str "a\"b\nc")))
  (output (: true Bool)))

; --- Ast.Str / cross-variant round-trip EDGES (pinning invariants so a change can't quietly flip them) ---
; The `Ast.Str` leaf round-trips through BOTH interchange paths (`print`/`read`, `Ast.encode`/`Ast.decode`)
; over the full payload range — empty, multibyte UTF-8, every escape, a keyword-colliding spelling — and a
; compound nesting ALL SIX leaf kinds round-trips too. These already hold; pinned here so a future change
; to the escape set, byte layout, or reader can't silently break a leaf (ast-encoding.md #The Encoding Is
; A Bijection; compiler-pipeline.md — printer/reader inverse).

(case "an empty-string Ast.Str round-trips through print and read"
  (doc    "The empty string is a valid `Ast.Str` payload — `print` renders `\"\"`, `read` parses it back.
           Pins the zero-length edge of the escape/quote rendering.")
  (input  (= (read (print (Ast.Str ""))) (Ast.Str "")))
  (output (: true Bool)))

(case "an empty-string Ast.Str round-trips through encode and decode"
  (doc    "The byte-path companion: an empty `Ast.Str` (length-prefix 0) encodes and decodes back equal
           (ast-encoding.md #The Encoding Is A Bijection).")
  (input  (match (Ast.decode (Ast.encode (Ast.Str "")))
            ((Ok a)  (= a (Ast.Str "")))
            ((Err _) false)))
  (output (: true Bool)))

(case "a multibyte-UTF-8 Ast.Str round-trips through encode and decode"
  (doc    "The byte-path companion of the multibyte print/read case: `\"héllo☃\"` (6 scalars, 10 UTF-8
           bytes) encodes and decodes back equal. The Str encoding is a length-prefix over the UTF-8 BYTES,
           so this pins that the prefix counts BYTES, not characters — every existing encode/decode Str case
           is ASCII (`\"\"`, `\"hi\"`, `\"x\"`) where byte-len == char-count and cannot distinguish the two.
           A codec that wrote a char-count length would pass those yet truncate or over-read this string.")
  (input  (match (Ast.decode (Ast.encode (Ast.Str "héllo☃")))
            ((Ok a)  (= a (Ast.Str "héllo☃")))
            ((Err _) false)))
  (output (: true Bool)))

(case "a multibyte-UTF-8 Ast.Str round-trips through print and read"
  (doc    "A string with non-ASCII scalars (`héllo☃` — 2- and 3-byte UTF-8) round-trips: the escape set
           touches only ASCII, so a multibyte scalar passes through and reads back intact. Pins the
           reader/printer are byte-faithful over UTF-8.")
  (input  (= (read (print (Ast.Str "héllo☃"))) (Ast.Str "héllo☃")))
  (output (: true Bool)))

(case "an all-escapes Ast.Str round-trips through print and read"
  (doc    "A payload with EVERY member of the closed escape set (`\\t \\r \\n \\\\ \\\"`) round-trips —
           each escaped on print, un-escaped on read. Pins the whole escape set at once, guarding against
           dropping or mis-pairing any one escape.")
  (input  (= (read (print (Ast.Str "\t\r\n\\\""))) (Ast.Str "\t\r\n\\\"")))
  (output (: true Bool)))

(case "a string spelled like a keyword round-trips as an Ast.Str, not an Ast.Bool or Ast.Name"
  (doc    "🔑 The disambiguation pin: the STRING `\"true\"` is an `Ast.Str`, not the boolean word or a
           name. `print` renders it QUOTED (`\"true\"`), so `read` parses it back as a string literal —
           never the `Ast.Bool` a bare `true` word reads as, nor an `Ast.Name`. Guards the print/read
           boundary between a quoted string and a bare keyword.")
  (input  (= (read (print (Ast.Str "true"))) (Ast.Str "true")))
  (output (: true Bool)))

(case "a deep compound nesting all six leaf kinds round-trips through encode and decode"
  (doc    "A compound nesting every realized leaf — `(Ast.List (Ast.Name \"f\") (Ast.Str \"x\") (Ast.Bool
           true) (Ast.Float 1.5) (Ast.List (Ast.Int 1)))` — round-trips through encode/decode to an equal
           value. Pins that Str/Bool/Float/Int/Name/List interleave correctly in one tree (each tag is
           self-delimiting), not just as standalone leaves.")
  (input  (match (Ast.decode (Ast.encode (Ast.List (list (Ast.Name "f") (Ast.Str "x") (Ast.Bool true) (Ast.Float 1.5) (Ast.List (list (Ast.Int 1)))))))
            ((Ok a)  (= a (Ast.List (list (Ast.Name "f") (Ast.Str "x") (Ast.Bool true) (Ast.Float 1.5) (Ast.List (list (Ast.Int 1)))))))
            ((Err _) false)))
  (output (: true Bool)))

; --- The Ast.Float leaf variant (completes the spec's Ast variant set) ----------------------------
; A FLOAT is the last of the six syntactic forms the `Ast` sum carries (type-system.md #The Abstract
; Syntax Tree Type Is An Ordinary Sum Type: "an integer, a FLOAT, a string, a boolean, a name, and a
; list"), so `(quote 1.5)` is `(Ast.Float 1.5)`. It carries a `Float64` payload, DISTINCT from `Ast.Int`
; (`(quote 3.0)` ≠ `(quote 3)`). The leaf constructs, destructures by match, round-trips through
; `Ast.encode`/`Ast.decode` (the 8-byte f64 bit pattern — a stable canonical form) and `print`/`read`
; (the shortest round-tripping decimal, always carrying a `.` so it re-reads as a float not an int), lifts
; through an active unquote (literal AND runtime), and `eval` executes it. With this variant the `Ast` sum
; realizes the COMPLETE spec set. (The behaviour landed source-first, gated by rcdzc unit tests; these
; corpus cases pin it at the spec level.)

(case "a quoted float equals the same node built by the Ast.Float constructor"
  (doc    "`(quote 1.5)` is the `Ast` sum value `(Ast.Float 1.5)` (metaprogramming.md #Quote Produces An
           AST Value; type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type — a float is a
           syntactic form). `(= (quote 1.5) (Ast.Float 1.5))` MUST be true, the float companion of the
           Int/Bool/Str equality cases.")
  (input  (= (quote 1.5) (Ast.Float 1.5)))
  (output (: true Bool)))

(case "a quoted float is distinct from the same magnitude quoted as an integer"
  (doc    "`Ast.Float` (a Float64 payload) and `Ast.Int` (an Int64 payload) are different variants:
           `(quote 3.0)` is `(Ast.Float 3.0)`, NOT `(Ast.Int 3)`, so comparing them is FALSE. Pins that a
           float literal is not collapsed to an integer — distinct syntactic forms with distinct payloads.")
  (input  (= (quote 3.0) (Ast.Int 3)))
  (output (: false Bool)))

(case "a match binds an Ast.Float payload"
  (doc    "The `Ast` sum is deconstructible by pattern matching, so a match over `(quote 2.5)` binds the
           `Ast.Float` payload — the Float64 — and comparing it to `2.5` is true. The catch-all covers the
           other variants.")
  (input  (match (quote 2.5)
            ((Ast.Float f) (= f 2.5))
            (_             false)))
  (output (: true Bool)))

(case "a built-in Ast.Float constructor applied to a wrong-type payload is a type error"
  (doc    "`Ast.Float`'s payload type is Float64, so `(Ast.Float \"x\")` applies it to a String — a type
           mismatch the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")`/`(Ast.Bool 5)` are.")
  (input  (Ast.Float "x"))
  (error  CDZ0201))

(case "eval of a quoted float executes it to the float value"
  (doc    "eval executes an AST value as code; a float form evaluates to itself, so `(eval (quote 1.5))`
           runs to `1.5` — the float companion of `(eval (quote true))`.")
  (input  (do (def (main) (eval (quote 1.5))) (export main)))
  (output (: 1.5 Float64)))

(case "encoding and decoding an Ast.Float round-trips to an equal value"
  (doc    "`(Ast.Float 1.5)` encodes (the f64 bit pattern) then decodes to an equal AST (ast-encoding.md
           #The Encoding Is A Bijection), as the Int/Bool/Str/Name/List round-trips do. `Ast.decode` is
           total, so the round-trip matches the `Ok` arm.")
  (input  (match (Ast.decode (Ast.encode (Ast.Float 1.5)))
            ((Ok a)  (= a (Ast.Float 1.5)))
            ((Err _) false)))
  (output (: true Bool)))

; --- The Float payload is the raw IEEE-754 f64 BIT PATTERN: sign and signed-zero survive --------------
; ast-encoding.md / lower.rs: an `Ast.Float f` encodes as tag 0x05 then the f64 BIT PATTERN as 8 bytes LE
; (`to_f64_bits`/`from_f64`), a canonical form where "equal doubles → equal bits; -0.0 ≠ 0.0". The round-
; trip case above uses only the positive `1.5`, so it never exercises the sign bit or the signed-zero
; distinction. A codec that normalized the bits (e.g. canonicalized -0.0 → 0.0, or dropped the sign) would
; pass `1.5` yet lose `-0.0`'s identity. These pin the BIT-EXACT contract: a negative float round-trips,
; and `-0.0` is byte-DISTINCT from `0.0` (they are `==` as floats but NOT bit-equal — the encoding keeps
; the difference the spec comment calls out). (NaN is not pinned here — `Float64.nan` is a field that does
; not fold in this const-codec position; its byte form is witnessed by the runtime-lift/print paths.)

(case "the Float codec round-trips a negative value"
  (doc    "`Ast.Float -2.5` encodes+decodes to an equal AST — the sign bit of the f64 payload survives the
           byte round-trip. The negative companion of the `Ast.Float 1.5` round-trip; a codec that mis-read
           the sign or the exponent bits would corrupt it.")
  (input  (match (Ast.decode (Ast.encode (Ast.Float -2.5)))
            ((Ok a)  (= a (Ast.Float -2.5)))
            ((Err _) false)))
  (output (: true Bool)))

(case "negative zero encodes to bytes distinct from positive zero"
  (doc    "`-0.0` and `0.0` are `==` as Float64 but have DISTINCT IEEE-754 bit patterns (only the sign bit
           differs), and the codec stores the raw bits — so `Ast.encode (Ast.Float -0.0)` ≠ `Ast.encode
           (Ast.Float 0.0)`. Pins the exact invariant the encoding comment calls out (\"-0.0 ≠ 0.0\"): a
           codec that canonicalized signed zero would collapse these to equal bytes and lose `-0.0`.")
  (input  (= (Ast.encode (Ast.Float -0.0)) (Ast.encode (Ast.Float 0.0))))
  (output (: false Bool)))

(case "negative zero round-trips through the codec by byte identity"
  (doc    "`Ast.Float -0.0` decodes to an AST that re-encodes to identical bytes — the sign bit of signed
           zero is preserved end-to-end (comparing by re-encoded bytes, since `-0.0 = 0.0` is true as a
           float value and would not distinguish them). Companion of the distinct-bytes case: pins that the
           round-trip, not just the initial encode, keeps signed zero.")
  (input  (match (Ast.decode (Ast.encode (Ast.Float -0.0)))
            ((Ok a)  (= (Ast.encode a) (Ast.encode (Ast.Float -0.0))))
            ((Err _) false)))
  (output (: true Bool)))

(case "print of an Ast.Float renders a re-readable decimal and read inverts it"
  (doc    "`print` renders an `Ast.Float` as the shortest round-tripping decimal — always carrying a `.`
           (or `e`) so it re-reads as a float — and `read` parses it back: `read(print v) == v`.")
  (input  (= (read (print (Ast.Float 1.5))) (Ast.Float 1.5)))
  (output (: true Bool)))

(case "print of an integer-valued Ast.Float keeps its float form through read"
  (doc    "🔑 The int-vs-float rendering pin: an integer-VALUED float `3.0` prints with an explicit `.0`
           (not the bare `3` an integer prints), so `read` parses it back as an `Ast.Float`, NOT an
           `Ast.Int`. `read(print (Ast.Float 3.0)) == (Ast.Float 3.0)`.")
  (input  (= (read (print (Ast.Float 3.0))) (Ast.Float 3.0)))
  (output (: true Bool)))

; --- `read` is TOTAL over its input: malformed text DECLINES, never traps or panics ------------------
; `read : String → Ast` parses the s-expression subset `print` emits (`lower_read`/`SexprReader`). The
; round-trip cases above only feed it WELL-FORMED text (`read(print v)`); none exercises the failure
; paths. `read` must be total the way the reader/lexer are "never panic" (syntax-vertical invariant) and
; the way `Ast.decode` is total over adversarial bytes — but `read` fails at COMPILE time (a constant-only
; fold), so a malformed input is a clean DECLINE (`Reject::decline`), not a runtime `Err` and never a
; trap/panic. These pin the three distinct decline arms in `lower_read`: text that is not a well-formed
; s-expression (the parser returns nothing), text with TRAILING content after the first node (the
; `at_end` check — a valid prefix must NOT be silently accepted), and an empty string. A reader change
; that panicked on unbalanced input, or that silently took the first node and dropped a trailing token,
; would break these. All → `(declines)`.

(case "read of text that is not a well-formed s-expression declines"
  (doc    "`(read \"(((\")` — unbalanced open parens are not a well-formed s-expression over the Ast
           subset, so `read` DECLINES (`lower_read`'s parse-failure arm) rather than trapping or fabricating
           a partial AST. Pins that the reader is total on malformed input — the `read` companion of the
           adversarial-bytes `Ast.decode` totality cases, and of the parser/lexer never-panic invariant.")
  (input  (read "((("))
  (declines))

(case "read of text with trailing content after the first s-expression declines"
  (doc    "`(read \"1 2\")` — a valid first node (`1`) FOLLOWED by more input (`2`). `read` must consume the
           WHOLE string (the `r.at_end()` check in `lower_read`), so trailing content declines rather than
           silently reading `1` and dropping `2`. The `read` parallel of the decode case where canonical
           bytes plus a trailing byte yield `Err`: a valid prefix is not a valid whole.")
  (input  (read "1 2"))
  (declines))

(case "read of the empty string declines"
  (doc    "`(read \"\")` — no s-expression at all. The empty string parses to no node, so `read` declines
           (never a trap or an empty/garbage AST). Pins the zero-input edge of the reader's totality.")
  (input  (read ""))
  (declines))

(case "an active unquote of a float literal lifts to an Ast.Float node"
  (doc    "`` `(f ,2.5) `` embeds the float literal `2.5` as the `Ast.Float` leaf its value denotes — the
           same node `(quote (f 2.5))` builds. The float companion of the literal Int/Bool/Str cases.")
  (input  (= (quasiquote (f (unquote 2.5)))
             (Ast.List (list (Ast.Name "f") (Ast.Float 2.5)))))
  (output (: true Bool)))

(case "an active unquote of a let-bound float lifts to Ast.Float by inferred type"
  (doc    "A RUNTIME float operand lifts by its inferred type: `x : Float64` → `Ast.Float`. `(let ((x 4.5))
           `(f ,x))` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Float 4.5)))` — the `ast-lift` path.")
  (input  (let ((x 4.5))
            (= (quasiquote (f (unquote x)))
               (Ast.List (list (Ast.Name "f") (Ast.Float 4.5))))))
  (output (: true Bool)))

(case "a quoted compound form equals the same AST built by the Ast.List constructor"
  (doc    "The list companion, and the sharpest case: `(quote (+ 1 2))` is
           `(Ast.List (list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2)))` — the very value form the FIRST
           case in this file records as `(quote (+ 1 2))`'s output. So comparing the quote against that
           hand-built Ast.List MUST be true (core-semantics.md #Equality Is Structural), because they
           are the same sum value. This equality is the compiler's own idiom: it builds an instruction
           AST by quasiquote and compares it against an expected AST built by constructors.")
  (input  (= (quote (+ 1 2)) (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))))
  (output (: true Bool)))

; The built-in `Ast` is an ordinary sum type — "a variant per syntactic form (an integer, a float, a
; string, a boolean, a name, and a list of child nodes)" (type-system.md #The Abstract Syntax Tree Type
; Is An Ordinary Sum Type) — so its constructors carry TYPED payloads: `Ast.Int` an Int64, `Ast.Name` a
; String, `Ast.List` a list of Ast. A constructor is a single-arity function whose argument is type-checked
; (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function + #Applying A Function Binds Its
; Parameter To Its Argument), so `(Ast.Int "x")` — a String where `Ast.Int`'s payload is Int64 — is a type
; mismatch the compiler MUST reject (CDZ0201), exactly as a user sum's `(T.Mk "x")` for `(type T (Mk
; Int64))` is. A compiler that checks a USER sum variant's payload type (that check landed) but not the
; BUILT-IN `Ast` constructors' declared payloads builds a malformed `(Ast.Int "x")` node: matching it binds
; the String where an Int64 is declared, and `(String.byte-len n)` reads it as a String and succeeds —
; running the ill-typed program. This is the built-in-Ast companion of the user-sum unary-variant
; payload-type case (05-compound-types.sexp): the Ast constructors are ordinary sum constructors and MUST
; type-check their payloads identically. A self-hosted front end that builds AST nodes with the Ast.*
; constructors depends on this — an unchecked `(Ast.Int "x")` is a malformed node it could emit. A
; generation that does not yet check the Ast constructors' payload types declines rather than building the
; mistyped node.

(case "a built-in Ast constructor applied to a wrong-type payload is a type error"
  (doc    "`Ast.Int`'s payload type is Int64 (the built-in `Ast` is an ordinary sum type, a variant per
           syntactic form — type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type), so
           `(Ast.Int \"x\")` applies it to a String — a type mismatch the compiler MUST reject (CDZ0201),
           exactly as a user sum's `(T.Mk \"x\")` for `(type T (Mk Int64))` is. Pins that the built-in
           `Ast` constructors type-check their declared payloads just as a user sum variant does — a
           compiler that checks user variants but not the built-in Ast constructors builds a malformed
           `(Ast.Int \"x\")` node (matching it binds the String where an Int64 is declared, and a
           downstream String use of the payload succeeds, running the ill-typed program). A self-hosted
           front end that constructs AST nodes with `Ast.*` depends on this check. A generation that does
           not yet check the Ast constructors' payload types declines rather than building the mistyped
           node.")
  (input  (Ast.Int "x"))
  (error  CDZ0201))

(case "quote-vs-constructor AST equality holds regardless of operand order"
  (doc    "The order-flipped companion: `(= (Ast.Int 42) (quote 42))` is the same comparison of one
           sum value against itself and MUST be true. Pins that neither operand order (constructor-built
           vs quote-built) is treated as a distinct type — structural equality is symmetric over the
           one AST value form.")
  (input  (= (Ast.Int 42) (quote 42)))
  (output (: true Bool)))

; --- Ast.encode / Ast.decode consume an AST built by the Ast.* constructors too ------------
; The encoding is a bijection over THE abstract syntax tree value (ast-encoding.md #The Encoding Is A
; Bijection With One Canonical Byte Form: "Decoding a canonical binary encoding MUST yield the abstract
; syntax tree it was encoded from"; "Two abstract syntax trees that are equal MUST have identical binary
; encodings"). Since an AST built by applying the Ast.* constructors is the same value form `quote`
; produces (the cases above), `Ast.encode` and `Ast.decode` must consume it exactly as they consume a
; quote-built AST — the round-trip is over AST VALUES, not over one privileged construction path. These
; are the encode/decode companions of the equality cases above: the same representation must reach the
; encoder however the AST was built. A quote-built AST (`(quote 7)`) and a constructor-built one
; (`(Ast.Int 7)`) are the ONE AST value form (the equality cases above), so `Ast.encode`/`Ast.decode`
; consume both identically — the round-trip is over AST VALUES, not one privileged construction path.
; A generation that has not yet bridged the two would decline the constructor-built operand rather than
; miscompiling (reject-don't-miscompile); the seed now bridges an applied `Ast.*` constructor to the AST
; value it denotes (via the constructor→node bridge), so these round-trip like the quote-built cases.

(case "encoding and decoding a constructor-built AST round-trips to an equal value"
  (doc    "`(Ast.Int 7)` is an AST value (the same form `(quote 7)` produces); encoding then decoding it
           MUST yield an equal AST (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t).
           The quote-built round-trip is witnessed earlier in this file; a constructor-built AST is the
           same value form, so it round-trips identically — the encoder bridges an applied `Ast.*`
           constructor to the AST value it denotes. `Ast.decode : Bytes → Result<Ast, _>` is total
           (value-interchange.md — decode of possibly-external bytes yields the error case, never traps),
           so the round-trip matches the `Ok` arm and equates its payload.")
  (input  (match (Ast.decode (Ast.encode (Ast.Int 7)))
            ((Ok a)  (= a (Ast.Int 7)))
            ((Err _) false)))
  (output (: true Bool)))

; --- The Int payload is a SIGNED two's-complement i64: negatives and the range boundary round-trip ---
; ast-encoding.md: an `Ast.Int n` encodes as tag 0x00 then `n` as 8 bytes little-endian TWO'S-COMPLEMENT
; i64 (`encode_ast_value`/`decode_ast_value` in lower.rs). The round-trip case above uses only the small
; positive `7`, which never exercises the sign bit or the full 8-byte width — so a decoder that mis-reads
; the payload as UNSIGNED, or a re-emit that sign-extends wrongly (the recurring hand-emitted-const class
; the house rules warn about), would pass `7` yet corrupt a negative or large value. These pin the SIGNED
; boundary: `i64::MIN` (-9223372036854775808 — the asymmetric two's-complement extreme, whose magnitude
; is not representable as a positive i64), and that `-1` and `1` encode to DISTINCT bytes (a decoder that
; drops the sign collapses them). A negative nested in a compound pins the same through the recursive
; encoder. Promoted from passing probes (breaker rule: pin the invariant so a future codec change can't
; quietly flip it).

(case "the Int codec round-trips i64::MIN — the two's-complement boundary"
  (doc    "`Ast.Int -9223372036854775808` (i64::MIN) encodes+decodes to an equal AST. This is the extreme
           of the signed 8-byte payload — its magnitude has no positive i64 representation, so a decoder
           that reads the bytes as unsigned, or negates during re-encode, corrupts it. The negative
           companion of the `Ast.Int 7` round-trip: pins the SIGNED two's-complement contract at its
           hardest value. `Ast.decode` is total, so the round-trip matches the `Ok` arm.")
  (input  (match (Ast.decode (Ast.encode (Ast.Int -9223372036854775808)))
            ((Ok a)  (= a (Ast.Int -9223372036854775808)))
            ((Err _) false)))
  (output (: true Bool)))

(case "a negative and its positive twin encode to distinct bytes"
  (doc    "`Ast.encode (Ast.Int -1)` ≠ `Ast.encode (Ast.Int 1)`: the sign is carried in the two's-
           complement byte form (all-ones vs a single low bit), so a codec that dropped or ignored the
           sign would collapse them to equal bytes. Pins that the encoding distinguishes sign — the
           byte-level companion of the i64::MIN round-trip.")
  (input  (= (Ast.encode (Ast.Int -1)) (Ast.encode (Ast.Int 1))))
  (output (: false Bool)))

(case "a negative integer nested in a compound round-trips by byte identity"
  (doc    "`(quote (f -42 \"s\"))` — a negative Int leaf beside a string, inside a list — decodes to an
           AST that re-encodes to identical bytes (the bijection's byte-identity face). Pins that the
           RECURSIVE encoder threads the signed payload through a compound, not only a bare leaf.")
  (input  (match (Ast.decode (Ast.encode (quote (f -42 "s"))))
            ((Ok a)  (= (Ast.encode a) (Ast.encode (quote (f -42 "s")))))
            ((Err _) false)))
  (output (: true Bool)))

(case "encoding and decoding a constructor-built compound AST round-trips"
  (doc    "The compound companion: a hand-built `(Ast.List (list (Ast.Name \"g\") (Ast.Int 5)))` MUST
           encode and decode back to an equal AST, exactly as a quote-built list does. Pins that the
           bijection round-trip reaches a constructor-built compound AST, not only a leaf node. `Ast.decode`
           is total (`Bytes → Result<Ast, _>`), so the round-trip matches the `Ok` arm.")
  (input  (match (Ast.decode (Ast.encode (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
            ((Ok a)  (= a (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
            ((Err _) false)))
  (output (: true Bool)))

(case "Ast.decode decodes bytes that arrive as a function argument"
  (doc    "`Ast.decode : Bytes → Result<Ast, _>` MUST decode bytes whatever their PROVENANCE — a literal,
           or (here) bytes passed in as a function ARGUMENT. The round-trip cases above decode a literal in
           tail position; this decodes `b`, a parameter of `dec`, which is how a program that reads its
           input decodes it (a compiler receives its program bytes as an argument, not a literal). The
           result is the same total `Result<Ast, _>`, so `(dec (Ast.encode (Ast.Int 42)))` matches the `Ok`
           arm and yields 42. A generation that realizes `Ast.decode` only over a compile-time-constant
           argument (folding it away) declines the runtime-argument form (\"unsupported dotted-application\")
           — but decode is an ordinary total operation on a runtime `Bytes` value, so it MUST run here.")
  (input  (do
            (def (main) (dec (Ast.encode (Ast.Int 42))))
            (def (dec b) (match (Ast.decode b)
                           ((Ok a)  (match a ((Ast.Int n) n) (other -1)))
                           ((Err _) -2))) (export main)))
  (output (: 42 Int64)))

(case "a quote-built and constructor-built AST of the same tree encode to identical bytes"
  (doc    "ast-encoding.md #The Encoding Is A Bijection With One Canonical Byte Form: \"Two abstract
           syntax trees that are equal MUST have identical binary encodings.\" `(quote 42)` and
           `(Ast.Int 42)` are the same AST (the equality cases above), so their encodings MUST be
           byte-identical. This is the encode-path witness of the one-canonical-byte-form requirement:
           the encoder must produce the same bytes for the one AST value however it was constructed. The
           seed declines the constructor-built operand, so it cannot yet witness the agreement.")
  (input  (= (Ast.encode (quote 42)) (Ast.encode (Ast.Int 42))))
  (output (: true Bool)))

(case "a quote-built and constructor-built FLOAT AST encode to identical bytes"
  (doc    "The float companion of the byte-identity case: `(quote 1.5)` and `(Ast.Float 1.5)` are the same
           AST value, so their encodings MUST be byte-identical (ast-encoding.md #The Encoding Is A
           Bijection With One Canonical Byte Form). Pins that the `Ast.Float` leaf's canonical bytes (the
           f64 bit pattern) are the same however the value is constructed.")
  (input  (= (Ast.encode (quote 1.5)) (Ast.encode (Ast.Float 1.5))))
  (output (: true Bool)))

(case "unquote-splicing splices list elements into parent"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,@<list-expr> evaluates <list-expr> to a list and splices its elements into the parent,
           not nested. `(+ ,@args) with args=(list 1 2 3) produces AST for (+ 1 2 3), not (+ (1 2 3)).")
  (input  (let ((args (list 1 2 3))) `(+ ,@args)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))) Ast)))

(case "splice flattens where unquote nests"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , nests the value; ,@ splices it. `(f ,x) embeds x as one element; `(f ,@x) with
           x=(list 1 2) splices to produce (f 1 2).")
  (input  (let ((x (list 1 2)))
            (= `(f ,@x) `(f 1 2))))
  (output (: true Bool)))

; --- Splicing requires a list --------------------------------------------------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation (witnessed above): `,@`
; "evaluates <list-expr> to a LIST and splices its elements into the parent." So splicing a value
; that is NOT a list — a scalar, a tuple, a string — has no elements to splice and is ill-typed:
; the compiler MUST reject it (CDZ0201) with the splice's non-list operand named — `unquote-splicing`
; is a recognized form, not an unbound name. A generation that does not yet check the splice operand's
; list type declines rather than running the program (reject-don't-miscompile).

(case "splicing a non-list value into a quasiquote is a type error"
  (doc    "`,@` splices the ELEMENTS of a list; splicing a non-list has no elements to splice.
           `(f ,@x)` with x bound to the Int64 `5` is ill-typed — the compiler MUST reject it
           (CDZ0201, metaprogramming.md: ,@ evaluates its operand to a LIST). A generation that does
           not yet check the splice operand's list type declines rather than running the program.")
  (input  (let ((x 5)) `(f ,@x)))
  (error  CDZ0201))

(case "splicing an integer literal directly is a type error"
  (doc    "The directly-written companion: `(unquote-splicing 5)` inside a quasiquote splices the
           literal `5`, which is not a list — a type error (CDZ0201). The rejection names the splice's
           non-list operand; `unquote-splicing`/`quasiquote` are recognized forms, not names, so this
           is not an unbound-name failure.")
  (input  (quasiquote ((unquote-splicing 5) 3)))
  (error  CDZ0201))

(case "quasiquote nests with inner unquote evaluated"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           quasiquote nests, so ``(+ ,,x) evaluates the inner , to produce `(+ ,<x-value>).
           With x=2, ``(+ ,,x) constructs an AST representing `(+ ,2).")
  (input  (let ((x 2)) ``(+ ,,x)))
  (output (: (Ast.List (list (Ast.Name "quasiquote")
                             (Ast.List (list (Ast.Name "+")
                                           (Ast.List (list (Ast.Name "unquote") (Ast.Int 2)))))))
             Ast)))

(case "nested quasiquote embeds a FLOAT via the inner unquote"
  (doc    "The float companion of nested-quasiquote: `` ``(+ ,,x) `` with x=2.5 evaluates the inner `,` and
           embeds the float, producing the AST of `` `(+ ,2.5) ``. The lifted value is an `(Ast.Float 2.5)`
           node inside the inert `unquote` structure. Pins that the active-unquote float lift composes with
           quasiquote NESTING (depth tracking) — the inner `,` fires at depth 1 as it does for an integer.")
  (input  (let ((x 2.5)) ``(+ ,,x)))
  (output (: (Ast.List (list (Ast.Name "quasiquote")
                             (Ast.List (list (Ast.Name "+")
                                           (Ast.List (list (Ast.Name "unquote") (Ast.Float 2.5)))))))
             Ast)))

(case "unquote outside quasiquote is a syntax error"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , and ,@ are only valid inside quasiquote context. Bare ,x is a syntax error — there's
           no quasiquote template to insert into — so the compiler rejects it at parse time (CDZ0003,
           the syntax-band unquote-outside-quasiquote code) rather than running the program.")
  (input  ,x)
  (error  CDZ0003))

; An `unquote` nested inside a PLAIN `quote` is still outside any quasiquote context — a `(quote …)`
; body is inert data, not a selective-evaluation template. metaprogramming.md #Quote Produces An AST
; Value: "(quote <expr>) MUST evaluate to an AST value representing the structure of <expr>, WITHOUT
; evaluating <expr> itself"; #Quasiquote Constructs AST With Selective Evaluation: "Unquote and
; unquote-splicing OUTSIDE a quasiquote context MUST be a syntax error." So `(quote (g ,x))` must NOT
; evaluate `,x` — it is exactly the "unquote outside quasiquote" the bare `,x` case above pins, one
; layer of quote deep, and MUST be rejected CDZ0003 (or preserved inert), NEVER evaluated. A compiler
; that treats the plain-quote nesting level as an active quasiquote level silently EVALUATES `,x` under
; `quote`, making `(quote (g ,x))` behave identically to the quasiquote `` `(g ,x) `` — a `quote` that
; is not inert, contradicting #Quote Produces An AST Value. (The companion arity leak: `(quote (unquote
; 1 2))` must be rejected CDZ0201 like `(quasiquote (unquote 1 2))`, not silently truncated to
; `(Ast.Int 1)` — the arity check the quasiquote path enforces applies under plain quote too.)

(case "an unquote nested inside a plain quote is a syntax error, not an active unquote"
  (doc    "`(quote (g ,x))` places an unquote inside a PLAIN quote — still outside any quasiquote
           context (a quote body is inert data, not a selective-evaluation template), so it is the same
           `,`-outside-quasiquote syntax error the bare `,x` case pins, rejected CDZ0003. metaprogramming.md
           #Quote Produces An AST Value forbids `quote` from evaluating its body; a compiler that treats
           plain quote as an active quasiquote level evaluates `,x` and makes `(quote (g ,x))` behave as
           the quasiquote `` `(g ,x) ``. The bug: the active-unquote test fires at quote's own nesting
           level rather than only inside a quasiquote (spec/learnings/2026-07-07-plain-quote-evaluated-a-nested-unquote-instead-of-treating-it-as-inert.md).")
  (input  (quote (g ,x)))
  (error  CDZ0003))

; `unquote` takes EXACTLY ONE operand — the expression to evaluate and embed. `(unquote 1 2)` supplies
; two, so it is malformed and the compiler MUST reject it (CDZ0201), never index just the first and
; drop the rest. The same arity check applies to an unquote encountered during quasiquote expansion as
; to one outside a quasiquote, so `` `(unquote 1 2) `` is rejected rather than silently truncated to
; `(Ast.Int 1)`. (Same class as over-applying a constructor `(Some 1 2)`, here for the `unquote` form
; inside a template.)

(case "unquote with more than one operand inside a quasiquote is malformed"
  (doc    "`(unquote 1 2)` inside a quasiquote gives `unquote` two operands where it takes exactly one —
           a malformed form the compiler MUST reject at compile time (CDZ0201) rather than silently
           take the first operand and drop the rest to yield `(Ast.Int 1)`. The same arity check
           applies during quasiquote expansion as outside a quasiquote.")
  (input  (quasiquote (unquote 1 2)))
  (error  CDZ0201))

(case "quasiquote makes AST construction readable"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Constructs AST Values Via Quasiquote:
           building an AST value via quasiquote is readable. Compare `(op-const ,n) vs
           (Ast.List (list (Ast.Name \"op-const\") n)) — quasiquote reads like the form it builds.
           This is the frontend/macro role quasiquote serves; the compiler's instruction backend
           instead uses a dedicated typed instruction sum built by ordinary constructors and matched
           to bytes (compiler-pipeline.md #The Compiler Operates On AST Values). Note: dotted names
           like i64.const expand to member access; hyphenated names avoid this.")
  (input  (let ((n 42)) `(op-const ,n)))
  (output (: (Ast.List (list (Ast.Name "op-const") (Ast.Int 42))) Ast)))

(case "Ast.decode converts bytes to an AST sum type value"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Operates On AST Values: the compiler receives
           a program as binary bytes and decodes it to an AST sum type value. `Ast.decode : Bytes →
           Result<Ast, _>` is total over possibly-external bytes (value-interchange.md — it never traps),
           so the compiler matches the `Ok` arm and then pattern-matches the AST within it.")
  (input  (match (Ast.decode (Ast.encode (quote 42)))
            ((Ok (Ast.Int n)) n)
            (_                0)))
  (output (: 42 Int64)))

(case "Ast.encode and Ast.decode round-trip"
  (doc    "Witnesses contracts/ast-encoding.md: encoding an AST to binary and decoding it back
           produces the same AST value. The compiler relies on this: it decodes the input, operates
           on AST values, and the encoding is faithful. `Ast.decode` is total (`Bytes → Result<Ast, _>`),
           so the round-trip matches the `Ok` arm and equates its payload to the original.")
  (input  (match (Ast.decode (Ast.encode (quote (+ 1 2))))
            ((Ok a)  (= a (quote (+ 1 2))))
            ((Err _) false)))
  (output (: true Bool)))

(case "decoding bytes that are not a canonical AST yields the error case, not a trap"
  (doc    "contracts/value-interchange.md #Decode Inverts Serialize And Refuses Otherwise + #A Decode Over
           External Bytes Is Total: `Ast.decode` consumes bytes that may come from an EXTERNAL source, so it
           MUST be total — a byte sequence that is not the canonical encoding of any AST yields the error
           case (`Err`), NOT a trap. `(Bytes.of (list 255 255 255))` is not a valid AST encoding, so the
           decode returns `Err` and the program handles it as an ordinary value. This is the fallible-reader
           discipline (like `String.from-bytes`), not reject-don't-miscompile: malformed EXTERNAL input is a
           handleable condition, not a program bug that traps.")
  (input  (match (Ast.decode (Bytes.of (list 255 255 255)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

(case "decoding canonical bytes followed by a trailing byte yields the error case"
  (doc    "contracts/deterministic-value-form.md #Decoding Is The Inverse Of The Canonical Byte Form: a byte
           sequence that is valid canonical bytes FOLLOWED BY additional bytes MUST NOT decode as the value
           the valid prefix encodes — the trailing byte is a detected error, not silently ignored. So
           `Ast.decode` of `(encode (Ast.Int 7)) ++ [99]` yields `Err`, not `Ok (Ast.Int 7)`. The total-decode
           companion of the round-trip cases: decode consumes the WHOLE input or reports an error, so a
           truncated or concatenated external input is caught rather than half-read.")
  (input  (match (Ast.decode (Bytes.concat (Ast.encode (Ast.Int 7)) (Bytes.of (list 99))))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

; Ast.decode stays TOTAL (Err, never a trap) on adversarial bytes that specifically exercise the FLOAT
; (tag 0x05, 8-byte f64) and STR (tag 0x04, len-prefixed UTF-8) decode arms this vertical added, plus an
; unknown tag. The generic non-canonical/trailing-byte cases above don't reach these arms; a change to the
; Float/Str decode that dropped a bounds/finite check would pass those yet regress here. All → Err.

(case "decode of a truncated Float tag yields the error case, not a trap"
  (doc    "The Float decode arm reads 8 bytes after tag 0x05; a truncated payload (tag + only 3 bytes) is
           not a canonical encoding, so `Ast.decode` returns `Err` (value-interchange.md #A Decode Over
           External Bytes Is Total). Pins the length check on the Float arm — never a partial read or trap.")
  (input  (match (Ast.decode (Bytes.of (list 5 1 2 3)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

(case "decode of a Float tag with a non-finite (NaN) bit pattern yields the error case"
  (doc    "A Float payload whose 8 bytes are a NaN bit pattern (`7ff8…0001`) has no finite `Decimal` value
           form, so the decode reports `Err` rather than fabricating a non-finite `Ast.Float` — the decode
           arm rejects a non-finite double. Pins that the byte→Decimal step stays total on NaN/inf.")
  (input  (match (Ast.decode (Bytes.of (list 5 1 0 0 0 0 0 248 127)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

(case "decode of a Str tag with an oversized length yields the error case"
  (doc    "The Str decode arm reads a 4-byte length then that many UTF-8 bytes; a length (255) exceeding
           the bytes present is not a canonical encoding, so `Ast.decode` returns `Err`. Pins the Str arm's
           bounds check — never reads past the input.")
  (input  (match (Ast.decode (Bytes.of (list 4 255 0 0 0)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

(case "decode of an unknown tag byte yields the error case"
  (doc    "A leading tag byte the encoding does not assign (0x09 — beyond Int/Name/List/Bool/Str/Float =
           0x00..0x05) is not a canonical AST, so `Ast.decode` returns `Err`. Pins that the tag dispatch's
           fallthrough is a clean decline, not a trap — total over ANY external byte.")
  (input  (match (Ast.decode (Bytes.of (list 9 0 0 0 0)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

; ============================================================================================
; Quote patterns — the quasiquote surface in PATTERN position (options/quote-patterns/)
; ============================================================================================
; The same `` ` ``/`,`/`,@` surface that CONSTRUCTS an AST value (above) serves the DUAL direction in
; pattern position: `` `<template> `` inside a `match` DESTRUCTURES an Ast scrutinee, reusing the
; constructor/pattern duality the language already has (a variant `(Some 5)` builds and `(Some n)`
; destructures). A quote pattern is EXACTLY EQUIVALENT to the pattern built from the `Ast.*` sum
; constructors — `` `(+ ,a ,b) `` IS `(Ast.List (list (Ast.Name "+") a b))` as a pattern — so it adds a
; surface, not a new value or a second matching mechanism. In the template: a literal subterm matches by
; equality (an integer as `(Ast.Int n)`, a bare name as `(Ast.Name "…")`), a compound `(h a b)` matches
; an `Ast.List` of EXACTLY that arity element-by-element, `,<pattern>` binds/further-matches the sub-AST
; at its position, and a FINAL `,@<name>` binds the remaining list elements. Exhaustiveness is the
; existing rule (a quote pattern never covers every AST — different head, different arity, a leaf where a
; list is expected all fail — so an Ast match needs a catch-all bare-name/`_` arm or it is CDZ0210), and
; equality/encoding are the constructor form's (a value matched through a quote pattern is matched through
; the very sum patterns the un-tagged cases above already run). NOTE the `,`/`,@` marks are meaningful
; only INSIDE a `` ` `` template — a top-level catch-all arm is an ordinary bare-name or `_` pattern, as
; a bare `,other` outside a quasiquote is the existing syntax error (the "unquote outside quasiquote"
; case above, CDZ0003), not a pattern.
;
; The seed ALREADY destructures AST sums by the `Ast.*` constructors — the un-tagged
; `(match (quote (+ 1 2)) ((Ast.List elems) …))` cases above run on it. The one NEW piece is the
; reader/lowering that recognizes a backtick in PATTERN position and desugars it to those constructor
; patterns. A later generation realizes that lowering
; (options/realized-capability-set/); the seed declines these — they pin the contract the realization
; must meet.

(case "a quote pattern binds an unquoted operand of a compound form"
  (doc    "`` `(+ ,a ,b) `` in pattern position IS `(Ast.List (list (Ast.Name \"+\") a b))` as a pattern
           (options/quote-patterns/quasiquote-pattern.md): the literal head `+` matches `(Ast.Name \"+\")`
           by equality, and `,a`/`,b` bind the two operand sub-ASTs. Matching `(quote (+ 3 5))` binds
           a=`(Ast.Int 3)` and b=`(Ast.Int 5)`; the arm returns b, so the AST for 5. Pins the core
           destructuring: unquote is the binder, a literal subterm matches by equality. The catch-all
           `other` is an ordinary bare-name pattern — `,` is meaningful only inside a `` ` `` template.")
  (input  (match (quote (+ 3 5))
            (`(+ ,a ,b) b)
            (other      other)))
  (output (: (Ast.Int 5) Ast)))

(case "a quote pattern is equivalent to the Ast.* constructor pattern"
  (doc    "A quote pattern lowers to the `Ast.*` constructor pattern, so the two spellings bind
           identically. `` `(+ ,a ,b) `` and `(Ast.List (list (Ast.Name \"+\") a b))` matched against the
           same `(quote (+ 1 2))` both bind a=`(Ast.Int 1)`; comparing the two bound values is true. Pins
           the equivalence the form rests on — the pattern adds a surface, not a second mechanism.")
  (input  (= (match (quote (+ 1 2)) (`(+ ,a ,b) a) (_ (Ast.Int 0)))
             (match (quote (+ 1 2)) ((Ast.List (list (Ast.Name "+") a b)) a) (_ (Ast.Int 0)))))
  (output (: true Bool)))

(case "a literal subterm in a quote pattern matches by equality"
  (doc    "A literal head/subterm matches by equality — the direct analogue of a literal value pattern
           `(match 2 (2 \"two\") …)`. `` `(+ ,a ,b) `` matches only a form headed by `+`; against
           `(quote (- 3 5))`, whose head is `-`, it does NOT match, so control falls to the `other`
           catch-all. Pins that the literal name in the template constrains the head, not merely the
           arity.")
  (input  (match (quote (- 3 5))
            (`(+ ,a ,b) 1)
            (other      0)))
  (output (: 0 Int64)))

(case "a quote pattern matches a fixed arity"
  (doc    "A compound template `` `(f ,a ,b) `` matches an `Ast.List` of EXACTLY three elements — the
           reading of `(Ast.List (list (Ast.Name \"f\") a b))`, whose `(list …)` sub-pattern fixes
           length. `(quote (f 1 2 3))` has four elements, so it does NOT match the two-operand pattern and
           falls to the catch-all. Pins fixed arity: variable length is expressed only through `,@`.")
  (input  (match (quote (f 1 2 3))
            (`(f ,a ,b) 2)
            (other      9)))
  (output (: 9 Int64)))

(case "a nested unquote pattern matches a sub-AST by shape"
  (doc    "`,<pattern>` nests an ordinary pattern at the sub-AST's position, so `` `(+ ,(Ast.Int n) ,b) ``
           matches only an addition whose first operand is an INTEGER LITERAL and binds its value to n.
           Against `(quote (+ 7 x))`, the first operand `(Ast.Int 7)` matches `(Ast.Int n)` binding n=7;
           the arm returns n. Pins that unquote takes a full pattern, not only a bare name.")
  (input  (match (quote (+ 7 x))
            (`(+ ,(Ast.Int n) ,b) n)
            (other                0)))
  (output (: 7 Int64)))

; A nested unquote pattern matches ANY Ast leaf variant, not just Int — the Float, Str, Bool, and Name
; variants (the leaves this vertical realized) destructure by shape exactly as `Ast.Int` does. These pin
; the interaction between the quote-pattern surface and those leaves: a `,(Ast.Float n)` matches only a
; float operand and binds its value; a `,(Ast.Str s)` matches only a string operand; a `,(Ast.Bool b)`
; matches only a boolean operand; a `,(Ast.Name n)` matches only an identifier operand and binds its
; spelling. A change to either the quote-pattern lowering or a leaf variant that broke this cross-feature
; match would flip these.

(case "a nested unquote pattern matches a Float sub-AST by shape"
  (doc    "`` `(f ,(Ast.Float n)) `` matches only a compound headed `f` whose operand is a FLOAT literal,
           binding its value. Against `(quote (f 2.5))` the operand `(Ast.Float 2.5)` matches `(Ast.Float
           n)` binding n=2.5, and `= n 2.5` is true. Pins that a quote pattern destructures the `Ast.Float`
           leaf (the float companion of the Int nested-unquote-pattern case above).")
  (input  (match (quote (f 2.5))
            (`(f ,(Ast.Float n)) (= n 2.5))
            (other               false)))
  (output (: true Bool)))

(case "a nested unquote pattern matches a Str sub-AST by shape"
  (doc    "The string companion: `` `(f ,(Ast.Str s)) `` matches only a compound headed `f` whose operand
           is a STRING literal, binding it. Against `(quote (f \"hi\"))` the operand `(Ast.Str \"hi\")`
           matches, and `String.byte-len s` is 2. Pins that a quote pattern destructures the `Ast.Str` leaf
           (distinct from `Ast.Name` — a string operand, not an identifier).")
  (input  (match (quote (f "hi"))
            (`(f ,(Ast.Str s)) (String.byte-len s))
            (other             0)))
  (output (: 2 Int64)))

(case "a nested unquote pattern matches a Bool sub-AST by shape"
  (doc    "The boolean companion, completing the leaf set: `` `(f ,(Ast.Bool b)) `` matches only a compound
           headed `f` whose operand is a BOOLEAN literal, binding it. Against `(quote (f true))` the operand
           `(Ast.Bool true)` matches `(Ast.Bool b)` binding b=true, so the arm returns true. Pins that a
           quote pattern destructures the `Ast.Bool` leaf exactly as it does Int/Float/Str — the last
           realized leaf in the nested-unquote-pattern family.")
  (input  (match (quote (f true))
            (`(f ,(Ast.Bool b)) b)
            (other              false)))
  (output (: true Bool)))

(case "a nested unquote Bool pattern does not match a non-boolean operand"
  (doc    "The discriminator companion: `` `(f ,(Ast.Bool b)) `` matches ONLY a boolean operand, so against
           `(quote (f 3))` — an INTEGER operand — the quote-pattern arm does NOT fire and control falls to
           the catch-all (→ 0). Pins that the nested-unquote leaf pattern is shape-SELECTIVE (a leaf pattern
           that matched any operand would wrongly bind the Int here), the negative face of the match cases.")
  (input  (match (quote (f 3))
            (`(f ,(Ast.Bool b)) 1)
            (other              0)))
  (output (: 0 Int64)))

(case "a nested unquote pattern binds an Ast.Name operand's identifier"
  (doc    "The Name companion, completing the leaf set (Int/Float/Str/Bool/Name): `` `(f ,(Ast.Name n)) ``
           matches only a compound headed `f` whose OPERAND is an identifier, binding its spelling to `n`.
           Against `(quote (f g))` the operand `(Ast.Name \"g\")` matches `(Ast.Name n)` binding n=\"g\", so
           `String.byte-len n` is 1. Distinct from the head-by-equality cases (which match a LITERAL name
           `(Ast.Name \"+\")`): here the unquote BINDS the operand name's string. Pins that a quote pattern
           destructures the `Ast.Name` leaf in operand position.")
  (input  (match (quote (f g))
            (`(f ,(Ast.Name n)) (String.byte-len n))
            (other              0)))
  (output (: 1 Int64)))

(case "a final unquote-splice binds the remaining elements as a list"
  (doc    "A final `,@<name>` binds the remaining list elements as a LIST (never a single element), the
           pattern-position dual of splicing construction. `` `(f ,@args) `` against `(quote (f 1 2 3))`
           binds args to the list `(Ast.Int 1) (Ast.Int 2) (Ast.Int 3)`; `List.len` of it is 3. Pins the
           tail splice binds the rest and that the elements are a list.")
  (input  (match (quote (f 1 2 3))
            (`(f ,@args) (List.len args))
            (other       0)))
  (output (: 3 Int64)))

(case "a quote pattern used to recognize a compiler form reads as that form"
  (doc    "The self-hosting payoff (options/quote-patterns/quasiquote-pattern.md #Why This Matters For
           Self-Hosting): the compiler's core is a `match` over the decoded AST, and a quote-pattern arm
           reads as the surface it lowers. Here a tiny `lower` distinguishes `(+ …)` from everything else
           by quote pattern; against `(quote (+ 4 6))` it selects the add arm and returns the first
           operand's node. Mirrors the construction idiom `` `(op-const ,n) `` on the pattern side.")
  (input  (match (quote (+ 4 6))
            (`(+ ,a ,b) a)
            (`(- ,a ,b) b)
            (other      other)))
  (output (: (Ast.Int 4) Ast)))

; An Ast match whose arms are only quote patterns does not cover the AST sum — a different head, a
; different arity, or a leaf scrutinee all fail to match — so it is non-exhaustive and rejected CDZ0210,
; exactly as a sum match missing a variant (core-semantics.md #Matching Is Exhaustive Or Rejected). A
; bare-name pattern (equivalently `_`) matches any AST and is the catch-all, so its ABSENCE is what makes
; the match non-exhaustive. Quote matching reuses exhaustiveness rather than adding a rule.

(case "a quote-pattern match with no catch-all is non-exhaustive"
  (doc    "`` `(+ ,a ,b) `` covers only additions; an Ast scrutinee can be a name, an integer, or a
           differently-headed list, none of which it matches. With no bare-name/`_` catch-all arm the
           match does not cover the AST sum and is rejected CDZ0210 — the same rejection a sum match
           missing a variant gets. Pins that quote matching reuses the existing exhaustiveness rule.")
  (input  (match (quote (+ 1 2))
            (`(+ ,a ,b) a)))
  (error  CDZ0210))

; `,@` binds the REST and so is only meaningful as the final element of its template: a `,@` before other
; elements would match a variable-length gap in the middle of a fixed sequence, turning a single
; positional scan into a search. That is an ill-formed quote pattern, rejected CDZ0221 (the CDZ02xx
; types-and-patterns band, the quote-pattern companion of the binary-form CDZ0220). Mirrors `bin`, where
; an unsized `(bytes rest)` is legal only as the final segment.

(case "a non-final unquote-splice in a quote pattern is ill-formed"
  (doc    "`,@<name>` binds the remaining elements, so it is meaningful only as the FINAL element of a
           template. `` `(f ,@init ,last) `` puts `,@init` before `,last`, requiring a variable-length gap
           flanked by a fixed tail — an ill-formed quote pattern, rejected CDZ0221
           (options/quote-patterns/quasiquote-pattern.md #Tail Splice Is Final-Position Only). Mirrors the
           binary-form rule that an unsized `bytes` segment is legal only last.")
  (input  (match (quote (f 1 2 3))
            (`(f ,@init ,last) last)
            (other             other)))
  (error  CDZ0221))

; --- eval drives CONTROL FLOW reified from quoted source (the Ast.Bool integration faces) ---------
; The Ast.Bool cases above pin the leaf (quote/match/eval/encode/print of a bare boolean); these pin
; the boolean leaf DOING ITS JOB inside evaluated control flow — an `if` whose condition arrives
; through the AST, both as a quoted literal and as a quoted comparison the evaluator must first
; reduce. A leaf realization that round-trips standalone but mis-tags inside a List ast (or an eval
; that reads the payload byte incorrectly) picks the wrong branch here.

(case "eval of a quoted conditional takes the branch its boolean literal selects"
  (doc    "`(eval (quote (if false 10 20)))` = 20: the quoted `if` reifies as a List ast whose condition
           element is an `Ast.Bool false` leaf; eval reconstructs the conditional and the false condition
           selects the else branch. Pins the Bool leaf composing INSIDE an evaluated compound — the
           branch-selection companion of the standalone `eval (quote true)` case above (an eval that
           mis-read the payload byte, or a quote that mis-tagged the leaf inside a List ast, answers 10).")
  (input  (do
            (def (main (: d Int64))
              (eval (quote (if false 10 20))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 20 Int64)))

(case "eval of a quoted conditional reduces its comparison condition first"
  (doc    "`(eval (quote (if (= 1 1) 7 8)))` = 7: the quoted condition is not a Bool LEAF but a
           comparison FORM the evaluator must reduce to a boolean before branching — the produced
           boolean exists only inside eval (no Ast.Bool node in the input tree; `(= 1 1)` reifies as a
           List ast). Pins that eval's boolean values and its branch dispatch agree end-to-end, not only
           when the boolean was quoted literally.")
  (input  (do
            (def (main (: d Int64))
              (eval (quote (if (= 1 1) 7 8))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 7 Int64)))

(case "a constructed Ast.Bool leaf drives an evaluated conditional"
  (doc    "`(if (eval (Ast.Bool true)) 5 6)` = 5: the Bool leaf is CONSTRUCTED (not quoted), evaluated
           to its payload, and the resulting runtime boolean drives an ORDINARY (non-reified) `if`.
           Closes the loop the constructor case above opens: a hand-built leaf's eval result is a
           first-class Bool usable in real control flow, not merely printable/encodable.")
  (input  (do
            (def (main (: d Int64))
              (if (eval (Ast.Bool true)) 5 6))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 5 Int64)))

(case "a quoted form's head reifies as a Name while its string argument is a Str"
  (doc    "`(quote (f \"s\"))` — ONE form carrying both String-payload leaf variants: the head `f` is an
           identifier reference (Ast.Name) and the argument a string literal (Ast.Str), same payload
           TYPE, different variants. The nested element match takes the Name arm for the head (→ 2) and
           a Str head-pattern does not fire (→ not 1). Pins the reifier keys the variant on the
           SYNTACTIC role, not the payload type — a quote that tags every string-payload leaf uniformly
           collapses call-heads and literals, and eval would then look up string literals as names.")
  (input  (do
            (def (main (: d Int64))
              (match (quote (f "s"))
                ((Ast.List (list (Ast.Str _) .. _)) 1)
                ((Ast.List (list (Ast.Name _) .. _)) 2)
                (_ 0)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 2 Int64)))

(case "three leaf variants in one quoted form each dispatch their own tag"
  (doc    "`(quote (\"s\" 5 true))` reifies a list whose three elements are DISTINCT leaf variants —
           Ast.Str, Ast.Int, Ast.Bool — bound by one list pattern and classified by a shared `kind`
           match: 1·100 + 2·10 + 3 = 123. The all-variants integration pin: each leaf realization was
           landed separately (Int/Name first, then Bool, then Str), and this case fails if ANY leaf's
           tag collides with another's inside a compound reification (a mis-tagged element shifts one
           digit of the answer, naming the culprit).")
  (input  (do
            (def (kind (: a Ast))
              (match a ((Ast.Str _) 1) ((Ast.Int _) 2) ((Ast.Bool _) 3) (_ 9)))
            (def (main (: d Int64))
              (match (quote ("s" 5 true))
                ((Ast.List (list a b c)) (+ (+ (* 100 (kind a)) (* 10 (kind b))) (kind c)))
                (_ -1)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 123 Int64)))

(case "a spliced boolean drives an evaluated conditional through the lifted leaf"
  (doc    "`(eval `(if ,false 10 20))` = 20 — the active unquote lifts `false` to an Ast.Bool inside
           the template, and eval's branch dispatch consumes that lifted leaf. The lift cases above
           pin node identity (unquote == quote); this pins the lifted node WORKING in eval'd control
           flow — a lift that built the right-looking node with a wrong payload byte answers 10.")
  (input  (do
            (def (main (: d Int64))
              (eval (quasiquote (if (unquote false) 10 20))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 20 Int64)))

(case "a spliced string participates in an evaluated equality"
  (doc    "`(eval `(if (= ,\"x\" \"x\") 7 8))` = 7 — the spliced Ast.Str leaf, reconstructed by eval,
           compares content-equal to the quoted literal it sits beside. The Str companion of the
           spliced-bool eval case: the lift must produce a leaf whose eval'd value round-trips into
           the ordinary string-equality path.")
  (input  (do
            (def (main (: d Int64))
              (eval (quasiquote (if (= (unquote "x") "x") 7 8))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 7 Int64)))

(case "mixed bool and int splices evaluate in one template"
  (doc    "`(eval `(if ,true (+ ,3 1) 0))` = 4 — two active unquotes of DIFFERENT value kinds (a Bool
           and an Int) lift in one template, and eval consumes both: the bool selects the branch, the
           int feeds the arithmetic. Pins the per-kind dispatch (`reify_active`) applying the right
           lift per operand within a single quasiquote, not latching one kind for the template.")
  (input  (do
            (def (main (: d Int64))
              (eval (quasiquote (if (unquote true) (+ (unquote 3) 1) 0))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 4 Int64)))

; --- The active-unquote lift is a RUNTIME operation, not only a constant fold ----------------------
; The `ast-lift` intrinsic behind an active `,e` is deliberately a RUNTIME lift (`lower::lower_ast_lift`
; — "the operand core need not be constant, which is the whole point over the literal-only reify"): it
; wraps the operand's VALUE in the `Ast` leaf its inferred type denotes, so a genuinely runtime scalar
; lifts too. Every eval-splice case ABOVE lifts a bare literal or a `(let ((x 3)) …)` const that the
; compiler folds at compile time, so none actually EXERCISES the runtime lift — a regression to a
; const-only reify (or a lift that mis-wraps a non-constant payload) would pass them all yet break real
; macro use. These pin the lift over a value that arrives at RUN TIME through the export boundary: the
; scalar reaches `ast-lift` as a live operand, is wrapped, reconstructed by eval, and computed on.
; `lower_ast_lift` has a per-type arm (Int64→Ast.Int, Bool→Ast.Bool, Float64→Ast.Float, String→Ast.Str);
; the Int/Bool/Float arms are pinned over a genuine runtime scalar here. The STRING arm is NOT pinned at
; run time because a `String` parameter can't cross the export boundary this harness calls through (a
; plain `String`-param export declines identically, so the decline is the boundary, not the lift) — its
; runtime lift stays witnessed by the literal/String-op cases above.

(case "an active unquote lifts a RUNTIME integer operand, not only a constant"
  (doc    "`(main n) = (eval `(+ ,n 1))` called with n=41 → 42. `n` is a runtime parameter (arrives via
           the `(call)`, so it is NOT compile-time-constant), and the active unquote lifts its live value
           through `ast-lift` — the runtime lift path (`lower_ast_lift`), distinct from the literal/let-
           const cases above which fold away before the runtime lift runs. Pins that the lift is a real
           runtime operation: a reversion to a constant-only reify declines this, and a lift that mis-
           wraps the non-constant Int64 payload computes garbage instead of 42.")
  (input  (do
            (def (main (: n Int64))
              (eval (quasiquote (+ (unquote n) 1))))
            (export main)))
  (call   main (: 41 Int64))
  (output (: 42 Int64)))

(case "an active unquote lifts a RUNTIME boolean operand, not only a constant"
  (doc    "The Bool arm of the runtime lift: `(main b) = (eval `(if ,b 10 20))` called with b=false → 20.
           `b` is a runtime parameter, so the `Ast.Bool` wrap is built on a NON-constant payload and eval's
           branch dispatch consumes the lifted leaf at run time. Companion of the runtime-int case: pins the
           `Bool→Ast.Bool` arm of `lower_ast_lift` over a live operand (a const-only reify declines it; a
           mis-wrapped bool payload selects the wrong branch and answers 10).")
  (input  (do
            (def (main (: b Bool))
              (eval (quasiquote (if (unquote b) 10 20))))
            (export main)))
  (call   main (: false Bool))
  (output (: 20 Int64)))

(case "an active unquote lifts a RUNTIME float operand, not only a constant"
  (doc    "The Float arm of the runtime lift: `(main x) = (eval `(+ ,x 1.5))` called with x=2.5 → 4.0. `x`
           is a runtime Float64 parameter, so the `Ast.Float` wrap carries a NON-constant payload that
           eval reconstructs into ordinary float arithmetic. Companion of the runtime-int case for the
           `Float64→Ast.Float` arm (a payload mis-read as the i64 bit pattern computes garbage, not 4.0).")
  (input  (do
            (def (main (: x Float64))
              (eval (quasiquote (+ (unquote x) 1.5))))
            (export main)))
  (call   main (: 2.5 Float64))
  (output (: 4.0 Float64)))

; --- `quote` is a grammar head in EXPRESSION position, not a reserved DEFINITION name -------------
; `quote`/`quasiquote` are grammar forms the resolver dispatches STRUCTURALLY only when they head an
; EXPRESSION — exactly as `if`/`match`/`bin`, all of which are freely definable as ordinary function
; names because a definition's SIGNATURE is never resolved as an expression. A user may therefore
; `def quote(x) = x + 2`; its signature is spelled `(quote x)`, a `(quote …)`-headed list that is a
; BINDING form, not a quote. The regression this pins: quote REIFICATION is a shape-driven pre-pass
; over every `(quote …)`/`(quasiquote …)` node, and it wrongly rewrote the def signature `(quote x)`
; into `(Ast.Name "x")`, erasing the parameter binder — the body's `x` then resolved CDZ0101
; "unbound name". The fix excludes a def-signature / fn-params list from reification (see
; `quote::binder_position_nodes`), so the def scans as an ordinary function named `quote` whose
; parameter binds. A bare reference to it (as a higher-order value here) reaches the def and computes
; `quote(5) = 7`; a genuine `(quote …)` in EXPRESSION position (the cases above) still reifies.
(case "a user function may be named quote — its signature is a binding form, not a quote"
  (doc    "Witnesses that `quote`/`quasiquote` are grammar heads in EXPRESSION position only, not
           reserved definition names — like `if`/`match`, a user may `def quote(x) = x + 2`. The
           def signature `(quote x)` MUST NOT be reified by the quote pre-pass (which would erase the
           parameter binder and report the body's `x` as CDZ0101 unbound); it scans as an ordinary
           function named `quote`. Referenced as a first-class value through `apply1`, it computes
           `quote(5) = 7`.")
  (input  (do
            (def (quote (: x Int64)) (+ x 2))
            (def (apply1 (: f (-> Int64 Int64)) (: n Int64)) (f n))
            (def (main (: d Int64)) (apply1 quote 5))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 7 Int64)))

(case "eval of a quoted float feeds ordinary float arithmetic"
  (doc    "`(* (eval (quote 2.5)) 2.0)` = 5.0 — the reconstructed float literal is a first-class
           Float64 in downstream arithmetic (a payload mis-read as the i64 bit pattern computes
           garbage). The arithmetic-consumption companion of the eval-to-value case above.")
  (input  (do
            (def (main (: d Int64))
              (* (eval (quote 2.5)) 2.0))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 5.0 Float64)))

(case "all four leaf kinds in one quoted form dispatch their own tags"
  (doc    "`(quote (\"s\" 5 true 2.5))` — Str, Int, Bool, and Float leaves in ONE reified list, each
           classified by a shared match: 1·1000 + 2·100 + 3·10 + 4 = 1234. The full-leaf-set
           integration pin: any mis-tagged element shifts one digit, naming the culprit.")
  (input  (do
            (def (kind (: a Ast))
              (match a ((Ast.Str _) 1) ((Ast.Int _) 2) ((Ast.Bool _) 3) ((Ast.Float _) 4) (_ 9)))
            (def (main (: d Int64))
              (match (quote ("s" 5 true 2.5))
                ((Ast.List (list a b c e)) (+ (+ (+ (* 1000 (kind a)) (* 100 (kind b))) (* 10 (kind c))) (kind e)))
                (_ -1)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1234 Int64)))

; --- Codec bijection over rich trees: the composition faces of encode/decode -----------------------
; The per-leaf round-trips and the adversarial-bytes totality pins grade single nodes; these grade
; the BIJECTION contract (ast-encoding.md — one canonical byte form) over structurally rich trees,
; promoted from passing breaker probes after the non-minimal-varint reject (codec bijection).

(case "a deep four-leaf tree round-trips through encode and decode"
  (doc    "`(quote (f (g 1 true) \"s\" 2.5))` — three nesting levels carrying all four leaf kinds —
           encodes and decodes to an EQUAL tree. The composition face of the per-leaf round-trips:
           length-prefixed lists nest, and each leaf's payload survives inside the compound framing
           (a framing error corrupts everything after the first nested list).")
  (input  (match (Ast.decode (Ast.encode (quote (f (g 1 true) "s" 2.5))))
            ((Ok a) (= a (quote (f (g 1 true) "s" 2.5))))
            ((Err _) false)))
  (output (: true Bool)))

(case "quote-built and constructor-built equal trees encode byte-identically"
  (doc    "`(quote (f 1))` and `(Ast.List (list (Ast.Name \"f\") (Ast.Int 1)))` are ONE value built
           two ways; the bijection contract (one canonical byte form per tree) means their encodings
           are byte-EQUAL, not merely decode-equivalent. A codec with construction-dependent framing
           (or the non-minimal varints just rejected) breaks exactly this equality.")
  (input  (= (Ast.encode (quote (f 1)))
             (Ast.encode (Ast.List (list (Ast.Name "f") (Ast.Int 1))))))
  (output (: true Bool)))

(case "one encode-decode cycle is byte-stable"
  (doc    "encode(decode(encode t)) = encode(t) — the decoded tree re-encodes to the SAME bytes (the
           bijection composed both directions). Catches a decoder that normalizes or a codec pair
           that round-trips values while drifting bytes (legal under decode-equality, illegal under
           the canonical-byte-form contract).")
  (input  (match (Ast.decode (Ast.encode (quote (f (g 1 true) "s" 2.5))))
            ((Ok a) (= (Ast.encode a) (Ast.encode (quote (f (g 1 true) "s" 2.5)))))
            ((Err _) false)))
  (output (: true Bool)))

(case "a runtime-assembled tree round-trips equal to its quoted twin"
  (doc    "`` `(f ,(+ 1 2)) `` — the tree is ASSEMBLED at run time (an active unquote splicing a
           computed 3) — encodes/decodes equal to the statically-quoted `(quote (f 3))`. Pins the
           codec over a runtime-built tree (the constant cases could fold end-to-end; a splice's
           lifted leaf must serialize identically to a quoted one).")
  (input  (match (Ast.decode (Ast.encode (quasiquote (f (unquote (+ 1 2))))))
            ((Ok a) (= a (quote (f 3))))
            ((Err _) false)))
  (output (: true Bool)))

; --- Ast-valued unquote splicing: the operand-binding faces -----------------------------------------
; The ast-lift intrinsic splices a COMPUTED Ast subtree into a quasiquote template (the RESOLVED
; splice gap; its pin covers a param-bound operand matched structurally). These pin the other
; operand bindings and the identity contract, promoted from passing breaker probes.

(case "a let-bound Ast splices into a template"
  (doc    "`(let ((sub (quote (* 2 3)))) `(+ ,sub 1))` — the spliced operand is a LET binding (the
           resolved pin covers a param). The grafted template is a 3-element list. Pins the splice
           over a local binding (an ast-lift keyed to param slots misses the local).")
  (input  (do
            (def (main (: d Int64))
              (let ((sub (quote (* 2 3))))
                (match (quasiquote (+ (unquote sub) 1)) ((Ast.List es) (List.len es)) (_ -1))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 3 Int64)))

(case "a grafted template is structurally equal to the directly-quoted tree"
  (doc    "`` `(+ ,(quote (* 2 3)) 1) `` = `(quote (+ (* 2 3) 1))` — the identity contract of the
           splice: inserting an Ast RESULT means grafting the node AS-IS, so the assembled tree is
           byte-for-byte the tree the plain quote builds (structural equality over the two). A
           re-wrapping splice (the old Ast.Int(...) coercion) or a copy that perturbs the subtree
           breaks the equality.")
  (input  (= (quasiquote (+ (unquote (quote (* 2 3))) 1))
             (quote (+ (* 2 3) 1))))
  (output (: true Bool)))

; Metaprogramming — quote/quasiquote and the AST as a sum type. Witnesses metaprogramming.md.
; Quote produces an AST value without evaluating; quasiquote allows selective evaluation for
; construction. The AST is a sum type deconstructible by pattern matching, so the compiler
; operates on AST values natively rather than using string-tagged reflection. Eval (executing
; AST as code) is optional for macros/REPL, not needed by the core compiler.
(case
  "quote produces an AST value without evaluating"
  (doc
    "Witnesses metaprogramming.md #Quote Produces An AST Value: (quote <expr>) returns an
           AST sum type value representing <expr>'s structure, without evaluating <expr>.
           (quote (+ 1 2)) produces an AST value, not 3.")
  (input (quote (+ 1 2)))
  (output (: (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2))) Ast)))

(case
  "each literal kind quotes to its own single Ast leaf that escapes and renders"
  (doc
    "The leaf-level companion of the compound case above: quoting a BARE name/string/boolean/float
           produces the matching single `Ast` variant, and each escapes the boundary rendering its
           canonical constructor form. A tuple gathers all four so one case pins the whole scalar-leaf
           set: `(tuple (quote foo) (quote \"hi\") (quote true) (quote 2.5))` = `(tuple (Ast.Name \"foo\")
           (Ast.Str \"hi\") (Ast.Bool true) (Ast.Float 2.5))`. Pins that a NAME quotes to `Ast.Name` (not
           evaluated — the identifier is data), a STRING to `Ast.Str` (distinct from `Ast.Name`), a
           BOOLEAN to `Ast.Bool`, a FLOAT to `Ast.Float` (distinct from `Ast.Int`), and that each single
           leaf VALUE crosses the boundary and reads back structurally — the value-face of the guide's
           opening `(quote 42)` / `(quote false)` examples (`Ast.Int` is already pinned standalone
           elsewhere). A compiler that folded a quote or mis-tagged a leaf variant would render a wrong
           node here.")
  (input #tuple((quote foo) (quote "hi") (quote true) (quote 2.5)))
  (output
    (:
      #tuple((Ast.Name "foo") (Ast.Str "hi") (Ast.Bool true) (Ast.Float 2.5))
      (Tuple Ast Ast Ast Ast))))

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
(case
  "a plain quote does not evaluate a quasiquote's unquote nested inside it"
  (doc
    "Witnesses metaprogramming.md #Quote Produces An AST Value (\"without evaluating <expr>\").
           `(quote `(+ ,x))` and `(quote `(+ ,y))` with x and y both bound to 1 quote two templates
           that mention DIFFERENT names (`x` vs `y`); a plain quote does not evaluate the nested
           `,x`/`,y`, so the two quoted structures differ and `=` is FALSE. A compiler that evaluates
           the nested unquote (embedding x's value 1 and y's value 1) collapses both to the AST of
           `(+ 1)` and wrongly answers true — it treated the quoted quasiquote as an active one,
           evaluating inside a plain quote. Companion (rejection side) below: a bare stray unquote
           under a plain quote is CDZ0003.")
  (input
    (let
      ((x 1))
      (let ((y 1)) (= (quote (quasiquote (+ (unquote x)))) (quote (quasiquote (+ (unquote y))))))))
  (output (: false Bool)))

; --- Nested quasiquote LEVEL arithmetic ----------------------------------------------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation: an unquote is active only
; when it is NOT wrapped by an intervening (inner) quasiquote — a quasiquote INCREMENTS the level and
; an unquote DECREMENTS it, and only an unquote that brings the level back to zero (the outermost
; quasiquote) actually SPLICES; a deeper one is preserved as structure. This is standard Lisp-family
; quasiquote nesting: `` `(a `(b ,x)) `` builds the AST of `(a `(b ,x))` in which the inner `,x` is
; still literal `(unquote x)` structure (level 1, deferred), NOT x's value. The outermost quasiquote
; is level-1-active: an unquote directly under it (not under an inner quasiquote) DOES splice. And a
; DOUBLE unquote `,,x` under one inner quasiquote decrements twice — back to active — so it splices
; the ENCLOSING value while remaining wrapped in a deferred `(unquote …)` node. The four cases below
; pin the whole level machine with spelling-independent discriminators (bind the same name to two
; DIFFERENT values and compare) plus two exact-structure renders, so a compiler that mis-counts the
; level (splices a deferred inner unquote, or fails to splice an active one / a double unquote) is
; caught. This is the constructive dual of the "plain quote is inert" case just above — there the
; whole form is quoted; here the OUTER form is an evaluated quasiquote and only the level decides.
(case
  "nested quasiquote: an inner unquote at level 2 is deferred, not spliced (exact structure)"
  (doc
    "`(let ((x 7)) `(a `(b ,x)))` builds the AST of `(a `(b ,x))`: the inner quasiquote raises the
           level, so `,x` is left as literal `(unquote x)` structure — the NAME `x`, not the value 7.
           Renders the exact deferred shape. A compiler that spliced the level-2 unquote would embed
           `(Ast.Int 7)` where `(Ast.Name \"x\")` must stand.")
  (input (let ((x 7)) (quasiquote (a (quasiquote (b (unquote x)))))))
  (output
    (:
      (Ast.List
        #list((Ast.Name "a")
          (Ast.List
            #list((Ast.Name "quasiquote")
              (Ast.List #list((Ast.Name "b") (Ast.List #list((Ast.Name "unquote") (Ast.Name "x")))))))))
      Ast)))

(case
  "nested quasiquote: a level-2 deferred unquote is INDEPENDENT of the enclosing binding"
  (doc
    "The spelling-independent discriminator for the render above: bind x to 1 vs 99 and quasiquote
           `(a `(b ,x))` each. Because the inner `,x` is deferred (level 2, not spliced), both build the
           SAME structure — mentioning the name `x`, never the value — so `=` is TRUE despite the differing
           bindings. A compiler that spliced the inner unquote would embed 1 and 99 and wrongly answer FALSE.")
  (input
    (=
      (let ((x 1)) (quasiquote (a (quasiquote (b (unquote x))))))
      (let ((x 99)) (quasiquote (a (quasiquote (b (unquote x))))))))
  (output (: true Bool)))

(case
  "nested quasiquote: a level-1 unquote splices even while a level-2 one is deferred (same template)"
  (doc
    "The companion that proves the OUTER quasiquote is active while the inner is deferred, in ONE
           template: `` `(a ,x `(b ,x)) `` — the first `,x` sits directly under the outer quasiquote (level 1,
           SPLICES the value) and the second is under the inner quasiquote (level 2, DEFERRED). Bind x to 1
           vs 99: the deferred copy is identical, but the spliced copy differs (1 vs 99), so `=` is FALSE.
           A compiler that failed to splice the level-1 unquote would make both structures equal (both mention
           the name) and wrongly answer TRUE; one that spliced the level-2 unquote too would also differ but
           for the wrong reason — the level-1/level-2 split is what this pins.")
  (input
    (=
      (let ((x 1)) (quasiquote (a (unquote x) (quasiquote (b (unquote x))))))
      (let ((x 99)) (quasiquote (a (unquote x) (quasiquote (b (unquote x))))))))
  (output (: false Bool)))

(case
  "nested quasiquote: a double unquote at level 2 decrements back to active and splices (exact structure)"
  (doc
    "`(let ((x 7)) `(a `(b ,,x)))`: the inner quasiquote raises the level to 2, and the DOUBLE
           unquote `,,x` decrements twice — back to active — so it splices the enclosing value 7 while the
           result stays wrapped in one deferred `(unquote …)` node. Renders the exact shape: the inner leaf
           is `(unquote 7)` (value spliced, still one level deferred), NOT `(unquote (unquote x))` and NOT a
           bare 7. A compiler that mis-counted the double unquote would leave the name unspliced or collapse
           the deferred wrapper.")
  (input (let ((x 7)) (quasiquote (a (quasiquote (b (unquote (unquote x))))))))
  (output
    (:
      (Ast.List
        #list((Ast.Name "a")
          (Ast.List
            #list((Ast.Name "quasiquote")
              (Ast.List #list((Ast.Name "b") (Ast.List #list((Ast.Name "unquote") (Ast.Int 7)))))))))
      Ast)))

(case
  "eval is optional for macros and interactive use"
  (doc
    "Witnesses metaprogramming.md #Eval Is Optional For Macros And Interactive Use: (eval <ast>)
           executes AST as code, optional for macros/REPL. Seed provides it; static generations need
           not. (eval (quote (+ 1 2))) produces 3.")
  (input (eval (quote (+ 1 2))))
  (output (: 3 Int64)))

(case
  "eval of a bare quoted integer executes it to the integer value"
  (doc
    "The Int companion of the bare-literal eval cases (bool/string/float have one; Int did not):
           `(eval (quote 42))` = 42 — eval of an already-fully-reduced integer leaf is the integer itself,
           the base case beneath the `(eval (quote (+ 1 2)))` arithmetic case above. Pins that eval's leaf
           dispatch handles a bare `Ast.Int` node, not only an operator-headed form.")
  (input (eval (quote 42)))
  (output (: 42 Int64)))

; `eval` needs a COMPILE-TIME-VISIBLE AST (a `(quote …)`), so `(eval q)` on a RUNTIME-visible value `q` (a
; def parameter) declines CDZ0101 with the teaching text naming the compile-time-AST requirement — NOT a bare
; "unbound name `eval`" (which misdirects toward imports). The teaching text is the SAME in every position:
; a match SCRUTINEE, a let INIT, an arithmetic operand (a prior bug gave the match-scrutinee position a bare
; unbound while the others taught). Exactly ONE diagnostic (a bare unbound copy at the same node is deduped).
; A genuinely-unbound plain name still reports its own plain unbound, no teaching. (Migrated from rcdzc
; a_match_scrutinee_eval_gives_the_teaching_message_not_a_bare_unbound.)
(case
  "eval on a runtime value in a match scrutinee gives the compile-time-AST teaching message"
  (input (do (def (main (: q Int64)) (match (eval q) (_ 0))) (export main)))
  (error CDZ0101 (message "COMPILE-TIME-VISIBLE AST") (count 1)))

(case
  "eval on a runtime value in a let init gives the compile-time-AST teaching message"
  (input (do (def (main (: q Int64)) (let ((r (eval q))) 0)) (export main)))
  (error CDZ0101 (message "COMPILE-TIME-VISIBLE AST")))

(case
  "eval on a runtime value in an arithmetic operand gives the compile-time-AST teaching message"
  (input (do (def (main (: q Int64)) (+ (eval q) 1)) (export main)))
  (error CDZ0101 (message "COMPILE-TIME-VISIBLE AST")))

(case
  "a genuinely unbound plain name reports its own plain unbound, not the eval teaching message"
  (input (do (def (main) nosuchname) (export main)))
  (error CDZ0101 (message "unbound") (not "COMPILE-TIME-VISIBLE AST")))

(case
  "eval on a RUNTIME Ast parameter gives the compile-time-AST teaching message — the requirement is compile-time VISIBILITY, not Ast-ness"
  (doc
    "The runtime-Ast face of the teaching message (migrated from rcdzc
           eval_of_a_non_compile_time_ast_names_the_form_not_an_unbound_eval): `(eval a)` where `a` is a
           RUNTIME `Ast` parameter — not a non-Ast scalar like the Int64 cases above — still declines
           CDZ0101 with the COMPILE-TIME-VISIBLE AST teaching text. `eval` desugars only a compile-time-
           visible AST (a `(quote …)` / literal `Ast.*`), so a runtime Ast has nothing to reconstruct; the
           requirement is compile-time VISIBILITY, not merely being an Ast value.")
  (input (do (def (f (: a Ast)) (eval a)) (export f)))
  (error CDZ0101 (message "COMPILE-TIME-VISIBLE AST")))

(case
  "a near-eval typo keeps the ordinary unbound did-you-mean to a near def, not the eval teaching message"
  (doc
    "The over-reach guard for the eval teaching path (migrated from the same rcdzc test): a bare
           `eval`-shaped typo that is NOT an `(eval …)` head — `(evel)`, distance-1 from both the form
           `eval` AND a user def `evil` — still gets the ORDINARY unbound-name did-you-mean pointing at the
           near def `evil`, NOT the eval-form COMPILE-TIME-VISIBLE AST message. Pins that the eval teaching
           text fires only for a genuine `(eval …)` head, and does not hijack an ordinary near-name typo.")
  (input (do (def (evil) 5) (def (main) (evel)) (export main)))
  (error CDZ0101 (message "did you mean `evil`?") (not "COMPILE-TIME-VISIBLE AST")))

(case
  "eval of a quoted RECURSIVE-sum construction builds the heap spine"
  (doc
    "Eval over a USER-declared recursive sum: `(eval (quote (S (S (Z)))))` must resolve the quoted
           constructor NAMES against the user's `(type Nat (Z) (S Nat))`, build the two-level heap spine at
           run time, and hand it to ordinary code — the recursive `depth` fold reads back 2. Pins that eval
           reaches user-sum constructors (not only built-in operators/literals) and that its result is a
           first-class recursive heap value indistinguishable from a directly-constructed one.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (depth (: v Nat)) (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
      (def (main) (depth (eval (quote (S (S (Z)))))))
      (export main)))
  (call main)
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a bare Eval.in-caller perform in NON-macro code has no home — the compile-time Eval effect is discharged only at a macro expansion, so an unfolded perform is a CDZ0401, never a silent miscompile"
  (doc
    "Witnesses the COMPILE-TIME Eval effect's discharge contract (metaprogramming.md / DESIGN-macro-system.md
           §3): `(effect Eval (op in-caller (-> Ast Ast)))` is a prelude-injected compile-time effect a MACRO
           carries in its written row; `(Eval.in-caller ast)` evaluates an Ast in the caller's env and is
           DISCHARGED (folded away, ERASING the effect) by the macro EXPANDER at a macro CALL SITE's
           reconstructed expansion — so a correctly-used in-caller never survives to the effect system. This
           pins the DUAL guarantee for a bare `(Eval.in-caller …)` written in ORDINARY (non-macro) code, which
           the expander never folds: (1) `Eval`/`in-caller` RESOLVE (the effect is prelude-injected into every
           `(do …)`/`(module …)` program — NOT a misleading `unbound name Eval`), and (2) the un-erased perform
           reaches the no-home check and is a clean CDZ0401 ('neither an enclosing handler nor a host
           delegation, so it has no home'), NEVER a silent miscompile. This is the guard that keeps the
           compile-time-effect erasure honest: if the expander's fold ever fails to erase an in-caller, the
           effect system catches it here. (A LITERAL in-caller INSIDE a macro folds → runs; see 31-macros.)")
  (input
    (do
      (def (main) (Eval.in-caller (quote 1)))
      (export main)))
  (error CDZ0401 (message "neither an enclosing handler nor a host delegation")))

(case
  "an Ast.Int carries a BEYOND-64-bit literal losslessly through quote"
  (doc
    "The lossless-storage acceptance witness of the Ast.Int Int64→BigInt flip: a 26-digit literal
           rides `quote` to an `(Ast.Int b)` bind whose payload equals the exact annotated BigInt — no
           truncation, no wrap, no float detour. The flip's Part-1 contract (STORAGE is lossless; eval/
           print of huge leaves are later increments).")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (quote 99999999999999999999999999)
          ((Ast.Int b) (if (= b (: 99999999999999999999999999 BigInt)) 1 0))
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a constructed huge Ast.Int equals its quoted twin"
  (doc
    "Construction-path irrelevance at the beyond-64-bit width: `(Ast.Int (: <26 digits> BigInt))`
           built by the constructor equals the `quote`-built twin — the two construction routes meet in
           ONE value form at a magnitude no Int64 payload could carry. The huge companion of the
           runtime-constructed-equals-quoted pin.")
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (= (Ast.Int (: 99999999999999999999999999 BigInt)) (quote 99999999999999999999999999))
          1
          0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "every Ast variant deconstructs to its own arm through a qualified (. Ast Ctor) pattern"
  (doc
    "First-user fence for the deconstruction dual of quote: a `kind` dispatcher matches a reflected
           AST with a QUALIFIED `((. Ast Ctor) subpat)` pattern for EVERY Ast variant — the exact pattern
           shape the Lean oracle recognizes (the `Ast` sum is built-in, absent from the scanned `(type …)`
           decls, so its qualified ctor patterns had fallen through to a headless-list skip). The Ast sum
           has SEVENTEEN variants: the nine scalar/generic leaves (Int/Float/Name/List/Bool/Str/Char/Bytes/
           Symbol), the seven native-compound-ctor variants (ListCtor/TupleCtor/RecordCtor/MapCtor/
           SetCtor/FieldPair/Member — operator 2026-08-28 native-collections-in-the-Ast), and the native
           Rational literal variant (`3/2` — reflection stays total over every well-formed literal leaf).
           All seventeen arms are present, so the match is EXHAUSTIVE over the whole sum with NO wildcard (a
           bare `_` would defeat the point of this fence — it proves coverage by naming every variant).
           `main` EXERCISES the nine QUOTABLE leaves, applied to one representative literal each, weighted by
           position `i` so a misclassification of arm `i` shifts the total off the self-witnessing `Σ_{1..9}
           i*i` = 285 (e.g. a Bytes literal miscaught as Symbol makes term 8 read 8*9). A `#\"…\"` literal is a
           SYMBOL (arm 9), a `b\"…\"` literal is BYTES (arm 8) — the two are distinct leaves that a naive
           reader conflates.
           The seven compound-ctor arms AND the rational arm (17) are present for EXHAUSTIVENESS but not
           exercised by `main` HERE — they are exercised elsewhere: the quote-BUILDS-them direction (each
           collection literal + member access reflecting to its dedicated ctor) is pinned by the dedicated
           case \"a quoted collection or member access equals the node built by its dedicated Ast ctor, never
           name-headed\" just below, and `(quote 3/2)` deconstructs through the `Ast.Rational` arm in its own
           case. So this fence proves the deconstruction arms EXIST + are exhaustive (no wildcard); the
           companion case proves quote CONSTRUCTS the compound ctors. (Co-owned by v-ast-compound, the Ast-sum
           owner, + v-metaprog.)")
  (input
    (do
      (def
        (kind a)
        (match
          a
          ((Ast.Int _) 1)
          ((Ast.Float _) 2)
          ((Ast.Name _) 3)
          ((Ast.List _) 4)
          ((Ast.Bool _) 5)
          ((Ast.Str _) 6)
          ((Ast.Char _) 7)
          ((Ast.Bytes _) 8)
          ((Ast.Symbol _) 9)
          ((Ast.ListCtor _) 10)
          ((Ast.TupleCtor _) 11)
          ((Ast.RecordCtor _) 12)
          ((Ast.MapCtor _) 13)
          ((Ast.SetCtor _) 14)
          ((Ast.FieldPair _) 15)
          ((Ast.Member _) 16)
          ((Ast.Rational _) 17)))
      (def
        (main)
        (+
          (* 1 (kind (quote 42)))
          (+
            (* 2 (kind (quote 2.5)))
            (+
              (* 3 (kind (quote foo)))
              (+
                (* 4 (kind (quote (a b))))
                (+
                  (* 5 (kind (quote true)))
                  (+
                    (* 6 (kind (quote "s")))
                    (+
                      (* 7 (kind (quote #\c)))
                      (+ (* 8 (kind (quote b"\x00"))) (* 9 (kind (quote #"sym"))))))))))))
      (export main)))
  (output (: 285 Int64)))

(case
  "a quoted collection or member access equals the node built by its dedicated Ast ctor, never name-headed"
  (doc
    "Witnesses metaprogramming.md §\"Quote Produces An AST Value\": quoting a collection construction — a
           list, tuple, record, map, or set — MUST produce that collection's OWN first-class ctor variant
           (`Ast.ListCtor`/`TupleCtor`/`RecordCtor`/`MapCtor`/`SetCtor`), a reflected record is a `RecordCtor`
           of `Ast.FieldPair` values and a reflected map a `MapCtor` of `FieldPair` values, and a member
           access `(. obj key)` an `Ast.Member` — NO collection reflects as a string- or name-headed node.
           Each `(quote <literal>)` is checked `= <the same node hand-built from the Ast ctor>`, weighted by
           position so a form that reflected to an `Ast.List`/`Ast.Name` (the old name-headed shape) instead
           of its dedicated ctor drops its term: 1(list)+2(tuple)+4(record-of-FieldPair)+8(map-of-FieldPair)+
           16(set)+32(member) = 63. The `FieldPair`/`Member` payload is a single `(Tuple Ast Ast)` (`#tuple`).
           Discharges the fence's TODO(quote-of-collections) for the reflection direction (the exhaustiveness
           fence above proves the deconstruction arms exist; this proves quote BUILDS them).")
  (input
    (do
      (def
        (main)
        (+
          (*
            1
            (if
              (= (quote #list(1 2 3)) (Ast.ListCtor #list((Ast.Int 1) (Ast.Int 2) (Ast.Int 3))))
              1
              0))
          (+
            (*
              2
              (if (= (quote #tuple(1 true)) (Ast.TupleCtor #list((Ast.Int 1) (Ast.Bool true)))) 1 0))
            (+
              (*
                4
                (if
                  (=
                    (quote #record((= a 1) (= b 2)))
                    (Ast.RecordCtor
                      #list((Ast.FieldPair #tuple((Ast.Name "a") (Ast.Int 1)))
                        (Ast.FieldPair #tuple((Ast.Name "b") (Ast.Int 2))))))
                  1
                  0))
              (+
                (*
                  8
                  (if
                    (=
                      (quote #map((= 1 true)))
                      (Ast.MapCtor #list((Ast.FieldPair #tuple((Ast.Int 1) (Ast.Bool true))))))
                    1
                    0))
                (+
                  (*
                    16
                    (if
                      (=
                        (quote #set(1 2 3))
                        (Ast.SetCtor #list((Ast.Int 1) (Ast.Int 2) (Ast.Int 3))))
                      1
                      0))
                  (*
                    32
                    (if
                      (= (quote obj.key) (Ast.Member #tuple((Ast.Name "obj") (Ast.Name "key"))))
                      1
                      0))))))))
      (export main)))
  (call main)
  (output (: 63 Int64)))

(case
  "Ast.gensym mints a fresh Ast.Name, distinct per call site, stable per binding (manual macro hygiene)"
  (doc
    "The fresh-name substrate for MANUAL macro hygiene (DESIGN-macro-system.md — macros are plain
           functions, non-hygienic by default; a macro avoids capture by minting its introduced binders with
           `Ast.gensym`). `(Ast.gensym base)` folds at compile time to a fresh `Ast.Name` whose spelling
           embeds an unreadable character (a space) + this call node's id, so it (a) IS an `Ast.Name`
           (weight 1), (b) two DISTINCT call sites produce DISTINCT names — freshness (weight 2, the `= …`
           is 0 so the term is 1), and (c) one gensym bound ONCE and referenced twice is the SAME name —
           stable per binding, the property manual hygiene relies on (weight 4). Self-witness 1+2+4 = 7. A
           name a source program cannot write (the space) guarantees no collision with a user identifier or
           another gensym; node-id keying keeps expansion DETERMINISTIC (same program → same names).")
  (input
    (do
      (def
        (main)
        (+
          (* 1 (match (Ast.gensym "x") ((Ast.Name _) 1) (_ 0)))
          (+
            (* 2 (if (= (Ast.gensym "x") (Ast.gensym "x")) 0 1))
            (* 4 (let ((g (Ast.gensym "x"))) (if (= g g) 1 0))))))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a qualified (. Ast Ctor) pattern binds each variant's payload at its own type"
  (doc
    "The payload-binding half of the deconstruction fence: `((. Ast Ctor) subpat)` binds the subpattern
           to the variant's payload AT ITS DECLARED TYPE — `Ast.Int` binds a BigInt (not Int64, the lossless-
           storage flip), `Ast.Name` binds a String, `Ast.Symbol` binds a SYMBOL (not a String — a `#\"…\"`
           leaf), `Ast.Bytes` binds a Bytes. Each arm reads its bound payload back and scores only on an exact
           match, summing 10+20+30+40 = 100. A binding that surfaced the wrong payload type (e.g. a Symbol as a
           String, or an Int64-truncated BigInt) would fail its arm's equality and drop the total. Companion to
           the nine-variant classification pin above.")
  (input
    (do
      (def
        (main)
        (+
          (match (quote 7) ((Ast.Int n) (if (= n (: 7 BigInt)) 10 0)) (_ 0))
          (+
            (match (quote foo) ((Ast.Name s) (if (= s "foo") 20 0)) (_ 0))
            (+
              (match (quote #"sy") ((Ast.Symbol y) (if (= y #"sy") 30 0)) (_ 0))
              (match (quote b"\x01\x02") ((Ast.Bytes b) (if (= (Bytes.len b) 2) 40 0)) (_ 0))))))
      (export main)))
  (output (: 100 Int64)))

(case
  "a bare rational literal quotes as a first-class Ast.Rational of its numerator and denominator"
  (doc
    "A rational literal `3/2` is its own syntactic form (a `(RationalTag <num> <den>)` node), so quoting
           it reflects to the DEDICATED `Ast.Rational` variant whose payload is a `(Tuple Ast Ast)` of the two
           child ASTs (each an `Ast.Int`) — NOT a name-headed generic node, mirroring the collection-ctor /
           FieldPair / Member variants. Deconstructs `(quote 3/2)` through `(Ast.Rational #tuple((Ast.Int n)
           (Ast.Int d)))` and scores `n*100 + d` = 302 (num 3, den 2 — already lowest terms). Before this
           the `Leaf::Rational` head hit the reifier's un-reifiable-leaf bail and the whole quote DECLINED
           (`quote produces an AST value, not supported`); this closes reflection totality over the rational
           literal leaf, and the encode/decode arms round-trip it through the binary-AST codec.")
  (input
    (do
      (def
        (main)
        (match
          (quote 3/2)
          ((Ast.Rational #tuple((Ast.Int n) (Ast.Int d))) (+ (* 100 (Int64.of n)) (Int64.of d)))
          (_ 0)))
      (export main)))
  (output (: 302 Int64)))

(case
  "a type-suffixed numeric literal quotes as the (: <body> Type) annotation the suffix denotes"
  (doc
    "A `100N`/`0.5R` suffix IS a terse annotation — the reader desugars it to `(: <body> BigInt|Rational)`,
           and the normalization that restores that invariant now runs BEFORE quote reification, so a
           suffixed literal inside a `(quote …)` reifies as its annotation form rather than declining on an
           un-reifiable `Suffixed` leaf. Pins the two spellings structurally EQUAL: `(quote 5N)` is the same
           `Ast` value as `(quote (: 5 BigInt))`, and `(quote 0.5R)` the same as `(quote (: 0.5 Rational))`.
           Before the ordering fix the whole quote declined (`quote produces an AST value, not supported`);
           this is the metaprogramming face of the reader's suffix-is-an-annotation rule.")
  (input
    (do
      (def
        (main)
        (+
          (if (= (quote 5N) (quote (: 5 BigInt))) 10 0)
          (if (= (quote 0.5R) (quote (: 0.5 Rational))) 20 0)))
      (export main)))
  (output (: 30 Int64)))

; A quasiquote in PATTERN position: a FINAL `,@name` binds the remaining elements of the matched form as a
; list (`` `(f ,@args) `` folds `.. args` against the constant `(quote (f 1 2 3))` → args = the 3 operand
; nodes). A NON-FINAL `,@` — `` `(f ,@init ,last) `` — is ill-formed (a rest binder is meaningful only last),
; rejected CDZ0221 (the quote-pattern analogue of the binary-form CDZ0220). (migrated from rcdzc
; a_quote_pattern_final_splice_binds_the_rest_and_a_non_final_splice_is_cdz0221.)
(case
  "a final splice in a quote pattern binds the remaining elements as a list"
  (input
    (do
      (def
        (main)
        (match
          (quote (f 1 2 3))
          ((quasiquote (f (unquote-splicing args))) (List.len args))
          (other 0)))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a non-final splice in a quote pattern is rejected CDZ0221"
  (input
    (do
      (def
        (main)
        (match
          (quote (f 1 2 3))
          ((quasiquote (f (unquote-splicing init) (unquote last))) last)
          (other other)))
      (export main)))
  (error CDZ0221))

(case
  "eval of a quoted subtraction that goes negative preserves the sign"
  (doc
    "`(eval (quote (- 3 10)))` = -7: the existing arithmetic eval cases produce only POSITIVE results,
           so none exercise a negative eval result. Pins that eval's arithmetic reduction carries the sign
           of a below-zero difference (a folder that clamped at zero or dropped the sign would pass the
           positive cases yet break here).")
  (input (eval (quote (- 3 10))))
  (output (: -7 Int64)))

(case
  "eval of a quoted multiplication folds through the compile-time evaluator"
  (doc
    "The multiplicative companion of the earlier `+`/`-` eval cases (the additive operators the
           eval-fold path witnessed before this family was completed): `(eval (quote (* 6 7)))` = 42. Pins
           that eval's arithmetic reduction dispatches the `*` head, not just the additive `+`/`-` heads — a
           folder that hard-coded only add/sub would pass every prior arithmetic case yet decline or
           miscompute here.")
  (input (eval (quote (* 6 7))))
  (output (: 42 Int64)))

(case
  "eval of a quoted division folds through the compile-time evaluator"
  (doc
    "The `/` companion of the multiplication case: `(eval (quote (/ 20 4)))` = 5. Pins that eval's
           arithmetic reduction dispatches integer division (distinct primitive from `*`), completing the
           four-operator arithmetic-eval family alongside `+`/`-`/`*`.")
  (input (eval (quote (/ 20 4))))
  (output (: 5 Int64)))

(case
  "eval of a quoted remainder folds through the compile-time evaluator"
  (doc
    "The `%` companion completing the arithmetic-eval operator set: `(eval (quote (% 17 5)))` = 2.
           Pins that eval's reduction dispatches the remainder head (yet another distinct primitive), so
           the whole `+`/`-`/`*`/`/`/`%` family is witnessed through the eval-fold path, not just addition.")
  (input (eval (quote (% 17 5))))
  (output (: 2 Int64)))

(case
  "eval of a quasiquote splicing a compile-time-known value"
  (doc
    "The core macro idiom (metaprogramming.md #Eval Is Optional / #Quasiquote Constructs AST With
           Selective Evaluation): eval a quasiquoted form whose unquote splices a compile-time-known VALUE,
           not just a bare literal. `(let ((x 3)) (eval `(+ ,x 4)))` reconstructs `(+ x 4)` and folds to 7.
           The eval desugar reconstructs `(eval AST)` to the source the AST denotes; an active unquote lifts
           its live operand into `(Ast.Int <e>)`, so reconstruction unwraps that back to `<e>` — a let-bound
           name, a def-const, or a computed constant, all resolving in the eval's enclosing scope. (A bare-
           LITERAL splice `(unquote 3)` and a plain `(quote …)` already worked; a NON-literal unquote once
           left the eval un-desugared, so its head `eval` reported a misleading 'unbound name eval'.) The
           reconstructed source must reach the enclosing `let`, so the desugar blanks the dead reified-
           argument wrappers, leaving the spliced `x` node parented at the eval position. Expected 7.")
  (input (do (def (main) (let ((x 3)) (eval (quasiquote (+ (unquote x) 4))))) (export main)))
  (output (: 7 Int64)))

; The eval-of-quasiquote macro idiom composes with the FLOAT, STRING, and BYTES leaves, and `print`
; renders a quoted float re-readably — pinning that the eval/print paths handle the leaves this vertical
; realized, not only integers/names. A float unquote lifts + reconstructs + folds like an integer one; a
; string splices through ordinary String ops; a byte-string unquote lifts + reconstructs + folds through
; Bytes ops the same way (case "eval of a quasiquote-built form with a byte-string unquote folds"); and
; `print` of a quoted float carries a `.` so it re-reads.
(case
  "eval of a quasiquote-built form with a float unquote folds"
  (doc
    "The float companion of the eval-splice idiom: `(let ((x 2.5)) (eval `(+ ,x 1.5)))` lifts the
           float `x` into the reconstructed `(+ x 1.5)` and folds to 4.0 — the active-unquote float lift
           (Ast.Float) composes with `eval`'s source reconstruction exactly as the integer case does.")
  (input (do (def (main) (let ((x 2.5)) (eval (quasiquote (+ (unquote x) 1.5))))) (export main)))
  (output (: 4.0 Float64)))

(case
  "eval of a quasiquote-built form with a byte-string unquote folds"
  (doc
    "The BYTES companion of the eval-splice idiom, closing the leaf-lift family for the `Ast.Bytes`
           variant this vertical realized (operator seq 113 — added AFTER the Int/Float/String/Name lift
           cases were pinned). `(let ((b b\"hi\")) (eval `(Bytes.concat ,b b\"x\")))` lifts the live byte-string
           `b` into the reconstructed `(Bytes.concat b b\"x\")` — the active unquote reifies its operand into an
           `(Ast.Bytes …)` node, which `eval`'s source reconstruction unwraps back to `b` in the enclosing
           scope, exactly as the integer/float/string lifts do. The result `b\"hix\"` has `Bytes.len` 3. A
           lift path that had no `Ast.Bytes` arm would leave the eval un-desugared (a misleading 'unbound
           name eval') or fail to reconstruct the byte operand.")
  (input
    (do
      (def (main) (let ((b b"hi")) (Bytes.len (eval (quasiquote (Bytes.concat (unquote b) b"x"))))))
      (export main)))
  (output (: 3 Int64)))

(case
  "eval of a quasiquote with TWO unquotes splices a bound and a computed value in one form"
  (doc
    "The eval-splice pins above are SINGLE-unquote; this reconstructs a form with TWO active
           unquotes at different nesting depths — one LET-BOUND (a=5) and one COMPUTED ((+ 3 4)) —
           so the desugar must lift and reconstruct BOTH (a per-form single-unquote assumption, or a
           lift that clobbers the first splice while processing the second, breaks it). 5 + 7·2 = 19.")
  (input
    (do
      (def (main) (let ((a 5) (b (+ 3 4))) (eval (quasiquote (+ (unquote a) (* (unquote b) 2))))))
      (export main)))
  (output (: 19 Int64)))

(case
  "a runtime-woven Ast compares structurally equal to an equivalent quote"
  (doc
    "The quote-vs-constructor eq pins compare const-foldable operands; a RUNTIME BigInt leaf
           inside the constructor-built tree forces the LIVE deep Ast walk against the reader-built
           quote. Hit + miss faces.")
  (input
    (do
      (def
        (main (: a Int64))
        (if
          (= (Ast.List #list((Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2))) (quote (+ 5 2)))
          1
          0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "a scalar Ast.Int built in a helper from a runtime param compares = a constant Ast.Int"
  (doc
    "The minimal SCALAR-top companion of the runtime-woven pin: an `Ast.Int` built by a helper
           from a runtime `Int64` param (grounded with `BigInt.of`) compares structurally `=` against a
           constant `Ast.Int`. This memorializes the resolution of the (2026-07-20 re-diagnosed) queue
           finding `mlrepro-recursive-tagged-template-tag-cannot-fold`, which claimed runtime `=` on an
           `Ast` value declined 'comparison of a compound value needs a heap walk' on all three backends.
           It does NOT — the `Ast` sum (an `Ast.List (List Ast)` variant + a `BigInt` `Ast.Int` payload)
           routes through `Core::ValueEqShaped`: `ty_contains_list` descends the sum to its List variant
           and `eq_shaped_walkable` admits the `BigInt` leaf, so the descriptor-guided element-wise walk
           (list-sound AND BigInt/float-leaf-canonical) fires. Hit + miss faces; guards against a future
           narrowing of the value-eq-shaped leaf/list domain silently re-declining scalar `Ast` equality.")
  (input
    (do
      (def (mk (: n Int64)) (Ast.Int (BigInt.of n)))
      (def (main (: n Int64)) (if (= (mk n) (Ast.Int (BigInt.of 3))) 1 0))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 0 Int64)))

(case
  "a THREE-deep constructor-woven Ast destructures through nested patterns to its runtime leaf"
  (doc
    "The nested-quote-pattern pin is 2-deep with const leaves; this weaves a 3-deep
           constructor tree carrying a runtime BigInt leaf at the bottom and destructures through
           THREE nested Ast.List patterns; the negative face rides the BigInt round-trip.")
  (input
    (do
      (def
        (main (: a Int64))
        (do
          (def
            t
            (Ast.List
              #list((Ast.Name "g")
                (Ast.List
                  #list((Ast.Name "h") (Ast.List #list((Ast.Name "i") (Ast.Int (BigInt.of a)))))))))
          (match
            t
            ((Ast.List
                #list((Ast.Name _g)
                  (Ast.List #list((Ast.Name _h) (Ast.List #list((Ast.Name _i) (Ast.Int n)))))))
              (Int64.of n))
            (_ -1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (output (: -3 Int64)))

(case
  "a constructor-tree eval's folded result composes with a runtime parameter per call"
  (doc
    "The eval pins return the fold directly; this let-binds it and adds a per-call runtime a
           — the eval-then-use idiom; also pins the explicit Int64.of on eval's BigInt result (no
           silent promotion). Zero-crossing face at a=-3.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((v (eval (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2))))))
          (+ a (Int64.of v))))
      (export main)))
  (call main (: 39 Int64))
  (output (: 42 Int64))
  (call main (: -3 Int64))
  (output (: 0 Int64)))

(case
  "eval of a quasiquote splicing a string works through String ops"
  (doc
    "The string companion: `(let ((s \"hi\")) (String.byte-len (eval `(String.concat ,s \"x\"))))`
           splices the runtime string `s` into the reconstructed `(String.concat s \"x\")`, evaluates it to
           `\"hix\"`, and reads its length 3. Pins that a string unquote reconstructs + folds through
           ordinary String operations in the eval'd source.")
  (input
    (do
      (def
        (main)
        (let ((s "hi")) (String.byte-len (eval (quasiquote (String.concat (unquote s) "x"))))))
      (export main)))
  (output (: 3 Int64)))

; --- HYGIENE: an unquoted variable is NOT captured by a same-named binder introduced in the template -----
; `,x` (an active unquote of a VARIABLE) splices x's value resolved in the quasiquote's ENCLOSING scope
; (metaprogramming.md §Quasiquote Constructs AST With Selective Evaluation — ",<expr> evaluates <expr>
; normally and inserts its result"). When the template ITSELF introduces a binder of the same name — `(let
; ((x 1)) (+ ,x 99))` — that inner binder MUST NOT capture the unquoted `x`: `,x` is the outer variable,
; the template's `x=1` is a separate (here unused) binding. `eval`'s desugar reconstructs source and, before
; the fix, spliced the unquoted variable as its bare NAME node, which the template's inner binder then
; lexically captured → silent WRONG value (100 = `(+ 1 99)` instead of 109 = `(+ 10 99)`), a variable-capture
; hygiene miscompile on both backends (breaker-found). The fix (`eval_ast::rename_captured_binders`)
; alpha-renames any template-introduced binder that collides with a spliced (enclosing-scope) name — so a
; template binder can never capture an unquoted variable. Binder-kind-agnostic: `let`, `fn`/lambda param,
; and `match`-pattern binders all covered.
(case
  "an unquoted variable is not captured by a same-named let binder in the template"
  (doc
    "`(let ((x 10)) (eval `(let ((x 1)) (+ ,x 99))))` = 109: the `,x` splices the OUTER x's value 10
           (its enclosing-scope binding), and the template's inner `(let ((x 1)) …)` — a same-named binder —
           MUST NOT capture it (the inner x=1 is a dead binding here). Pins the hygiene fix: eval-reconstruct
           alpha-renames the capturing template binder, so `,x`→10 gives `(+ 10 99)` = 109, not the captured
           `(+ 1 99)` = 100. Classic macro variable-capture, silent wrong value on both backends before the fix.")
  (input (let ((x 10)) (eval (quasiquote (let ((x 1)) (+ (unquote x) 99))))))
  (output (: 109 Int64)))

(case
  "an unquoted variable is not captured by a same-named fn param in the template"
  (doc
    "The lambda-param companion: `(let ((z 7)) (eval `((fn (z) (+ ,z 99)) 3)))` = 106. The `,z` splices
           the enclosing z=7, and the template's `(fn (z) …)` param — same name — must not capture it (the
           param z=3 is a separate binding). Pins that the hygiene fix is binder-kind-agnostic: a fn/lambda
           param cannot capture an unquoted variable either (captured → `(+ 3 99)` = 102, wrong).")
  (input (let ((z 7)) (eval (quasiquote ((fn (z) (+ (unquote z) 99)) 3)))))
  (output (: 106 Int64)))

(case
  "an unquoted variable is not captured by a same-named match-pattern binder in the template"
  (doc
    "The match-binder companion: `(let ((x 10)) (eval `(match 1 (x (+ ,x x)))))` = 11. The `,x` splices
           the enclosing x=10; the template's `match` arm binds `x` to the scrutinee 1 — a same-named
           binder — which must not capture the unquoted x. So `(+ ,x x)` = `(+ 10 1)` = 11, not the captured
           `(+ 1 1)` = 2. Pins the fix over a match-pattern binder.")
  (input (let ((x 10)) (eval (quasiquote (match 1 (x (+ (unquote x) x)))))))
  (output (: 11 Int64)))

(case
  "hygiene fix does not over-reach: a different-named template binder needs no rename"
  (doc
    "The control: an unquoted `x` into a template binding a DIFFERENT name `y` — `(let ((x 10)) (eval
           `(let ((y 1)) (+ ,x 99))))` = 109 — is already correct (no collision, no capture), and the
           hygiene pass leaves it untouched. Pins that the alpha-rename fires ONLY on a genuine name
           collision, never renaming an innocent binder.")
  (input (let ((x 10)) (eval (quasiquote (let ((y 1)) (+ (unquote x) 99))))))
  (output (: 109 Int64)))

; The hygiene fix reaches a binder NESTED in a COMPOUND match pattern, not only a bare-name pattern — a
; variant payload `(Some x)`, a `(tuple x y)`, a `(list x .. rest)`, and nesting. `eval_ast`'s
; `collect_pattern_binders` recurses into the pattern (skipping the ctor/alias head + the `..` marker), so
; every pattern binder that collides with a spliced unquote name is alpha-renamed. Without this, a
; compound-pattern binder still captured the spliced var (`(match (Some 1) ((Some x) ,x))` → 1 not 10) —
; the gap that remained after the bare-pattern fix.
(case
  "an unquoted variable is not captured by a variant-payload match-pattern binder"
  (doc
    "`(let ((x 10)) (eval `(match (Some 1) ((Some x) ,x) (_ 0))))` = 10: the `,x` splices the enclosing
           x=10; the template arm's `(Some x)` payload binder — a binder NESTED in a compound pattern — must
           not capture it. Pins that the hygiene rename recurses into a variant-payload pattern (captured
           would give the payload 1).")
  (input (let ((x 10)) (eval (quasiquote (match (Some 1) ((Some x) (unquote x)) (_ 0))))))
  (output (: 10 Int64)))

(case
  "an unquoted variable is not captured by a tuple-pattern binder"
  (doc
    "`(let ((x 10)) (eval `(match (tuple 1 2) ((tuple x y) (+ ,x y)))))` = 12: the `,x`=enclosing 10,
           the tuple pattern's `x` binder (= 1) must not capture it → `(+ 10 2)` = 12, not the captured
           `(+ 1 2)` = 3. Pins the recursion into a `tuple` compound pattern.")
  (input (let ((x 10)) (eval (quasiquote (match #tuple(1 2) (#tuple(x y) (+ (unquote x) y)))))))
  (output (: 12 Int64)))

(case
  "an unquoted variable is not captured by a list-rest-pattern binder"
  (doc
    "`(let ((x 10)) (eval `(match (list 1 2) ((list x .. rest) ,x) (_ 0))))` = 10: the `list` pattern's
           head-element binder `x` (= 1) must not capture the spliced enclosing x=10. Pins the recursion into
           a `(list … .. rest)` pattern — the `..` rest marker is skipped, its binder neighbors are renamed.")
  (input
    (let ((x 10)) (eval (quasiquote (match #list(1 2) (#list(x (.. rest)) (unquote x)) (_ 0))))))
  (output (: 10 Int64)))

(case
  "an unquoted variable is not captured by a map-pattern value binder"
  (doc
    "`(let ((x 10)) (eval `(match (map (1 2)) ((map (1 x)) ,x) (_ 0))))` = 10: a `(map (k p)…)` element
           is a KEY-DIRECTED lookup pattern (05-compound-types), so its value sub-pattern `x` is a binder
           (here it binds the value 2 stored at key 1). The spliced `,x` must keep its enclosing value 10, so
           the template's map-pattern value binder must NOT capture it → 10, not the captured 2. Pins that
           `collect_pattern_binders` recurses through a `(map (k p))` element's VALUE sub-pattern — the map
           face of the variant/tuple/list-rest binder-kind family above (the key `1` is a literal, not a
           binder, so only the value sub-pattern is renamed).")
  (input (let ((x 10)) (eval (quasiquote (match #map((= 1 2)) (#map((= 1 x)) (unquote x)) (_ 0))))))
  (output (: 10 Int64)))

(case
  "an eval folds a quoted match whose #map pattern carries a (.. rest) marker (OPEN pattern)"
  (doc
    "Regression fence for the #6855 map-rest miscompile: a `(.. _r)` rest marker inside a QUOTED `#map`
           PATTERN must reflect + reconstruct as an OPEN pattern (a rest binder), not a `(= .. _r)` field
           pair. `reify_inner` reified the ctor children as `Ast.FieldPair`s and ran `field_kv` on EVERY
           child — its 2-element-list fallback mis-read `(.. _r)` as a key/value pair, CLOSING the pattern,
           so the eval-of-quoted-match found no such field and FELL THROUGH to the catch-all, folding to the
           WRONG value (-1 instead of the value bound at key 1) — a decline-don't-miscompile violation
           (breaker `adv-quoted-map-rest-pattern-falls-through-after-reify-6855`). The direct (un-quoted)
           twin binds `v`=10; the quoted form must agree. Now folds to 10.")
  (input (eval (quote (match #map((= 1 10)) (#map((= 1 v) (.. _r)) v) (_ -1)))))
  (output (: 10 Int64)))

(case
  "the quoted #map-rest pattern's rest binder captures the RESIDUAL entries"
  (doc
    "Pins that the reflected rest binder is not merely tolerated but binds the residual map: over a
           two-entry map the `(= 1 v)` face binds `v`=10 and `(.. r)` binds the rest `{2:20}`, so
           `(+ v (Map.len r))` = 10 + 1 = 11. Distinguishes a correct OPEN pattern from one that just drops
           the marker.")
  (input
    (eval (quote (match #map((= 1 10) (= 2 20)) (#map((= 1 v) (.. r)) (+ v (Map.len r))) (_ -1)))))
  (output (: 11 Int64)))

(case
  "a QUASIQUOTED #map-rest pattern reflects through reify_active and splices an unquote"
  (doc
    "The `reify_active` (quasiquote-value) twin of the map-rest fence: the same `(.. r)` rest marker
           inside a QUASIQUOTED `#map` pattern must stay OPEN, and the arm body splices an unquoted
           enclosing value. `v`=10 (bound at key 1), `,x`=5, so `(+ v ,x)` = 15.")
  (input
    (let
      ((x 5))
      (eval
        (quasiquote (match #map((= 1 10) (= 2 20)) (#map((= 1 v) (.. r)) (+ v (unquote x))) (_ -1))))))
  (output (: 15 Int64)))

(case
  "an eval folds a quoted match whose #record pattern carries a (.. rest) marker (OPEN pattern)"
  (doc
    "The `#record` face of the map-rest fence — the same reflect/reconstruct path (`fieldpair_children`)
           handles both `#map` and `#record`. A `(.. _r)` rest marker inside a QUOTED `#record` pattern
           stays OPEN: `(= a x)` binds `x`=1 and `(.. _r)` binds the residual `{b:2}`, so the arm yields 1.")
  (input (eval (quote (match #record((= a 1) (= b 2)) (#record((= a x) (.. _r)) x) (_ -1)))))
  (output (: 1 Int64)))

(case
  "the hygiene rename recurses into a NESTED compound match pattern"
  (doc
    "Depth companion: `(match (Some (tuple 1 2)) ((Some (tuple x y)) (+ ,x y)))` with enclosing x=10 →
           12. The `x` binder is TWO compound levels deep (`Some` payload, then `tuple` element), and the
           rename still reaches it, so `,x`=10 gives `(+ 10 2)` = 12. Pins that `collect_pattern_binders`
           recurses to arbitrary compound-pattern depth.")
  (input
    (let
      ((x 10))
      (eval (quasiquote (match (Some #tuple(1 2)) ((Some #tuple(x y)) (+ (unquote x) y)) (_ 0))))))
  (output (: 12 Int64)))

; The hygiene cases above pin one unquote colliding with one binder (per kind) + the no-over-reach control.
; These push the capture-avoiding alpha-rename harder: TWO distinct unquotes each colliding with a DIFFERENT
; template binder (both must rename independently), a NESTED same-name shadow (an unquote spliced past two
; levels of the colliding name), and a splice BESIDE the template's own use of the renamed binder (the
; renamed binder's occurrences must still resolve to the template value while the splice keeps its own).
(case
  "hygiene: two distinct unquotes each colliding with a different template binder are both renamed"
  (doc
    "`(let ((x 10) (y 20)) (eval `(let ((x 1) (y 2)) (+ ,x ,y))))` — the template binds BOTH x and y,
           and BOTH are unquoted with colliding names. Each splice keeps its enclosing-scope value (x=10,
           y=20), so the sum is 30, not 3 (the captured 1+2). Pins that the alpha-rename handles multiple
           independent collisions in one template, not just a single binder.")
  (input (let ((x 10) (y 20)) (eval (quasiquote (let ((x 1) (y 2)) (+ (unquote x) (unquote y)))))))
  (output (: 30 Int64)))

(case
  "hygiene: an unquote is uncaptured through two nested same-named template binders"
  (doc
    "`(let ((x 100)) (eval `(let ((x 1)) (let ((x 2)) (+ ,x x)))))` — the template nests TWO binders
           of the colliding name `x`. The spliced `,x` keeps its enclosing value 100; the template's own `x`
           resolves to the INNERMOST binder (2). Sum = 102, not 3 (2+1 captured) or 4 (2+2). Pins that the
           rename tracks the splice's provenance across nested same-name shadows, and the template's own `x`
           still binds to its lexically-nearest (innermost) binder.")
  (input (let ((x 100)) (eval (quasiquote (let ((x 1)) (let ((x 2)) (+ (unquote x) x)))))))
  (output (: 102 Int64)))

(case
  "hygiene: a spliced unquote sits beside the template's own use of the renamed binder"
  (doc
    "`(let ((x 5)) (eval `(let ((x 1)) (* ,x x))))` — the template both splices `,x` (value 5) AND
           uses its own `x` (bound to 1) in the same expression. After the rename, the splice is 5 and the
           template `x` is 1, so the product is 5, not 1 (both captured to 1) or 25 (both the splice 5). Pins
           that the renamed binder's own occurrences resolve to the template value while the splice keeps its
           enclosing value — the two `x`-shaped operands end up DIFFERENT.")
  (input (let ((x 5)) (eval (quasiquote (let ((x 1)) (* (unquote x) x))))))
  (output (: 5 Int64)))

(case
  "print of a quote containing a float renders re-readably"
  (doc
    "`print : Ast → String` renders a quoted compound containing a float as its canonical re-readable
           s-expression: `(quote (f 1.5))` prints `\"(f 1.5)\"` — the `Ast.Float` leaf renders with a `.` so
           the text re-reads as a float (not an integer). Pins that `print` handles the float leaf inside a
           compound, the companion of the leaf-level print/read round-trip cases.")
  (input (= (Ast.print (quote (f 1.5))) "(f 1.5)"))
  (output (: true Bool)))

; `print`'s EXACT canonical rendering — not just its round-trip. The `read(Ast.print v) == v` cases pin the
; printer/reader as INVERSES, but a round-trip normalizes, so it does NOT pin the exact text `print` emits
; (spacing between elements, nested parenthesization, the empty-list form). These assert the literal string,
; catching a printer that changed spacing/nesting yet still round-tripped: a deep compound with a nested
; list and a quoted string renders `(f (g 1) "s")` (one space between elements, inner parens, the Str leaf
; quoted), and an empty list renders `()`.
(case
  "print renders a nested compound with a string leaf as its exact canonical text"
  (doc
    "`print` of `(quote (f (g 1) \"s\"))` is exactly `\"(f (g 1) \\\"s\\\")\"`: elements space-
           separated, the nested list `(g 1)` parenthesized in place, and the `Ast.Str` leaf rendered as a
           QUOTED literal (distinct from the bare name `f`). Pins the exact rendering of nesting + spacing +
           string-quoting in one string — a printer that dropped a space or a paren would still round-trip
           but flip this literal-text assertion.")
  (input (= (Ast.print (quote (f (g 1) "s"))) "(f (g 1) \"s\")"))
  (output (: true Bool)))

(case
  "print renders an empty Ast.List as the empty-parens form"
  (doc
    "`print (Ast.List (list))` is exactly `\"()\"` — the zero-element list rendering (open then close
           with nothing between). Pins the empty-list edge of the printer, which the non-empty compound
           cases never reach.")
  (input (= (Ast.print (Ast.List #list())) "()"))
  (output (: true Bool)))

(case
  "print renders a single-element Ast.List as one parenthesized element"
  (doc
    "`print (quote (f))` is exactly `\"(f)\"` — the ONE-element (arity-1) list: open paren, the single
           element, close paren, no inter-element space. Completes the list-arity rendering coverage — 0
           elements → `()`, 1 → `(f)`, 2+ → the nested/compound cases above. Pins that the space-separator
           logic (only BETWEEN elements) emits none for a lone element.")
  (input (= (Ast.print (quote (f))) "(f)"))
  (output (: true Bool)))

; The bare-LEAF companions of the compound exact-print cases above. The compound cases pin exact spacing/
; nesting/parens; these pin the exact text a BARE scalar leaf renders — the `read(Ast.print v) == v` round-trip
; cases prove the printer/reader are inverses but a round-trip NORMALIZES, so it does not pin the literal
; string `print` emits for a lone leaf. A printer that rendered an Int with a leading `+`, a Bool as `#t`,
; or a Name with surrounding quotes would still round-trip yet emit the wrong exact text — caught here.
; `Ast.Int` (positive + NEGATIVE, so the sign is exact), `Ast.Bool` (the bare keyword), `Ast.Name` (the
; bare identifier, no quotes — the Str-vs-Name print distinction). Float exact-text is pinned separately.
(case
  "print renders a bare Ast.Int as its exact decimal text"
  (doc
    "`print (Ast.Int 42)` is exactly `\"42\"` — the bare decimal, no `+` sign, no leading zero, no
           wrapper. The leaf companion of the compound exact-print cases; pins the literal text a lone Int
           leaf emits, which the normalizing round-trip cases don't fix.")
  (input (= (Ast.print (Ast.Int 42)) "42"))
  (output (: true Bool)))

(case
  "print renders a bare NEGATIVE Ast.Int as its exact signed text"
  (doc
    "`print (Ast.Int -7)` is exactly `\"-7\"` — the minus is part of the rendered text (not dropped,
           not spaced). The signed companion of the positive-Int exact-print case.")
  (input (= (Ast.print (Ast.Int -7)) "-7"))
  (output (: true Bool)))

(case
  "print renders a bare Ast.Bool as its exact keyword text"
  (doc
    "`print (Ast.Bool true)` is exactly `\"true\"` — the bare boolean keyword the reader re-lexes as
           `Ast.Bool`, not `#t`/`True`/`1`. Pins the exact Bool rendering (a printer emitting a different
           spelling would still round-trip via read's keyword arm yet emit wrong text).")
  (input (= (Ast.print (Ast.Bool true)) "true"))
  (output (: true Bool)))

(case
  "print renders a bare Ast.Name as its exact identifier text"
  (doc
    "`print (Ast.Name \"foo\")` is exactly `\"foo\"` — the bare identifier with NO surrounding quotes
           (the print-side of the Name-vs-Str distinction: a Str renders `\"foo\"` WITH quotes, a Name
           without). A printer that quoted a Name would re-lex it as an `Ast.Str` — this pins that it does
           not.")
  (input (= (Ast.print (Ast.Name "foo")) "foo"))
  (output (: true Bool)))

(case
  "an Ast.Bytes node constructs and deconstructs by pattern matching"
  (doc
    "The AST sum has a `Bytes` variant for a raw byte-sequence LITERAL (`b\"…\"`) — a binary blob is a
           syntactic form, carried as a single `Bytes` payload (operator seq 113) so a blob rides the AST +
           its codec as ONE length-prefixed raw-bytes leaf rather than a node-per-byte list. `Ast.Bytes` is
           an ordinary sum variant: `(Ast.Bytes b\"hi\")` constructs it and a `(Ast.Bytes b)` pattern
           deconstructs it, exactly like the other Ast leaves.")
  (input (match (Ast.Bytes b"hi") ((Ast.Bytes _) 1) (_ 0)))
  (output (: 1 Int64)))

(case
  "quote of a byte-string literal reifies to an Ast.Bytes node"
  (doc
    "`(quote b\"hi\")` reifies the `b\"…\"` byte-string literal to an `Ast.Bytes` node whose payload is
           the blob — the bytes companion of `(quote \"hi\")`→`Ast.Str` and `(quote 42)`→`Ast.Int`. Pins that
           the quote reifier has a `Leaf::Bytes`→`Ast.Bytes` arm (it previously declined, having no Bytes
           variant), so a quoted binary literal is a first-class AST node.")
  (input (match (quote b"hi") ((Ast.Bytes _) 1) (_ 0)))
  (output (: 1 Int64)))

(case
  "print renders a bare Ast.Bytes as its exact b\"…\" byte-literal text"
  (doc
    "`print (Ast.Bytes b\"hi\")` is exactly `\"b\\\"hi\\\"\"` — the `b\"…\"` byte-literal spelling
           (printable ASCII verbatim, `\\n \\t \\r \\\\ \\\"` named, else `\\xNN`), the canonical re-readable
           form for a blob. Pins the Bytes rendering distinct from an `Ast.Str` (which has no `b` prefix).")
  (input (= (Ast.print (Ast.Bytes b"hi")) "b\"hi\""))
  (output (: true Bool)))

(case
  "print of an Ast.Bytes escapes a non-printable byte as \\xNN"
  (doc
    "The escape face of the Bytes printer: a byte outside printable ASCII renders `\\xNN` (two
           lowercase hex), matching the reader's byte-literal escapes. `b\"\\x00\\xff\"` (a NUL and a 0xff)
           prints back to exactly that spelling. Pins the non-printable escape path the `b\"hi\"` case
           doesn't reach.")
  (input (= (Ast.print (Ast.Bytes b"\x00\xff")) "b\"\\x00\\xff\""))
  (output (: true Bool)))

(case
  "an Ast.Bytes node round-trips through encode and decode"
  (doc
    "The interchange face of `Ast.Bytes` (operator seq 113 payoff): `Ast.encode` writes the blob as a
           SINGLE length-prefixed raw-bytes node (a fresh additive tag past the Int/Name/List/Bool/Str/Float
           tags — legacy bytes decode exactly as before), and `Ast.decode` reads it back to an EQUAL
           `Ast.Bytes`. This is the whole point: a byte blob crosses the value codec as one node, not a
           node-per-byte `Ast.List` of `Ast.Int` u8s. `Ast.decode` is total, so the round-trip matches the
           `Ok` arm.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Bytes b"hi")))
      ((Ok a) (= a (Ast.Bytes b"hi")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "an empty Ast.Bytes round-trips through encode and decode"
  (doc
    "The zero-length edge of the bytes codec: `(Ast.Bytes b\"\")` encodes to the tag + a zero length
           (no payload bytes) and decodes back to an equal empty `Ast.Bytes`. Pins that the length-prefix
           framing handles the empty blob — a decoder mis-reading a zero length would diverge here.")
  (input
    (match (Ast.decode (Ast.encode (Ast.Bytes b""))) ((Ok a) (= a (Ast.Bytes b""))) ((Err _) false)))
  (output (: true Bool)))

(case
  "an Ast.Bytes carrying NUL and high bytes round-trips through encode and decode"
  (doc
    "The raw-byte face (distinct from `Ast.Str`, which is UTF-8): `(Ast.Bytes b\"\\x00\\xff\")` carries
           a NUL and a 0xff — bytes an `Ast.Str` could not — and round-trips byte-exactly through
           `Ast.encode`/`Ast.decode`. Pins that the bytes payload is RAW (not re-validated as UTF-8), the
           key difference from the Str codec arm.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Bytes b"\x00\xff")))
      ((Ok a) (= a (Ast.Bytes b"\x00\xff")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "Ast.decode of a RUNTIME byte sequence round-trips (not const-folded)"
  (doc
    "The RUNTIME decode path (value-heap op `ast-decode`, index 94): unlike every `Ast.decode`
           case above — whose bytes are a COMPILE-TIME constant that the compiler const-FOLDS — here the
           encoded bytes derive from a RUNTIME parameter `n`, so `Ast.encode` runs at run time (op 93) and
           its result is a RUNTIME `Bytes` the compiler cannot see through. `Ast.decode` must therefore
           parse it AT RUN TIME (op 94) rather than folding. Round-trips `Ast.Int (BigInt.of n)` through
           encode→decode across the runtime boundary and matches the `Ok` arm: `decode(encode(v)) == v`.
           A compiler that could not decode runtime bytes DECLINED here (op 94 was runtime-only, the
           compiler path missing); this pins that the runtime decode now works and stays total.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Ast.decode (Ast.encode (Ast.Int (BigInt.of n))))
          ((Ok a) (= a (Ast.Int (BigInt.of n))))
          ((Err _) false)))
      (export main)))
  (call main (: 42 Int64))
  (output (: true Bool))
  ; KNOWN-LEAK: a runtime Ast-value round-trip leaks heap cells (encode-only leaks 1; this
  ; encode→decode→`=` round-trip leaks 3) — a PRE-EXISTING runtime Ast-value/`=`/BigInt reclaim gap,
  ; NOT introduced by runtime Ast.decode (op 94): the analogous reflection round-trip
  ; (11-modules.sexp `(= (Ast.encode a) (Ast.encode __ast__))`) is already `(live-objects known-leak)`.
  ; Marked known-leak (consistent with that precedent) so this pins the runtime-decode VALUE round-trip;
  ; the underlying leak is surfaced to the memory-safety/runtime lane to fix (then drop this marker).
  (live-objects known-leak 3))

(case
  "an Ast.Bytes nested in an Ast.List round-trips through encode and decode"
  (doc
    "Composition: an `Ast.Bytes` as a child of an `Ast.List` (`(f b\"hi\")`) round-trips through the
           value codec — the list arm recurses into the bytes arm and back. Pins that the bytes node
           composes inside a compound, not only standalone.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.List #list((Ast.Name "f") (Ast.Bytes b"hi")))))
      ((Ok a) (= a (Ast.List #list((Ast.Name "f") (Ast.Bytes b"hi")))))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "print then read round-trips an Ast.Bytes through the text path"
  (doc
    "The TEXT interchange face: `print (Ast.Bytes b\"hi\")` renders `b\"hi\"` and `read` parses that
           byte-literal back to an EQUAL `Ast.Bytes` — `read(Ast.print v) == v` for a bytes node, the byte
           companion of the Str/Int text round-trips. Pins the reader's `b\"…\"` arm (a `b` immediately
           before `\"`), distinct from a bare `b` name.")
  (input (= (Ast.read (Ast.print (Ast.Bytes b"hi"))) (Ast.Bytes b"hi")))
  (output (: true Bool)))

(case
  "print then read round-trips an Ast.Bytes carrying escapes and high bytes"
  (doc
    "The escape face of the bytes text round-trip: `b\"\\x00\\xff\\n\"` (NUL, 0xff, newline) prints
           with `\\xNN`/`\\n` escapes and `read` parses them back byte-exactly. Pins the reader's `\\xNN`
           two-hex escape + the named `\\n` — the byte-literal escape set the printer emits, which a plain
           string reader (no `\\x`) would reject.")
  (input (= (Ast.read (Ast.print (Ast.Bytes b"\x00\xff\n"))) (Ast.Bytes b"\x00\xff\n")))
  (output (: true Bool)))

(case
  "read of a bare b token is an Ast.Name, not a byte-literal"
  (doc
    "The `b\"…\"` reader arm fires only when `b` is IMMEDIATELY followed by `\"` — a bare `b` token is
           an ordinary identifier, so `read \"b\"` is an `Ast.Name`. Pins that the byte-literal detection
           does not over-match a lone `b` (which would break reading the common single-letter name).")
  (input (match (Ast.read "b") ((Ast.Name _) 1) (_ 0)))
  (output (: 1 Int64)))

; `read : String → Ast` parses re-readable text, so its argument MUST be a String. A non-String operand is a
; type error, but a naive scheme-unify grounded the operand parameter to String and leaked the OPAQUE "String
; and <T> must be the same type here" clash. The reject now NAMES the real fault ("`read`'s argument must be a
; String"), the sibling of the `trap`-message-requirement reject (corpus 07). (The valid String-argument reads
; above are the no-false-positive companions.) (Migrated from rcdzc
; a_non_string_read_argument_names_the_string_requirement_not_a_phantom_clash.)
(case
  "a non-String Ast.read argument names the String requirement (Int64 operand)"
  (input (do (def (f) (Ast.read 5)) (export f)))
  (error
    CDZ0203
    (message "`read`'s argument must be a String")
    (message "a value of type Int64 was given")
    (not "must be the same type here")))

(case
  "a non-String Ast.read argument names the String requirement (Bool operand)"
  (input (do (def (f) (Ast.read true)) (export f)))
  (error
    CDZ0203
    (message "`read`'s argument must be a String")
    (message "a value of type Bool was given")
    (not "must be the same type here")))

(case
  "eval of a quoted byte-string literal folds to the bytes value"
  (doc
    "`(eval (quote b\"hi\"))` reconstructs the `b\"…\"` byte literal (which evaluates to itself, like a
           quoted string) and folds to that `Bytes` value — the bytes companion of `(eval (quote \"hi\"))`.
           Pins the eval reconstruct arm for `Ast.Bytes`.")
  (input (= (eval (quote b"hi")) b"hi"))
  (output (: true Bool)))

(case
  "the AST is a sum type deconstructible by pattern matching"
  (doc
    "Witnesses metaprogramming.md #Quote Produces An AST Value (2nd sentence): the AST is a
           sum type with variants for each syntactic form. Pattern matching over (quote 42) binds
           the integer payload, demonstrating AST variants are proper sum types. Because the AST is
           an ORDINARY sum (type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type),
           its match is subject to the same exhaustiveness rule any sum match is (#A Match Is
           Exhaustive Against The Sum Type's Variant Set), so a match that inspects one form carries a
           catch-all `_` arm for the others.")
  (input (match (quote 42) ((Ast.Int n) n) (_ 0N)))
  (output (: 42 BigInt)))

(case
  "an Ast.Int stores an integer wider than Int64 without loss"
  (doc
    "🔑 THE non-lossy-AST-storage pin (numeric-model.md — a literal grounds to `BigInt` losslessly):
           `Ast.Int`'s payload is `BigInt`, so a quoted integer with more than 19 digits (past the i64
           range) is stored + extracted EXACTLY. `(match (quote 12345678901234567890123456789) ((Ast.Int
           n) n))` binds the full 29-digit value — under the old `Int64` payload this DECLINED (the value
           did not fit). The extracted `n` is a `BigInt` (a stored AST integer is full-precision), so the
           catch-all is a `BigInt` (`0N`). This is why the payload is `BigInt`, not `Int64`: a compiler
           that quotes a program must not lose a large integer literal.")
  (input (match (quote 12345678901234567890123456789) ((Ast.Int n) n) (_ 0N)))
  (output (: 12345678901234567890123456789 BigInt)))

(case
  "the byte codec round-trips an Ast.Int wider than Int64"
  (doc
    "The CODEC face of non-lossy storage (ast-encoding.md — the encoding is a bijection): `Ast.encode`
           serializes an `Ast.Int` as tag + sign + magnitude-length + magnitude bytes, so a value PAST the
           i64 range round-trips exactly through `Ast.decode`. `(Ast.decode (Ast.encode (Ast.Int
           12345678901234567890123456789N)))` re-reads the full 29-digit value — the sign-magnitude wire
           form carries an arbitrary-width magnitude (a fixed 8-byte i64 field would truncate it). Distinct
           from the quote+match `stores wider than Int64` pin above: this exercises the BYTE codec at a
           magnitude that exceeds i64, the regime the length-prefixed format exists for.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Int 12345678901234567890123456789N)))
      ((Ok (Ast.Int n)) n)
      (_ 0N)))
  (output (: 12345678901234567890123456789 BigInt)))

(case
  "structural equality of two Ast.Int wider than Int64 compares by full value"
  (doc
    "Runtime structural `=` over an `Ast.Int` whose payload EXCEEDS i64 compares the full BigInt
           magnitude, not a truncated i64 window — `(= (Ast.Int 12345678901234567890123456789N) (Ast.Int
           12345678901234567890123456789N))` is `true`. Pins that the sum's structural equality walks the
           boxed-BigInt payload at its full width (a comparison that narrowed to i64 first would spuriously
           equate two distinct >i64 values sharing low bits). The equality face of the non-lossy payload.")
  (input (= (Ast.Int 12345678901234567890123456789N) (Ast.Int 12345678901234567890123456789N)))
  (output (: true Bool)))

(case
  "eval of a quoted integer literal grounds to Int64 (BigInt is AST storage, not eval width)"
  (doc
    "The dual of the lossless-storage pin: while an `Ast.Int` STORES its integer as `BigInt`, an
           `eval` RECONSTRUCTS the source the AST denotes and re-infers it at the ORDINARY context width —
           so `(eval (quote 5))` is an `Int64` `5`, exactly as the bare literal `5` would be, NOT a
           `BigInt`. `eval_ast::reconstruct` strips the reifier's `(: N BigInt)` grounding wrapper for
           this: BigInt is a property of the stored AST value, not one the reconstructed source carries
           out. Pins the storage-vs-eval-width distinction the operator directed (eval goes through the
           same int-width inference paths as the rest of the codebase).")
  (input (eval (quote 5)))
  (output (: 5 Int64)))

(case
  "eval of a quoted HUGE integer literal declines CDZ0201 — BigInt storage does not widen eval"
  (doc
    "The huge-leaf face of the eval-width boundary (see 'eval of a quoted integer literal grounds
           to Int64 (BigInt is AST storage, not eval width)' — the small-literal twin): a quoted
           26-digit literal is stored losslessly (see 'an Ast.Int carries a BEYOND-64-bit literal
           losslessly through quote'), but `eval` re-infers at ordinary width, so the huge leaf
           overflows Int64 and REJECTS with CDZ0201 (naming the BigInt annotation escape hatch)
           rather than truncating. Pins the Part-1 storage-only scope: lossless in the AST, ordinary
           width in eval.")
  (input (do (def (main) (eval (quote 99999999999999999999999999))) (export main)))
  (error CDZ0201))

(case
  "print renders a HUGE Ast.Int losslessly — the full 26-digit decimal, no truncation"
  (doc
    "The print face of the storage-vs-eval-width family: `print` of a BigInt-annotated Ast.Int
           renders the STORED value's exact decimal text (26 digits), unlike `eval` which re-infers
           at Int64 and declines (the huge-leaf eval case beside this one). Print reads the storage,
           so it inherits the losslessness of 'an Ast.Int carries a BEYOND-64-bit literal losslessly
           through quote'.")
  (input
    (do
      (def
        (main)
        (= (Ast.print (Ast.Int (: 99999999999999999999999999 BigInt))) "99999999999999999999999999"))
      (export main)))
  (output (: true Bool)))

(case
  "pattern matching over AST distinguishes forms"
  (doc
    "Witnesses metaprogramming.md #Quote Produces An AST Value: the compiler pattern-matches
           over AST sums to distinguish syntactic forms. Matching (quote (+ 1 2)) as an Ast.List
           allows inspecting its structure recursively. The AST is an ordinary sum, so the match
           covers the remaining variants with a catch-all `_` arm (#A Match Is Exhaustive Against The
           Sum Type's Variant Set).")
  (input (match (quote (+ 1 2)) ((Ast.List elems) (List.len elems)) (_ 0)))
  (output (: 3 Int64)))

; The case above matches ONE variant + catch-all; these pin VARIANT-TAG DISCRIMINATION — a match over an
; `Ast` value must dispatch on the scrutinee's actual variant, SKIPPING a preceding non-matching arm to
; reach the right one, not fall through to the catch-all or (worse) mis-fire the wrong arm. Two confusable
; leaf pairs: a quoted FLOAT skips a preceding `Ast.Int` arm (numeric-adjacent), and a quoted STRING skips
; a preceding `Ast.Name` arm (both text-carrying). A match that discriminated leaves by payload rather than
; variant tag, or that collapsed Float↔Int / Str↔Name, would mis-dispatch here.
(case
  "a match over a quoted float dispatches the Ast.Float arm past a preceding Ast.Int arm"
  (doc
    "`(quote 2.5)` is an `Ast.Float`, so a match with an `Ast.Int` arm FIRST skips it and selects the
           `Ast.Float` arm (= 2), not the catch-all. Pins variant-tag dispatch over the numeric-adjacent
           Float/Int pair — the discrimination the single-arm case above doesn't exercise.")
  (input (match (quote 2.5) ((Ast.Int _) 1) ((Ast.Float _) 2) (_ 0)))
  (output (: 2 Int64)))

(case
  "a match over a quoted string dispatches the Ast.Str arm past a preceding Ast.Name arm"
  (doc
    "`(quote \"hi\")` is an `Ast.Str`, so a match with an `Ast.Name` arm FIRST skips it and selects
           the `Ast.Str` arm (= 2). Pins variant-tag dispatch over the text-carrying Str/Name pair (both
           hold a String payload, so a payload-based rather than tag-based match would confuse them).")
  (input (match (quote "hi") ((Ast.Name _) 1) ((Ast.Str _) 2) (_ 0)))
  (output (: 2 Int64)))

; A NESTED match on the recursive `Ast` sum — `Ast.List` inside `Ast.List` — over a CONSTANT `quote` reads
; the deep leaf at the RIGHT depth. `Ast` is a recursive sum (`Ast.List` holds `(List Ast)`), and a quoted
; literal is a compile-time-constant scrutinee, so this is exactly the "recursive-sum nested match with a
; statically-known outer discriminant" shape — the canonical AST-tree-walk. It was a latent MISCOMPILE (the
; known-disc fold dropped the outer switch, so a nested read landed at the wrong depth) fixed at the emit
; layer (v-patterns `ce182df365`); pinned HERE over `Ast` (the tree type this vertical's macros walk) so a
; regression is caught from the metaprogramming angle, not only the generic-sum one.
(case
  "a nested Ast.List match over a constant quote reads the deep leaf at the right depth"
  (doc
    "`(match (quote (f (g 7))) ((Ast.List (list _ (Ast.List (list _ (Ast.Int n))))) n) (_ -1))` = 7 —
           the outer `Ast.List` is a constant (from `quote`), and the pattern reaches TWO levels down to bind
           the `Ast.Int 7` inside the inner `Ast.List`. Pins that a nested match on the recursive `Ast` sum
           with a known outer discriminant reads the inner payload at the correct depth (a walk that dropped
           the outer switch would read the wrong level — the miscompile v-patterns fixed). The AST-walk shape
           every macro that inspects nested structure relies on.")
  (input (match (quote (f (g 7))) ((Ast.List #list(_ (Ast.List #list(_ (Ast.Int n))))) n) (_ -1N)))
  (output (: 7 BigInt)))

(case
  "a nested Ast.List match falls through when the inner leaf variant differs"
  (doc
    "The discriminator companion: the SAME nested shape but the inner pattern expects an `Ast.Str`
           where the tree holds an `Ast.Int` — `(match (quote (f (g 7))) ((… (Ast.Str s))) 1) (_ 0))` = 0.
           Pins that the nested recursive-sum match is variant-SELECTIVE at depth (it does not spuriously
           fire on a depth-correct but variant-wrong inner leaf) — the negative face guarding the known-disc
           fold.")
  (input (match (quote (f (g 7))) ((Ast.List #list(_ (Ast.List #list(_ (Ast.Str s))))) 1) (_ 0)))
  (output (: 0 Int64)))

(case
  "eval on malformed AST traps"
  (doc
    "Witnesses metaprogramming.md #Eval Is Optional: eval on malformed AST traps. An Ast.List
           with no elements is malformed (no operator), so eval traps. The eval desugar reconstructs
           the source an `Ast.*` construction denotes; an empty `Ast.List` has no operator to
           reconstruct, so `eval_ast::reconstruct` rewrites it to an explicit `(trap \"malformed AST\")`
           — a diverging halt, not a value. The trap's canonical KIND is `unreachable`, the SAME on
           every backend: an explicit `trap` lowers to wasm's `unreachable` instruction and to a Rust
           `panic!` whose reason classifies as `unreachable` (a message-less halt — the trap_kind grader
           classifies the actual reason, and `Core::Trap` carries no string through either backend, so
           the observable kind is `unreachable`, matching the explicit-`trap` lowering pinned by the
           runtime expect-on-absent case in 02-binding-and-control.sexp).")
  (input (eval (Ast.List #list())))
  (trap "unreachable"))

; MALFORMED-AST FAMILY (companions to the empty-list case above): the eval desugar FAITHFULLY reconstructs the
; source an `Ast.*` construction denotes and hands it to the ordinary pipeline — it does NOT itself validate or
; paper over an ill-typed program. So a STRUCTURALLY-reconstructable but ILL-TYPED eval argument reconstructs to
; ordinary source that the type-checker then rejects with the SAME diagnostic that hand-written source would get.
; These pin that soundness: the reconstruct's job is faithful denotation, and semantic errors surface as the
; ordinary CDZ codes rather than being swallowed into a wrong value. Guards against a future reconstruct change
; that silently accepted (mis-folded) an ill-typed constructed AST instead of letting the checker reject it.
(case
  "eval of an Ast.List with a non-operator head is the ordinary application type error"
  (doc
    "`(eval (Ast.List (list (Ast.Int 1) (Ast.Int 2))))` reconstructs to the source `(1 2)` — an Int64
           applied as if it were a function. The eval desugar reconstructs faithfully; the reconstructed
           source is then type-checked like any other, so it is rejected as CDZ0201 (cannot apply a value of
           type Int64 — it is not a function), exactly as hand-written `(1 2)` is. Pins that a malformed
           (non-operator-headed) constructed AST surfaces the ORDINARY application type error via faithful
           reconstruction, not a silent success or a bespoke eval-only failure.")
  (input (eval (Ast.List #list((Ast.Int 1) (Ast.Int 2)))))
  (error CDZ0201))

(case
  "eval of a constructor-built Ast.List with a valid operator head computes its value"
  (doc
    "The SUCCESS mirror of the non-operator-head case above, on the same Ast.List shape: `(eval
           (Ast.List (list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2))))` — an AST built ENTIRELY by hand
           from constructors (no quote anywhere) — reconstructs to `(+ 1 2)` and evaluates to 3. The
           eval-to-value pins elsewhere reach eval only via `quote` (a different producer of the same Ast
           data); the constructor path was pinned only on the ERROR side (the case above proves it
           faithfully rejects a bad head). This proves it faithfully SUCCEEDS on a good one — catching a
           reconstruct regression that broke constructor-built operator application while leaving the
           quote path intact. Expected: 3.")
  (input (eval (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))))
  (output (: 3 Int64)))

(case
  "eval of a bare Ast.Name for an unbound name is the ordinary unbound-name error"
  (doc
    "`(eval (Ast.Name \"nonexistent\"))` reconstructs to the bare name `nonexistent` as a program.
           The reconstruct is faithful; the reconstructed name is resolved like any other reference, so an
           unbound one is rejected as CDZ0101 (unbound name), exactly as a hand-written bare `nonexistent`
           is. Pins that eval of a name-denoting AST goes through ordinary NAME RESOLUTION — the reconstruct
           does not invent a binding or swallow the reference — so an unbound program name is the ordinary
           unbound-name error, not an eval-specific one.")
  (input (eval (Ast.Name "nonexistent")))
  (error CDZ0101))

(case
  "quoting an empty compound produces an empty Ast.List"
  (doc
    "`(quote ())` reifies the empty compound `()` to an EMPTY `Ast.List` — the reifier maps a
           parenthesized form to `Ast.List` of its reified elements, and zero elements give an empty list
           (NOT a reify error, and NOT a leaf). `List.len` of its elements is 0. The source-level companion
           of the constructor-built `(Ast.List (list))`: this is the very value the eval-malformed case
           above traps on, so it pins where that empty list COMES FROM — a quoted empty compound is a
           well-formed (if operator-less) Ast, distinct from a leaf or a rejected form.")
  (input (match (quote ()) ((Ast.List es) (List.len es)) (_ -1)))
  (output (: 0 Int64)))

(case
  "quasiquote constructs AST with selective evaluation"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           `<template> quotes like quote, but ,<expr> evaluates <expr> normally and inserts result
           into the AST being constructed. `(+ ,x 10) with x=2 produces AST for (+ 2 10), not (+ x 10).
           This is construction, not eval — ,x evaluates the variable x, not an AST.")
  (input (let ((x 2)) (quasiquote (+ (unquote x) 10))))
  (output (: (Ast.List #list((Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

(case
  "unquote in quasiquote evaluates normally and embeds"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,<expr> evaluates <expr> normally (not as AST) and embeds the result.
           `(+ ,(+ 1 1) 10) evaluates (+ 1 1) to 2, constructs AST with that value.")
  (input (quasiquote (+ (unquote (+ 1 1)) 10)))
  (output (: (Ast.List #list((Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

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
(case
  "an active unquote of a boolean literal lifts to an Ast.Bool node"
  (doc
    "`` `(f ,true) `` embeds the boolean literal `true` as the `Ast.Bool` leaf its value denotes —
           the same node `(quote (f true))` builds — so it equals `(Ast.List (list (Ast.Name \"f\")
           (Ast.Bool true)))` (metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           the unquote inserts its result). The boolean companion of the integer embed case above.")
  (input (= (quasiquote (f (unquote true))) (Ast.List #list((Ast.Name "f") (Ast.Bool true)))))
  (output (: true Bool)))

(case
  "an active unquote of a string literal lifts to an Ast.Str node"
  (doc
    "`` `(f ,\"x\") `` embeds the string literal `\"x\"` as the `Ast.Str` leaf — the same node
           `(quote (f \"x\"))` builds — so it equals `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"x\")))`.
           The string companion; pins that the active-unquote lift dispatches on the operand's value kind
           (a string literal → `Ast.Str`, not the `Ast.Int` the integer/runtime path uses).")
  (input (= (quasiquote (f (unquote "x"))) (Ast.List #list((Ast.Name "f") (Ast.Str "x")))))
  (output (: true Bool)))

(case
  "an active-unquoted boolean literal equals the quoted form"
  (doc
    "The unquote-vs-quote agreement for the boolean form: `` `(f ,true) `` and `(quote (f true))`
           build the SAME `Ast` value (both `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`), so they
           are structurally equal (core-semantics.md #Equality Is Structural). An active unquote of a
           literal produces the same node quote of that literal does.")
  (input (= (quasiquote (f (unquote true))) (quote (f true))))
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
(case
  "an active unquote of an Ast-valued expression splices the subtree as identity"
  (doc
    "The canonical AST-building macro: `(def (wrap sub) `(+ ,sub 1))` embeds a COMPUTED sub-AST into
           a template. When the unquoted value is ALREADY an `Ast`, \"insert its result\" splices that node
           AS-IS — NOT re-wrapped in `Ast.Int` (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation). `(wrap (Ast.Int 9))` builds `(+ 9 1)` — a 3-element `Ast.List` — so `List.len` is 3.
           Pins the identity lift the compiler/macro layer needs; previously this type-errored (CDZ0201,
           Ast against Ast.Int's Int64 payload).")
  (input
    (do
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
(case
  "eval of a template splicing a runtime VALUE folds — the working side of the boundary"
  (doc
    "The positive face: a splice whose operand is an ordinary runtime value reconstructs as source and
           folds. `(main n) = (eval `(+ ,n 1))` with runtime n=6 → 7 — the operand `n` is spliced as source,
           not as an `Ast` node, so the reconstructed `(+ n 1)` evaluates. Contrast the Ast-value-splice
           decline below.")
  (input (do (def (main (: n Int64)) (eval (quasiquote (+ (unquote n) 1)))) (export main)))
  (call main (: 6 Int64))
  (output (: 7 Int64)))

(case
  "eval through a splice whose operand is itself an Ast value is rejected with CDZ0201 (Ast in a numeric position)"
  (doc
    "The boundary of the optional eval surface: `(eval `(+ ,(quote (* 2 3)) 1))` is REJECTED with CDZ0201.
           The unquote operand `(quote (* 2 3))` is itself an `Ast` value, so eval's static source
           reconstruction produces `(+ <Ast-value> 1)` — an `Ast` in a numeric position, a type error
           (CDZ0201). Evaluating it would need a nested RUNTIME AST interpreter (metaprogramming.md marks
           runtime eval OPTIONAL; the seed folds at compile time). A genuine user-facing reject with a code
           (corpus = impl-independent spec) — the construction splices the subtree fine (case above) and eval
           of the hand-built equal tree works; only the static reconstruction does not see through a spliced
           Ast value. Breaker-found; ruled a deliberate limit; v-deferral grade-confirmed the emitted CDZ0201.")
  (input (eval (quasiquote (+ (unquote (quote (* 2 3))) 1))))
  (error CDZ0201))

(case
  "a bare Ast literal used as an arithmetic operand names the compile-time-metadata misuse"
  (doc
    "The bare-literal companion of the eval-splice case above (migrated from rcdzc
           an_ast_operand_in_arithmetic_names_the_compile_time_metadata_misuse): a `(quote x)` Ast value used
           directly in a numeric position — `(+ (quote x) 1)` — is CDZ0201, and the message names the real
           category (an `Ast` value is compile-time metadata, not a runtime value to splice), NOT the generic
           `a Ast and an Int64 are different types` cross-type clash. Pins the diagnostic QUALITY: an Ast in
           arithmetic is a metadata misuse, however the Ast got there (a bare literal here, an eval
           reconstruction above).")
  (input (+ (quote x) 1))
  (error CDZ0201 (message "compile-time metadata") (message "runtime splice") (not "a Ast and")))

; The boundary is specifically the EVAL/execution surface — not the spliced Ast value, which is a
; perfectly well-formed tree. The SAME template that `eval` declines above is handled by the NON-executing
; interchange paths: `print` renders it and `Ast.encode`/`Ast.decode` round-trip it. These pin that an
; Ast-value-spliced template is a valid AST (it is only RUNNING it as code that hits the optional-runtime-
; eval line), so a future reader does not mistake the eval decline for a malformed template.
(case
  "print renders a template that splices an Ast value — the non-executing path works"
  (doc
    "The same template `eval` declines above prints fine: `(quasiquote (+ ,(quote (* 2 3)) 1))` splices
           the quoted subtree `(* 2 3)` at its position, and `print` renders the whole as `\"(+ (* 2 3) 1)\"`.
           Pins that the Ast-value splice builds a WELL-FORMED tree (the eval limit is the execution surface,
           not the construction) — `print` reads through the spliced subtree with no decline.")
  (input (= (Ast.print (quasiquote (+ (unquote (quote (* 2 3))) 1))) "(+ (* 2 3) 1)"))
  (output (: true Bool)))

(case
  "an Ast-value-spliced template round-trips through encode and decode"
  (doc
    "The byte-path companion: the template `eval` declines encodes and decodes back equal. `Ast.encode`/
           `Ast.decode` treat the spliced `(* 2 3)` subtree as ordinary nested AST structure, so the
           bijection holds over it — confirming the spliced value is a valid AST the interchange paths handle,
           and only the eval/execution surface has the (optional-runtime-eval) limit.")
  (input
    (match
      (Ast.decode (Ast.encode (quasiquote (+ (unquote (quote (* 2 3))) 1))))
      ((Ok a) (= a (quasiquote (+ (unquote (quote (* 2 3))) 1))))
      ((Err _) false)))
  (output (: true Bool)))

; The metacircular face of the same optional-runtime-eval boundary: `eval` executes only a COMPILE-TIME-
; VISIBLE `Ast` construction (a `(quote …)` / literal `Ast.*`) — NOT the result of ANOTHER `eval`. Nesting
; `eval` around an inner `eval` gives the outer one a non-constant argument (an `eval` APPLICATION), so it
; refuses (CDZ0101) rather than running a dynamically-produced AST. This is the nested-eval ENTRY PATH to
; the "no runtime AST interpreter" line the Ast-value-splice case above pins by a different entry — both
; land on the same spec sentence (metaprogramming.md: "the compiler … does not execute dynamically-
; constructed AST"). A coded reject, so a future change that made the outer eval silently reconstruct the
; inner eval's runtime result (a miscompile — running un-analyzed AST) would flip this to a value and trip.
(case
  "eval does not execute the result of a nested eval — no runtime AST interpreter"
  (doc
    "`(eval (eval (quote (quote (+ 2 3)))))` is rejected CDZ0101. The OUTER `eval`'s argument is the
           inner `(eval (quote (quote (+ 2 3))))` — an `eval` APPLICATION, i.e. a runtime / non-constant
           construction, not a `(quote …)` or literal `Ast.*` the reconstructor sees through. `eval`
           executes only a compile-time-visible AST construction (it reconstructs the source that AST
           denotes and compiles it); it does NOT run the dynamically-produced result of another `eval`
           (metaprogramming.md marks runtime eval OPTIONAL — the seed folds at compile time, it ships no
           runtime AST interpreter). The metacircular companion of the Ast-value-splice boundary above:
           same spec line, different entry path (nested eval vs a spliced Ast operand). A CODED reject, so
           a future change that silently executed the inner eval's runtime result — running un-analyzed
           AST — would flip this to a value and trip the gate.")
  (input (eval (eval (quote (quote (+ 2 3))))))
  (error CDZ0101))

(case
  "a quote carrying a symbol literal reifies to an Ast.Symbol value"
  (doc
    "`(quote #\"hi\")` reifies to the `Ast` value `(Ast.Symbol #\"hi\")` — the symbol is captured as
           data (DISTINCT from `Ast.Name`'s identifier String and `Ast.Str`'s String). This case previously
           pinned a DECLINE (the `Ast` sum had no `Ast.Symbol` variant, so reify's leaf-dispatch bailed);
           the operator directive to make quote/reflection TOTAL over syntax leaves added `Ast.Char` +
           `Ast.Symbol`, so reify now captures the symbol instead of bailing — exactly the flip this case's
           former doc predicted for 'the day an Ast.Symbol variant lands'.")
  (input (= (quote #"hi") (Ast.Symbol #"hi")))
  (output (: true Bool)))

(case
  "eval of a quote carrying a symbol literal is rejected CDZ0101 — the reify bail as a perf-bound tripwire"
  (doc
    "`(eval (quote (Qty.of 5 (Unit.of #\"zorks\"))))` is rejected CDZ0101. The rejection is at the QUOTE
           (reify), NOT eval's reconstructor: `Unit.of` takes a `#\"…\"` symbol argument, and quote's reify has
           no `Ast.Symbol` variant to build (the minimal-root case above), so the whole quote bails before eval
           sees a node. A CODED tripwire (not merely a missing feature): the reify bail is the boundary that
           keeps a runtime-synthesized `Unit.of` node from ever existing — exactly the assumption a
           node-count-bounded unknown-units analysis relies on. The day an `Ast.Symbol` variant lands and quote
           reifies the symbol, this flips to a value (a running `(Qty.of 5 (Unit.of #\"zorks\"))`, which would
           then surface the unknown-unit CDZ0201 the direct form already reports, 18-units-of-measure) and trips
           the gate, flagging that the node-count bound now needs re-examining. Companion to the
           no-runtime-AST-interpreter rejection above (also a coded CDZ0101): both are the quote/eval
           reconstruction boundary, different non-reifiable entry point.")
  (input (eval (quote (Qty.of 5 (Unit.of #"zorks")))))
  (error CDZ0101))

(case
  "an active unquote of a let-bound boolean lifts to Ast.Bool by inferred type"
  (doc
    "A RUNTIME operand (a let-bound name) lifts by its inferred type: `b : Bool` → `Ast.Bool`.
           `(let ((b true)) `(f ,b))` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`. Pins the
           runtime-Bool lift (the literal case is above; this exercises the inferred-type path at lower).")
  (input
    (let
      ((b true))
      (= (quasiquote (f (unquote b))) (Ast.List #list((Ast.Name "f") (Ast.Bool true))))))
  (output (: true Bool)))

(case
  "an active unquote of a let-bound string lifts to Ast.Str by inferred type"
  (doc
    "The runtime-String companion: `s : String` → `Ast.Str`. `(let ((s \"hi\")) `(f ,s))` builds
           `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"hi\")))`. Pins the runtime-String inferred-type lift.")
  (input
    (let
      ((s "hi"))
      (= (quasiquote (f (unquote s))) (Ast.List #list((Ast.Name "f") (Ast.Str "hi"))))))
  (output (: true Bool)))

(case
  "an active unquote of a let-bound integer still lifts to Ast.Int"
  (doc
    "Regression guard: a runtime Int64 operand still lifts to `Ast.Int` (the original active-unquote
           behavior, now via the inferred-type path). `(let ((n 42)) `(op-const ,n))` builds
           `(Ast.List (list (Ast.Name \"op-const\") (Ast.Int 42)))`.")
  (input
    (let
      ((n 42))
      (= (quasiquote (op-const (unquote n))) (Ast.List #list((Ast.Name "op-const") (Ast.Int 42))))))
  (output (: true Bool)))

(case
  "an active unquote of a BigInt lifts to Ast.Int (the payload type, no widen)"
  (doc
    "The BigInt companion of the let-bound-integer lift: `Ast.Int`'s payload IS `BigInt`, so an
           operand ALREADY typed `BigInt` lifts to `Ast.Int` by wrapping DIRECTLY — no `Int64`→`BigInt`
           widen (that arm is for a fixed-width Int64 operand). `` `(op-const ,42N) `` builds `(Ast.List
           (list (Ast.Name \"op-const\") (Ast.Int 42N)))`. Regression guard for the `lower_ast_lift`
           `Ty::BigInt` arm — without it a BigInt unquote fell through to decline even though `Ast.Int`
           is the right leaf (the splice-surface gap the Int64→BigInt payload flip introduced).")
  (input
    (= (quasiquote (op-const (unquote 42N))) (Ast.List #list((Ast.Name "op-const") (Ast.Int 42N)))))
  (output (: true Bool)))

(case
  "an active unquote of a RUNTIME BigInt (from arithmetic) lifts to Ast.Int"
  (doc
    "The RUNTIME-value companion of the `,42N` literal lift above: a BigInt produced by runtime
           arithmetic — not a constant — lifts through the `lower_ast_lift` `Ty::BigInt` arm (wrap the
           already-BigInt operand directly, no widen) rather than folding at reify time. `(let ((x (+
           20N 22N))) `(f ,x))` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Int 42)))`; the match reads
           the payload back and narrows it to compare — 42. Exercises the runtime `ast-lift` path for a
           heap BigInt (distinct from the constant-fold `,42N` case), green on all backends. (A BigInt
           supplied as an EXPORTED-ENTRY argument is a separate wasm entry-arg-marshalling gap, not this
           lift — so the runtime BigInt here comes from internal arithmetic.)")
  (input
    (let
      ((x (+ 20N 22N)))
      (match (quasiquote (f (unquote x))) ((Ast.List #list(_ (Ast.Int n))) (Int64.of n)) (_ -1))))
  (output (: 42 Int64)))

(case
  "an active unquote of a computed boolean expression lifts to Ast.Bool"
  (doc
    "A non-leaf (computed) runtime operand lifts by its inferred type too: `(= 1 1) : Bool` →
           `Ast.Bool`. `` `(f ,(= 1 1)) `` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Bool true)))`.
           Pins that the inferred-type lift covers a computed expression, not only a bound name.")
  (input (= (quasiquote (f (unquote (= 1 1)))) (Ast.List #list((Ast.Name "f") (Ast.Bool true)))))
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
(case
  "an unquote of an expression with an unbound name is rejected, not quoted"
  (doc
    "`` `(a ,(+ b 1)) `` unquotes `(+ b 1)`, which references the unbound name `b` — the unquote
           MUST evaluate its expression (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation), so this is the ordinary unbound-name error (CDZ0101, core-semantics.md #Binding
           Is Lexical — unconditional), exactly as the bare `(+ b 1)` is. Pins that an unquote whose
           expression cannot be evaluated is rejected, NOT silently quoted as inert AST: a compiler that
           falls back to quoting the un-evaluable expression (yielding an `(Ast.List …)` for `(+ b 1)`)
           turns the selective-evaluation unquote into a second quote and swallows the scope error. With
           `b` bound (`(let ((b 5)) `(a ,(+ b 1)))`) the unquote evaluates to 6; unbound, it is CDZ0101.")
  (input (quasiquote (a (unquote (+ b 1)))))
  (error CDZ0101))

; An AST built by quasiquote-with-unquote is an ordinary AST VALUE: it must be structurally EQUAL to
; the same AST built any other way, and encode to the same bytes (core-semantics.md #Equality Is
; Structural; the AST is an ordinary sum type — type-system.md #The Abstract Syntax Tree Type Is An
; Ordinary Sum Type). So `` `(f ,x) `` with x=1 equals `` `(f ,1) `` and `(quote (f 1))` — all three
; are `(Ast.List (Ast.Name "f") (Ast.Int 1))`. An unquote that embeds a RUNTIME (let-bound) value
; must build the same `(Ast.Int 1)` node a const fold produces, so structural equality and encoding
; see the two as identical. This is the compiler's own idiom: it builds instruction ASTs by
; quasiquoting runtime values, then compares/encodes them.
(case
  "an AST from quasiquoting a runtime value equals the same AST built by quote"
  (doc
    "`` `(f ,x) `` with x bound to 1 builds `(Ast.List (Ast.Name \"f\") (Ast.Int 1))`, the same
           AST `(quote (f 1))` builds — so they are structurally equal (core-semantics.md #Equality Is
           Structural). An unquote that embeds a runtime value produces the same node as a const fold,
           so the two compare equal. MUST be true.")
  (input (let ((x 1)) (= (quasiquote (f (unquote x))) (quote (f 1)))))
  (output (: true Bool)))

(case
  "quasiquotes unquoting a runtime variable and a literal build equal ASTs"
  (doc
    "The companion isolating the runtime-vs-const embedding: `` `(f ,x) `` (x=1, a runtime local)
           and `` `(f ,1) `` (a literal) build the same AST and MUST be equal — the runtime-unquoted
           node is structurally identical to the const-unquoted one.")
  (input (let ((x 1)) (= (quasiquote (f (unquote x))) (quasiquote (f (unquote 1))))))
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
(case
  "a quoted integer equals the same node built by the Ast.Int constructor"
  (doc
    "`(quote 42)` is the AST sum value `(Ast.Int 42)` (metaprogramming.md #Quote Produces An AST
           Value — quote evaluates to an AST SUM value). The corpus records quote outputs in exactly
           this constructor form and matches `(quote 42)` as `(Ast.Int n)` binding 42, so the two
           denote ONE sum value and structural equality MUST be true. A representation that stores a
           quote result differently from an applied Ast.* constructor — comparing them unequal — splits
           the single AST value form the encoding bijection is defined over. MUST be true.")
  (input (= (quote 42) (Ast.Int 42)))
  (output (: true Bool)))

(case
  "a quoted name equals the same node built by the Ast.Name constructor"
  (doc
    "The Name companion: `(quote foo)` is `(Ast.Name \"foo\")` — a quoted bare name is the
           Ast.Name sum value carrying the name as a String payload (metaprogramming.md #Quote Produces
           An AST Value). `(= (quote foo) (Ast.Name \"foo\"))` MUST be true, exactly as the Int case.
           Pins that the quote-vs-constructor equality holds for the leaf name node too.")
  (input (= (quote foo) (Ast.Name "foo")))
  (output (: true Bool)))

; --- The Ast.Bool leaf variant --------------------------------------------------------------------
; The built-in `Ast` is an ordinary sum type with "a variant per syntactic form (an integer, a float, a
; string, a BOOLEAN, a name, and a list of child nodes)" (type-system.md #The Abstract Syntax Tree Type
; Is An Ordinary Sum Type). A BOOLEAN literal is one such form, so `(quote true)` is the `Ast` sum value
; `(Ast.Bool true)` — the boolean companion of `(quote 42)`=`(Ast.Int 42)` and `(quote foo)`=`(Ast.Name
; "foo")`. It carries a `Bool` payload (a single-arity variant constructor whose argument is type-checked,
; like every other `Ast.*`), it destructures by pattern match binding that payload, it round-trips through
; `Ast.encode`/`Ast.decode` and `print`/`read`, and `eval` executes it (a boolean form evaluates to itself).
(case
  "a quoted boolean equals the same node built by the Ast.Bool constructor"
  (doc
    "The boolean companion of the Int/Name equality cases: `(quote true)` is the `Ast` sum value
           `(Ast.Bool true)` (metaprogramming.md #Quote Produces An AST Value; type-system.md #The Abstract
           Syntax Tree Type Is An Ordinary Sum Type — a boolean is a syntactic form). `(= (quote true)
           (Ast.Bool true))` MUST be true (core-semantics.md #Equality Is Structural), exactly as
           `(= (quote 42) (Ast.Int 42))` is — the quote result and the constructor-built node are ONE value.")
  (input (= (quote true) (Ast.Bool true)))
  (output (: true Bool)))

(case
  "a match binds an Ast.Bool payload"
  (doc
    "The `Ast` sum is deconstructible by pattern matching like any other sum (type-system.md #The
           Abstract Syntax Tree Type Is An Ordinary Sum Type), so a match over `(quote false)` binds the
           `Ast.Bool` payload. The arm returns the bound boolean; the catch-all covers the other variants
           (the match is exhaustive against the sum's variant set). Yields false.")
  (input (match (quote false) ((Ast.Bool b) b) (_ true)))
  (output (: false Bool)))

(case
  "a built-in Ast.Bool constructor applied to a wrong-type payload is a type error"
  (doc
    "`Ast.Bool`'s payload type is Bool (a variant per syntactic form — type-system.md #The Abstract
           Syntax Tree Type Is An Ordinary Sum Type), so `(Ast.Bool 5)` applies it to an Int64 — a type
           mismatch the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")` (a String where Int64
           is declared) is. Pins that the built-in `Ast.Bool` constructor type-checks its declared payload
           like any user sum variant.")
  (input (Ast.Bool 5))
  (error CDZ0201))

(case
  "a quoted compound form containing a boolean reifies with an Ast.Bool element"
  (doc
    "A boolean nested inside a quoted compound reifies as an `Ast.Bool` element, exactly as an
           integer reifies as `Ast.Int`. `(quote (f true))` is `(Ast.List (list (Ast.Name \"f\") (Ast.Bool
           true)))`, so comparing it against that hand-built node MUST be true — the leaf reification is
           structural and covers the boolean form.")
  (input (= (quote (f true)) (Ast.List #list((Ast.Name "f") (Ast.Bool true)))))
  (output (: true Bool)))

(case
  "eval of a quoted boolean executes it to the boolean value"
  (doc
    "eval executes an AST value as code (metaprogramming.md #Eval Is Optional For Macros And
           Interactive Use); a boolean form evaluates to itself, so `(eval (quote true))` runs to true.
           The boolean companion of `(eval (quote (+ 1 2)))`=3 — `eval` reconstructs the source the
           `Ast.Bool` denotes (the `true` literal) and folds it through the ordinary path.")
  (input (do (def (main) (eval (quote true))) (export main)))
  (output (: true Bool)))

(case
  "encoding and decoding an Ast.Bool round-trips to an equal value"
  (doc
    "`(Ast.Bool true)` is an AST value; encoding then decoding it MUST yield an equal AST
           (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t), exactly as the Int/Name/
           List round-trips do. `Ast.decode : Bytes → Result<Ast, _>` is total, so the round-trip matches
           the `Ok` arm and equates its payload.")
  (input
    (match (Ast.decode (Ast.encode (Ast.Bool true))) ((Ok a) (= a (Ast.Bool true))) ((Err _) false)))
  (output (: true Bool)))

(case
  "print of an Ast.Bool renders the bare word and read inverts it"
  (doc
    "`print : Ast → String` renders an `Ast.Bool` as the bare word `true`/`false` — the canonical
           re-readable spelling — and `read : String → Ast` parses it back, so `read(Ast.print v) == v`
           (compiler-pipeline.md — the printer and reader are inverse over the AST value). A boolean word
           is unambiguously a boolean literal (never a name), so the round-trip is exact.")
  (input (= (Ast.read (Ast.print (Ast.Bool false))) (Ast.Bool false)))
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
(case
  "a quoted string equals the same node built by the Ast.Str constructor"
  (doc
    "`(quote \"hi\")` is the `Ast` sum value `(Ast.Str \"hi\")` (metaprogramming.md #Quote Produces
           An AST Value; type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type — a string
           is a syntactic form). `(= (quote \"hi\") (Ast.Str \"hi\"))` MUST be true (core-semantics.md
           #Equality Is Structural), the string companion of the Int/Bool/Name equality cases.")
  (input (= (quote "hi") (Ast.Str "hi")))
  (output (: true Bool)))

(case
  "a quoted string is distinct from the same text quoted as a name"
  (doc
    "`Ast.Str` (a string LITERAL) and `Ast.Name` (an identifier reference) are different variants
           even though both carry a String payload. `(quote \"foo\")` is `(Ast.Str \"foo\")`, NOT
           `(Ast.Name \"foo\")`, so comparing them is FALSE — the reifier maps a string literal and a bare
           name to distinct forms. Pins that a string is not collapsed to a name (they are separate
           syntactic forms).")
  (input (= (quote "foo") (Ast.Name "foo")))
  (output (: false Bool)))

(case
  "a match binds an Ast.Str payload"
  (doc
    "The `Ast` sum is deconstructible by pattern matching (type-system.md #The Abstract Syntax Tree
           Type Is An Ordinary Sum Type), so a match over `(quote \"hey\")` binds the `Ast.Str` payload —
           the String literal — and `String.byte-len` of it is 3. The catch-all covers the other variants.")
  (input (match (quote "hey") ((Ast.Str s) (String.byte-len s)) (_ 0)))
  (output (: 3 Int64)))

(case
  "a built-in Ast.Str constructor applied to a wrong-type payload is a type error"
  (doc
    "`Ast.Str`'s payload type is String, so `(Ast.Str 5)` applies it to an Int64 — a type mismatch
           the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")` and `(Ast.Bool 5)` are. Pins
           that the built-in `Ast.Str` constructor type-checks its declared payload like any sum variant.")
  (input (Ast.Str 5))
  (error CDZ0201))

(case
  "a quoted compound form containing a string reifies with an Ast.Str element"
  (doc
    "A string nested inside a quoted compound reifies as an `Ast.Str` element. `(quote (f \"x\"))` is
           `(Ast.List (list (Ast.Name \"f\") (Ast.Str \"x\")))` — the head `f` is a name, the argument
           `\"x\"` a string literal — so comparing it against that hand-built node MUST be true. Pins that
           the string leaf reifies structurally inside a list, distinct from the head name.")
  (input (= (quote (f "x")) (Ast.List #list((Ast.Name "f") (Ast.Str "x")))))
  (output (: true Bool)))

(case
  "eval of a quoted string executes it to the string value"
  (doc
    "eval executes an AST value as code (metaprogramming.md #Eval Is Optional For Macros And
           Interactive Use); a string form evaluates to itself, so `(eval (quote \"abcd\"))` runs to the
           string `\"abcd\"` — `String.byte-len` of it is 4. The string companion of `(eval (quote true))`
           — `eval` reconstructs the source the `Ast.Str` denotes (the string literal) and folds it.")
  (input (do (def (main) (String.byte-len (eval (quote "abcd")))) (export main)))
  (output (: 4 Int64)))

(case
  "encoding and decoding an Ast.Str round-trips to an equal value"
  (doc
    "`(Ast.Str \"hi\")` is an AST value; encoding then decoding it MUST yield an equal AST
           (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t), exactly as the Int/Bool/
           Name/List round-trips do. `Ast.decode` is total, so the round-trip matches the `Ok` arm.")
  (input
    (match (Ast.decode (Ast.encode (Ast.Str "hi"))) ((Ok a) (= a (Ast.Str "hi"))) ((Err _) false)))
  (output (: true Bool)))

; --- The Name text round-trip is scoped to grammatically-valid identifiers; the byte codec is total ---
; `print` renders an `Ast.Name` as its bare word, and `read` classifies a bare token by the language's
; number/identifier boundary: a DIGIT-LED token is a NUMBER (spec/learnings — a digit-led token is a
; number, never an identifier). So an `Ast.Name` whose spelling is digit-led (`"1.5"`, `"123"`) — a name
; that CANNOT arise from parsing real source, since no valid identifier starts with a digit — prints as
; that numeric text and reads back as `Ast.Float`/`Ast.Int`, not the original `Name`. This is the correct
; grammar behavior, not a bug: the TEXT round-trip `read(Ast.print v) == v` holds for well-formed names (a valid
; identifier). The BYTE codec is total over ANY name string — its tag delimits the payload — so a digit-led
; name still round-trips through `encode`/`decode`. These pin the boundary so it can't silently change and
; so the two interchange paths' differing domains are explicit. (Found bug-hunting; the printer docstring
; was corrected from an unconditional round-trip claim to this scoped one.)
(case
  "the byte codec round-trips a digit-led Ast.Name that the text path would reclassify"
  (doc
    "`Ast.encode`/`Ast.decode` is total over any `Name` string: `(Ast.Name \"1.5\")` — a name spelled
           like a float — round-trips to an EQUAL `Ast.Name` through the byte path, because the Name tag
           delimits its payload (no re-lexing). Contrast the text path below, which reclassifies it. Pins
           that the codec's domain is every name, digit-led or not.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Name "1.5")))
      ((Ok a) (= a (Ast.Name "1.5")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "print then read of a digit-led Ast.Name reclassifies it per the number/identifier boundary"
  (doc
    "The TEXT round-trip is scoped to grammatically-valid identifiers. `print (Ast.Name \"1.5\")`
           renders the bare word `1.5`, and `read` classifies a digit-led token as a NUMBER (the language's
           number/identifier boundary — no valid identifier is digit-led), so it comes back as an
           `Ast.Float`, not the original `Ast.Name`. This is correct grammar behavior, NOT a round-trip bug:
           `Ast.Name \"1.5\"` is a name that could never be parsed from source. Pins the boundary (matched
           via the Float arm) so a future printer/reader change is a deliberate decision, not an accident.")
  (input (match (Ast.read (Ast.print (Ast.Name "1.5"))) ((Ast.Float _) 1) ((Ast.Name _) 2) (_ 0)))
  (output (: 1 Int64)))

(case
  "read of a leading-'+' integer token is an Ast.Name, not an Ast.Int (within i64)"
  (doc
    "A leading `+` is NEVER part of a numeric literal in Cadenza: the front-end lexer makes `+` an
           operator (Kind::Plus) always and begins a number only on a digit, so `read` — the printer's
           inverse and a mirror of that reader — classifies `+5` as an `Ast.Name`, not an `Ast.Int 5`.
           (`print` never emits a leading `+`, so this is not a round-trip case; it pins the reader's sign
           handling directly.) Without the guard `str::parse::<i64>` would accept the `+` and mis-read it as
           an integer. Matched via the Name arm. (v-syntax ruling A.)")
  (input (match (Ast.read "+5") ((Ast.Name _) 1) ((Ast.Int _) 2) (_ 0)))
  (output (: 1 Int64)))

(case
  "read of a leading-'+' BEYOND-i64 integer token is an Ast.Name too (both sides of the boundary)"
  (doc
    "The beyond-i64 companion of the leading-`+` case: a `+`-prefixed 26-digit token is ALSO an
           `Ast.Name`, not an `Ast.Int`. This is the exact spot the pre-ruling inconsistency lived — `+5`
           read as an Ast.Int (i64 fast path accepts `+`) while `+<beyond-i64>` fell through the
           arbitrary-precision path (which strips only `-`) to a Name. Pinning BOTH sides of i64 witnesses
           that the leading-`+`→Name classification is now uniform across the boundary. (v-syntax ruling A.)")
  (input (match (Ast.read "+99999999999999999999999999") ((Ast.Name _) 1) ((Ast.Int _) 2) (_ 0)))
  (output (: 1 Int64)))

; The keyword companion of the digit-led boundary: `true`/`false` are BOOLEAN literals in the grammar, not
; identifiers, so the same text-round-trip scoping applies. `print (Ast.Name "true")` emits the bare word
; `true`, which `read` classifies as `Ast.Bool` (the reader's keyword arm) — not the original `Ast.Name`.
; Like a digit-led name, `Ast.Name "true"` cannot arise from parsing real source (the lexer yields `true`
; as a boolean, never a name). The byte codec is total over it. (These correct the reader's comment that
; claimed "a name can never collide" — a HAND-CONSTRUCTED keyword/numeric-spelled name can, and the text
; round-trip is scoped to grammatically-valid identifiers accordingly.)
(case
  "the byte codec round-trips a keyword-spelled Ast.Name that the text path would reclassify"
  (doc
    "`Ast.encode`/`Ast.decode` is total over a name spelled like a keyword: `(Ast.Name \"true\")`
           round-trips to an EQUAL `Ast.Name` through the byte path (its tag delimits the payload, no
           re-lexing). The keyword companion of the digit-led byte-codec case; contrast the text path below.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Name "true")))
      ((Ok a) (= a (Ast.Name "true")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "print then read of a keyword-spelled Ast.Name reclassifies it as the boolean literal"
  (doc
    "`print (Ast.Name \"true\")` renders the bare word `true`, which `read` classifies as `Ast.Bool`
           (the reader's keyword arm — `true`/`false` are boolean literals, not identifiers), NOT the
           original `Ast.Name`. Like the digit-led case, `Ast.Name \"true\"` can't arise from source (the
           lexer never yields a name spelled `true`). Correct grammar behavior, not a bug — the text
           round-trip is scoped to grammatically-valid identifiers. Matched via the Bool arm.")
  (input (match (Ast.read (Ast.print (Ast.Name "true"))) ((Ast.Bool _) 1) ((Ast.Name _) 2) (_ 0)))
  (output (: 1 Int64)))

; The STRING-SPELLED companion of the digit-led / keyword-spelled name-reclassification boundary: a name
; whose SPELLING is a quote-delimited string (`Ast.Name "\"x\""`, the two-character text `"x"` including
; the quotes) prints as the bare word `"x"`, which the reader lexes as a STRING literal — so `read`
; returns `Ast.Str`, not the original `Ast.Name`. Like a digit-led or keyword name, such a name cannot
; arise from parsing source (the lexer yields a string, never a name). Correct grammar behavior, not a
; round-trip bug — the text round-trip is scoped to grammatically-valid identifiers. The byte codec, by
; contrast, is total over it (its tag delimits the payload, no re-lexing), completing the trio.
(case
  "the byte codec round-trips a string-spelled Ast.Name that the text path would reclassify"
  (doc
    "`Ast.encode`/`Ast.decode` is total over a name whose spelling looks like a string literal:
           `(Ast.Name \"\\\"x\\\"\")` (text `\"x\"`, quotes included) round-trips to an EQUAL `Ast.Name`
           through the byte path — the tag delimits the payload, so no re-lexing reclassifies it. The
           string-lookalike companion of the digit-led / keyword-spelled byte-codec cases; contrast the
           text path below where the quotes make `read` see a string.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Name "\"x\"")))
      ((Ok a) (= a (Ast.Name "\"x\"")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "print then read of a string-spelled Ast.Name reclassifies it as an Ast.Str"
  (doc
    "`print (Ast.Name \"\\\"x\\\"\")` renders the bare word `\"x\"` (the name's spelling includes the
           quote characters), which `read` lexes as a STRING literal → `Ast.Str`, NOT the original
           `Ast.Name`. Like the digit-led and keyword-spelled cases, `Ast.Name \"\\\"x\\\"\"` cannot arise
           from source (the lexer yields a string for quote-delimited text, never a name). Correct grammar
           behavior, not a bug — the text round-trip is scoped to grammatically-valid identifiers. Matched
           via the Str arm; completes the reclassification trio (number / keyword / string spelling).")
  (input (match (Ast.read (Ast.print (Ast.Name "\"x\""))) ((Ast.Str _) 1) ((Ast.Name _) 2) (_ 0)))
  (output (: 1 Int64)))

(case
  "print of an Ast.Str renders a quoted literal with escapes and read inverts it"
  (doc
    "`print : Ast → String` renders an `Ast.Str` as a `\"…\"` literal, escaping the closed set
           (`\\n \\t \\r \\\\ \\\"`) — the canonical re-readable spelling — and `read : String → Ast`
           parses it back, so `read(Ast.print v) == v` (compiler-pipeline.md — printer and reader are inverse).
           The payload here holds an embedded quote and newline (`a\"b\\nc`), so this pins the escape
           round-trip, not just plain text — distinct from `Ast.Name`, which prints the bare word.")
  (input (= (Ast.read (Ast.print (Ast.Str "a\"b
c"))) (Ast.Str "a\"b
c")))
  (output (: true Bool)))

; --- Ast.Str / cross-variant round-trip EDGES (pinning invariants so a change can't quietly flip them) ---
; The `Ast.Str` leaf round-trips through BOTH interchange paths (`print`/`read`, `Ast.encode`/`Ast.decode`)
; over the full payload range — empty, multibyte UTF-8, every escape, a keyword-colliding spelling — and a
; compound nesting ALL SIX leaf kinds round-trips too. These already hold; pinned here so a future change
; to the escape set, byte layout, or reader can't silently break a leaf (ast-encoding.md #The Encoding Is
; A Bijection; compiler-pipeline.md — printer/reader inverse).
(case
  "an empty-string Ast.Str round-trips through print and read"
  (doc
    "The empty string is a valid `Ast.Str` payload — `print` renders `\"\"`, `read` parses it back.
           Pins the zero-length edge of the escape/quote rendering.")
  (input (= (Ast.read (Ast.print (Ast.Str ""))) (Ast.Str "")))
  (output (: true Bool)))

(case
  "an empty-string Ast.Str round-trips through encode and decode"
  (doc
    "The byte-path companion: an empty `Ast.Str` (length-prefix 0) encodes and decodes back equal
           (ast-encoding.md #The Encoding Is A Bijection).")
  (input (match (Ast.decode (Ast.encode (Ast.Str ""))) ((Ok a) (= a (Ast.Str ""))) ((Err _) false)))
  (output (: true Bool)))

(case
  "a multibyte-UTF-8 Ast.Str round-trips through encode and decode"
  (doc
    "The byte-path companion of the multibyte print/read case: `\"héllo☃\"` (6 scalars, 10 UTF-8
           bytes) encodes and decodes back equal. The Str encoding is a length-prefix over the UTF-8 BYTES,
           so this pins that the prefix counts BYTES, not characters — every existing encode/decode Str case
           is ASCII (`\"\"`, `\"hi\"`, `\"x\"`) where byte-len == char-count and cannot distinguish the two.
           A codec that wrote a char-count length would pass those yet truncate or over-read this string.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Str "héllo☃")))
      ((Ok a) (= a (Ast.Str "héllo☃")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "a multibyte-UTF-8 Ast.Str round-trips through print and read"
  (doc
    "A string with non-ASCII scalars (`héllo☃` — 2- and 3-byte UTF-8) round-trips: the escape set
           touches only ASCII, so a multibyte scalar passes through and reads back intact. Pins the
           reader/printer are byte-faithful over UTF-8.")
  (input (= (Ast.read (Ast.print (Ast.Str "héllo☃"))) (Ast.Str "héllo☃")))
  (output (: true Bool)))

(case
  "an all-escapes Ast.Str round-trips through print and read"
  (doc
    "A payload with EVERY member of the closed escape set (`\\t \\r \\n \\\\ \\\"`) round-trips —
           each escaped on print, un-escaped on read. Pins the whole escape set at once, guarding against
           dropping or mis-pairing any one escape.")
  (input (= (Ast.read (Ast.print (Ast.Str "\t\r
\\\""))) (Ast.Str "\t\r
\\\"")))
  (output (: true Bool)))

(case
  "a string spelled like a keyword round-trips as an Ast.Str, not an Ast.Bool or Ast.Name"
  (doc
    "🔑 The disambiguation pin: the STRING `\"true\"` is an `Ast.Str`, not the boolean word or a
           name. `print` renders it QUOTED (`\"true\"`), so `read` parses it back as a string literal —
           never the `Ast.Bool` a bare `true` word reads as, nor an `Ast.Name`. Guards the print/read
           boundary between a quoted string and a bare keyword.")
  (input (= (Ast.read (Ast.print (Ast.Str "true"))) (Ast.Str "true")))
  (output (: true Bool)))

(case
  "a deep compound nesting all six leaf kinds round-trips through encode and decode"
  (doc
    "A compound nesting every realized leaf — `(Ast.List (Ast.Name \"f\") (Ast.Str \"x\") (Ast.Bool
           true) (Ast.Float 1.5) (Ast.List (Ast.Int 1)))` — round-trips through encode/decode to an equal
           value. Pins that Str/Bool/Float/Int/Name/List interleave correctly in one tree (each tag is
           self-delimiting), not just as standalone leaves.")
  (input
    (match
      (Ast.decode
        (Ast.encode
          (Ast.List
            #list((Ast.Name "f")
              (Ast.Str "x")
              (Ast.Bool true)
              (Ast.Float 1.5)
              (Ast.List #list((Ast.Int 1)))))))
      ((Ok a)
        (=
          a
          (Ast.List
            #list((Ast.Name "f")
              (Ast.Str "x")
              (Ast.Bool true)
              (Ast.Float 1.5)
              (Ast.List #list((Ast.Int 1)))))))
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
(case
  "a quoted float equals the same node built by the Ast.Float constructor"
  (doc
    "`(quote 1.5)` is the `Ast` sum value `(Ast.Float 1.5)` (metaprogramming.md #Quote Produces An
           AST Value; type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type — a float is a
           syntactic form). `(= (quote 1.5) (Ast.Float 1.5))` MUST be true, the float companion of the
           Int/Bool/Str equality cases.")
  (input (= (quote 1.5) (Ast.Float 1.5)))
  (output (: true Bool)))

(case
  "a quoted float is distinct from the same magnitude quoted as an integer"
  (doc
    "`Ast.Float` (a Float64 payload) and `Ast.Int` (an Int64 payload) are different variants:
           `(quote 3.0)` is `(Ast.Float 3.0)`, NOT `(Ast.Int 3)`, so comparing them is FALSE. Pins that a
           float literal is not collapsed to an integer — distinct syntactic forms with distinct payloads.")
  (input (= (quote 3.0) (Ast.Int 3)))
  (output (: false Bool)))

(case
  "a match binds an Ast.Float payload"
  (doc
    "The `Ast` sum is deconstructible by pattern matching, so a match over `(quote 2.5)` binds the
           `Ast.Float` payload — the Float64 — and comparing it to `2.5` is true. The catch-all covers the
           other variants.")
  (input (match (quote 2.5) ((Ast.Float f) (= f 2.5)) (_ false)))
  (output (: true Bool)))

(case
  "a built-in Ast.Float constructor applied to a wrong-type payload is a type error"
  (doc
    "`Ast.Float`'s payload type is Float64, so `(Ast.Float \"x\")` applies it to a String — a type
           mismatch the compiler MUST reject (CDZ0201), exactly as `(Ast.Int \"x\")`/`(Ast.Bool 5)` are.")
  (input (Ast.Float "x"))
  (error CDZ0201))

; --- A non-canonical float cannot be reified into an Ast.Float (uniform decline) ------------------
; A NaN Float64 has no canonical value form — its bit pattern is rejected at the host canonical-value
; encode boundary. Reifying one into an `Ast.Float` node used to DIVERGE across backends (wasm TRAPped at
; the encode boundary, rust accepted the value), a differential miscompile on an accepted program.
; Operator ruling (A): `(Ast.Float <non-canonical>)` DECLINES uniformly at compile time, matching the
; sibling non-canonical-float-in-AST paths that already decline (`,@` of a NaN list; `(Ast.Float +inf)`,
; whose non-finite arithmetic operand declines upstream). A FINITE float still reifies. These pin the
; CONSTANT half of that fix (a runtime-produced NaN payload is caught at the escape boundary separately).
(case
  "reifying a constant NaN into an Ast.Float is rejected (no canonical value form)"
  (doc
    "`(Ast.Float Float64.nan)` — wrapping a NaN, which has no canonical value form — DECLINES at
           compile time rather than trapping on one backend and returning a value on the other. The
           construction guard rejects a non-canonical constant float payload, so the node is never built.
           Pins the uniform-decline fix (adv-ast-float-nan differential); a bare NaN value still crosses.")
  (input (do (def (main) (Ast.Float Float64.nan)) (export main)))
  (error CDZ0201))

(case
  "reifying a positive-infinity float into an Ast.Float is rejected"
  (doc
    "The +inf companion: `(Ast.Float (/ 1.0 0.0))` declines — the non-finite division has no value
           form (declines upstream), so the reify never gets a canonical payload. Pins that ALL
           non-canonical floats (NaN and infinities) are uniformly kept out of an `Ast.Float` node.")
  (input (do (def (main) (Ast.Float (/ 1.0 0.0))) (export main)))
  (error CDZ0201))

(case
  "reifying a finite float into an Ast.Float is unaffected by the non-canonical guard"
  (doc
    "The control: a FINITE float reifies normally — `(Ast.Float 2.5)` builds the node and its payload
           reads back as 2.5. Pins that the non-canonical-float guard is surgical (only NaN/inf decline), a
           finite Ast.Float is untouched.")
  (input (match (Ast.Float 2.5) ((Ast.Float f) (= f 2.5)) (_ false)))
  (output (: true Bool)))

(case
  "an active unquote of a constant NaN is rejected (the ast-lift path, consistent with the ctor)"
  (doc
    "The active-unquote lift `,expr` shares the non-canonical-float rule with the direct `Ast.Float`
           ctor: `(quasiquote (f (unquote Float64.nan)))` EVALUATES the NaN and would lift it into an
           `Ast.Float` node, which has no canonical value form. It DECLINES uniformly on both backends,
           closing the same wasm-traps/rust-accepts split via the `ast-lift` path (not just the ctor path).
           A finite unquote (`,2.5`) and a runtime finite float still lift — the guard is surgical.")
  (input (do (def (main) (quasiquote (f (unquote Float64.nan)))) (export main)))
  (error CDZ0201))

(case
  "an active unquote of a finite float lifts to Ast.Float (ast-lift control)"
  (doc
    "The control for the ast-lift guard: `,2.5` lifts to `(Ast.Float 2.5)` normally — reading child 1
           of `(quasiquote (f (unquote 2.5)))` back gives 2.5. Pins that the non-canonical guard on the lift
           path only rejects a NaN, not a finite float.")
  (input
    (match
      (quasiquote (f (unquote 2.5)))
      ((Ast.List xs) (match (List.at xs 1) ((Option.Some (Ast.Float v)) v) (_ 0.0)))
      (_ 0.0)))
  (output (: 2.5 Float64)))

(case
  "a tagged-template tag returning a non-canonical Ast.Float is rejected (the guard fires through the expander)"
  (doc
    "The non-canonical-float guard fires on EVERY path that constructs an `Ast.Float`, including the
           tagged-template expander: a tag function returning `(Ast.Float Float64.nan)` is β-reduced through
           the ordinary ctor path at expansion, so the same construction guard declines it — the tagged
           template does not smuggle a NaN node past the guard. A tag returning a FINITE `Ast.Float` still
           expands (the control below). Pins the ctor-guard × tagged-template-expander interaction.")
  (input
    (do
      (def (bad chunks holes) (Ast.Float Float64.nan))
      (def
        (main)
        (match (tagged-template bad (chunks "x") (holes)) ((Ast.Float f) (= f 1.0)) (_ false)))
      (export main)))
  (error CDZ0201))

(case
  "a tagged-template tag returning a finite Ast.Float expands normally (expander control)"
  (doc
    "The control for the expander × non-canonical guard: a tag returning a FINITE `(Ast.Float 2.5)`
           expands and folds — the guard only rejects a non-canonical payload, not a finite one, on the
           expander path just as on the direct ctor.")
  (input
    (do
      (def (ok chunks holes) (Ast.Float 2.5))
      (def
        (main)
        (match (tagged-template ok (chunks "x") (holes)) ((Ast.Float f) (= f 2.5)) (_ false)))
      (export main)))
  (output (: true Bool)))

(case
  "eval of an Ast.Float NaN node executes to the NaN value — it does NOT construct an escaping node"
  (doc
    "The eval-vs-construct asymmetry for a non-canonical float: CONSTRUCTING an escaping `(Ast.Float
           Float64.nan)` value declines (no canonical value form), but `(eval (Ast.Float Float64.nan))`
           RUNS to the NaN value. `eval_ast::desugar_eval` is a load-time structural rewrite that
           reconstructs the SOURCE float literal the node denotes and executes it — it never builds an
           escaping `Ast.Float` SumNew, so the construction guard correctly does not fire; eval yields a
           bare `Float64` NaN, which is a legal value (a bare NaN crosses fine, `= nan nan` is true by the
           canonical byte form). Pins that the guard is about the AST-node escape surface, not about NaN as
           a value — eval executes, it does not reify-and-escape. `(= (eval (Ast.Float Float64.nan))
           Float64.nan)` is true.")
  (input (do (def (main) (= (eval (Ast.Float Float64.nan)) Float64.nan)) (export main)))
  (output (: true Bool)))

(case
  "eval of a quoted float executes it to the float value"
  (doc
    "eval executes an AST value as code; a float form evaluates to itself, so `(eval (quote 1.5))`
           runs to `1.5` — the float companion of `(eval (quote true))`.")
  (input (do (def (main) (eval (quote 1.5))) (export main)))
  (output (: 1.5 Float64)))

(case
  "eval of a quoted RECURSIVE call folds through the compile-time evaluator"
  (doc
    "eval reconstructs the quoted form and evaluates it on the one-tier compile-time evaluator, which
           reduces RECURSION (the depth-guarded fold): `(eval (quote (sum-to 4)))` with `sum-to` a
           self-recursive `1..n` sum runs to 10. Distinct from the runtime recursive-AST-evaluator cases
           (which recurse over Ast VALUES at runtime) — this pins that eval's compile-time reduction folds a
           recursive USER function call to a constant. A terminating recursion folds; the evaluator's depth
           backstop stops a non-terminating one. Companion of the recursive-tag fold in 24-tagged-templates.")
  (input
    (do
      (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1)))))
      (def (main) (eval (quote (sum-to 4))))
      (export main)))
  (output (: 10 Int64)))

(case
  "encoding and decoding an Ast.Float round-trips to an equal value"
  (doc
    "`(Ast.Float 1.5)` encodes (the f64 bit pattern) then decodes to an equal AST (ast-encoding.md
           #The Encoding Is A Bijection), as the Int/Bool/Str/Name/List round-trips do. `Ast.decode` is
           total, so the round-trip matches the `Ok` arm.")
  (input
    (match (Ast.decode (Ast.encode (Ast.Float 1.5))) ((Ok a) (= a (Ast.Float 1.5))) ((Err _) false)))
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
(case
  "the Float codec round-trips a negative value"
  (doc
    "`Ast.Float -2.5` encodes+decodes to an equal AST — the sign bit of the f64 payload survives the
           byte round-trip. The negative companion of the `Ast.Float 1.5` round-trip; a codec that mis-read
           the sign or the exponent bits would corrupt it.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Float -2.5)))
      ((Ok a) (= a (Ast.Float -2.5)))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "negative zero encodes to bytes distinct from positive zero"
  (doc
    "`-0.0` and `0.0` are `==` as Float64 but have DISTINCT IEEE-754 bit patterns (only the sign bit
           differs), and the codec stores the raw bits — so `Ast.encode (Ast.Float -0.0)` ≠ `Ast.encode
           (Ast.Float 0.0)`. Pins the exact invariant the encoding comment calls out (\"-0.0 ≠ 0.0\"): a
           codec that canonicalized signed zero would collapse these to equal bytes and lose `-0.0`.")
  (input (= (Ast.encode (Ast.Float -0.0)) (Ast.encode (Ast.Float 0.0))))
  (output (: false Bool)))

(case
  "negative zero round-trips through the codec by byte identity"
  (doc
    "`Ast.Float -0.0` decodes to an AST that re-encodes to identical bytes — the sign bit of signed
           zero is preserved end-to-end (comparing by re-encoded bytes, since `-0.0 = 0.0` is true as a
           float value and would not distinguish them). Companion of the distinct-bytes case: pins that the
           round-trip, not just the initial encode, keeps signed zero.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Float -0.0)))
      ((Ok a) (= (Ast.encode a) (Ast.encode (Ast.Float -0.0))))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "print of an Ast.Float renders a re-readable decimal and read inverts it"
  (doc
    "`print` renders an `Ast.Float` as the shortest round-tripping decimal — always carrying a `.`
           (or `e`) so it re-reads as a float — and `read` parses it back: `read(Ast.print v) == v`.")
  (input (= (Ast.read (Ast.print (Ast.Float 1.5))) (Ast.Float 1.5)))
  (output (: true Bool)))

(case
  "print of an integer-valued Ast.Float keeps its float form through read"
  (doc
    "🔑 The int-vs-float rendering pin: an integer-VALUED float `3.0` prints with an explicit `.0`
           (not the bare `3` an integer prints), so `read` parses it back as an `Ast.Float`, NOT an
           `Ast.Int`. `read(Ast.print (Ast.Float 3.0)) == (Ast.Float 3.0)`.")
  (input (= (Ast.read (Ast.print (Ast.Float 3.0))) (Ast.Float 3.0)))
  (output (: true Bool)))

(case
  "print of a NEGATIVE Ast.Float round-trips through read (sign survives the text path)"
  (doc
    "The existing print/read cases use only positive `1.5`/`3.0`, so they never exercise the SIGN
           through the TEXT path (distinct from the encode/decode BYTE path pinned above). A printer that
           dropped or mis-rendered the sign would still pass those but lose a negative float here.
           `read(Ast.print (Ast.Float -1.5)) == (Ast.Float -1.5)` pins that the minus survives print → read.")
  (input (= (Ast.read (Ast.print (Ast.Float -1.5))) (Ast.Float -1.5)))
  (output (: true Bool)))

; The Ast.Int TEXT-path companions of the negative-Float case above: the print/read cases elsewhere use
; only small positive integers, so they never exercise a NEGATIVE integer's sign or the i64::MIN
; two's-complement boundary through the TEXT path (distinct from the encode/decode BYTE path, where
; i64::MIN is pinned below at "the Int codec round-trips i64::MIN"). A printer that dropped the minus, or
; one that rendered i64::MIN by negating its magnitude (which overflows i64 — |i64::MIN| is not
; representable), would pass the positive cases yet break here. `read(Ast.print (Ast.Int n)) == (Ast.Int n)`.
(case
  "print then read of a NEGATIVE Ast.Int round-trips (sign survives the text path)"
  (doc
    "The Int companion of the negative-Float text-path case: `print (Ast.Int -42)` renders `-42`
           and `read` parses it back to the same `Ast.Int`, so the minus survives print → read. Pins the
           sign through the TEXT path (distinct from the byte codec) — a printer that dropped the sign
           would still pass the positive `Ast.Int` cases but lose a negative one here.")
  (input (match (Ast.read (Ast.print (Ast.Int -42))) ((Ast.Int n) n) (_ 0N)))
  (output (: -42 BigInt)))

(case
  "print then read an Ast.Int at i64::MIN round-trips (text-path two's-complement boundary)"
  (doc
    "🔑 The TEXT-path companion of the byte-codec i64::MIN pin below: `Ast.Int -9223372036854775808`
           (i64::MIN) prints and re-reads to the same value. i64::MIN is the two's-complement extreme whose
           positive twin is NOT representable, so a printer that rendered a negative integer by negating its
           magnitude would overflow HERE (never on a small negative). Pins that the text path is exact at
           the boundary, not just the byte path.")
  (input (match (Ast.read (Ast.print (Ast.Int -9223372036854775808))) ((Ast.Int n) n) (_ 0N)))
  (output (: -9223372036854775808 BigInt)))

(case
  "print then read of a BEYOND-i64 Ast.Int round-trips (text path is arbitrary-precision)"
  (doc
    "🔑 The TEXT-path companion of the beyond-64-bit quote pins above: an `Ast.Int` whose payload is a
           26-digit BigInt (no i64 could carry it) `print`s to its full decimal and `read` parses it back
           to the exact same value. Pins that the printer renders the WHOLE magnitude (a `to_i64`-based
           renderer would decline/truncate the print) and that the reader classifies an all-digits token
           past the i64 boundary as an `Ast.Int`, not misread it as an `Ast.Name` — so `read(Ast.print v) == v`
           holds at arbitrary precision, not only up to i64::MIN.")
  (input
    (match
      (Ast.read (Ast.print (Ast.Int (: 99999999999999999999999999 BigInt))))
      ((Ast.Int n) n)
      (_ 0N)))
  (output (: 99999999999999999999999999 BigInt)))

(case
  "print then read of a BEYOND-i64 NEGATIVE Ast.Int round-trips (sign survives at arbitrary precision)"
  (doc
    "The negative twin of the beyond-i64 text-path case: a 26-digit NEGATIVE BigInt `Ast.Int` prints
           with its leading `-` and re-reads to the exact same value. Pins that the reader's sign handling
           and the printer's full-magnitude rendering compose past the i64 boundary — a printer that dropped
           the sign, or a reader that stripped `-` only for in-i64 tokens, would pass the positive beyond-i64
           case but lose this one.")
  (input
    (match
      (Ast.read (Ast.print (Ast.Int (: -99999999999999999999999999 BigInt))))
      ((Ast.Int n) n)
      (_ 0N)))
  (output (: -99999999999999999999999999 BigInt)))

(case
  "print then read of an Ast.Int at i64::MAX+1 round-trips (the exact reader handoff seam)"
  (doc
    "🔑 The FIRST magnitude past the i64 fast path: `9223372036854775808` is `i64::MAX + 1`, exactly
           where `read`'s `str::parse::<i64>` overflows and hands the token to the arbitrary-precision
           decimal path. An off-by-one at the handoff (accepting one-too-few digits, or reading this as a
           misclassified `Ast.Name`) would surface HERE but pass both the in-i64 `i64::MIN` case above and
           the comfortably-large 26-digit case. Pins the seam value itself, not just a value far past it.")
  (input
    (match (Ast.read (Ast.print (Ast.Int (: 9223372036854775808 BigInt)))) ((Ast.Int n) n) (_ 0N)))
  (output (: 9223372036854775808 BigInt)))

(case
  "print then read of an Ast.Int at i64::MIN-1 round-trips (the negative reader handoff seam)"
  (doc
    "The negative twin of the handoff-seam case: `-9223372036854775809` is `i64::MIN - 1`, the first
           NEGATIVE magnitude where `str::parse::<i64>` overflows (NegOverflow). i64::MIN's own magnitude is
           not positively i64-representable, so a reader that recovered a negative by parsing the magnitude
           as i64 then negating would already fail one step earlier — this pins that the sign + full
           magnitude compose correctly right at the negative handoff boundary.")
  (input
    (match (Ast.read (Ast.print (Ast.Int (: -9223372036854775809 BigInt)))) ((Ast.Int n) n) (_ 0N)))
  (output (: -9223372036854775809 BigInt)))

(case
  "print then read of a LIST carrying a beyond-i64 Ast.Int round-trips (bignum token bounded by parens)"
  (doc
    "The COMPOSITIONAL companion of the bare beyond-i64 scalar cases: a `(f <26-digit>)` list prints to
           `(f 99999999999999999999999999)` and `read` recovers the exact `Ast.Int` FROM INSIDE the list.
           This exercises a seam the bare-scalar cases don't — the reader tokenizes the bignum digit-run
           bounded by a closing `)` (not whitespace/EOF) during RECURSIVE list parsing, and the printer
           renders the full magnitude with no surrounding delimiter to absorb a truncation. A reader that
           only applied the arbitrary-precision path to a top-level token, or a token scan that mis-bounded
           the digit-run at `)`, would pass every bare-scalar bignum case yet drop the payload here. The
           head `f` re-reads as a name and the payload as an `Ast.Int` — so the extracted second element's
           value pins the full round-trip.")
  (input
    (match
      (Ast.read
        (Ast.print (Ast.List #list((Ast.Name "f") (Ast.Int (: 99999999999999999999999999 BigInt))))))
      ((Ast.List xs) (match xs (#list(_ (Ast.Int n)) n) (_ 0N)))
      (_ 0N)))
  (output (: 99999999999999999999999999 BigInt)))

(case
  "print of an exponent-scale Ast.Float round-trips through read"
  (doc
    "A large-magnitude float `1e10` is rendered by `print` (shortest round-tripping form, which may
           use `e` notation) and `read` parses it back bit-exactly. Pins the exponent/large-magnitude
           rendering path of the printer, which the small `1.5`/`3.0` cases don't reach.")
  (input (= (Ast.read (Ast.print (Ast.Float 10000000000.0))) (Ast.Float 10000000000.0)))
  (output (: true Bool)))

(case
  "print of a negative-zero Ast.Float preserves the sign through read (text-path signed-zero)"
  (doc
    "🔑 The TEXT-path companion of the byte-path signed-zero pin (`negative zero encodes to bytes
           distinct from positive zero`): `-0.0` and `0.0` are `==` as floats but bit-distinct, and the
           canonical value form keeps them apart. A printer that rendered `-0.0` as bare `0.0` would pass
           the byte-codec pin (which never prints) yet silently drop the sign here. `read(Ast.print (Ast.Float
           -0.0)) == (Ast.Float -0.0)` pins that print → read preserves negative zero's identity too.")
  (input (= (Ast.read (Ast.print (Ast.Float -0.0))) (Ast.Float -0.0)))
  (output (: true Bool)))

; The sign/exponent/-0.0 cases above pin the FORM of the text path; these pin its PRECISION. `print` renders
; the SHORTEST round-tripping decimal — the contract is `read(Ast.print v) == v` for EVERY Float64, so a printer
; that emitted too few significant digits would pass the small-integer/sign cases yet silently corrupt a value
; needing full precision. These pin the round-trip at a 17-significant-digit value (the `0.1 + 0.2` result,
; the classic precision stress), the maximum finite magnitude, the smallest subnormal, and the INJECTIVITY
; the shortest-form contract requires: two DISTINCT doubles must print to DISTINCT text (`0.3` and the
; `0.1+0.2` result are `==`-close but different doubles, so they must NOT round-trip-collide — a printer
; truncating both to `0.3` would collapse them).
(case
  "print then read an Ast.Float at full 17-significant-digit precision round-trips"
  (doc
    "The precision companion of the form cases: `0.30000000000000004` (the exact `0.1 + 0.2` binary64
           result) needs all 17 significant digits to round-trip. `read(Ast.print (Ast.Float 0.30000000000000004))
           == it` pins that `print` emits enough digits — a printer rendering the shorter `0.3` would re-read
           as a DIFFERENT double and fail this, though it passes the `1.5`/`3.0`/sign cases.")
  (input (= (Ast.read (Ast.print (Ast.Float 0.30000000000000004))) (Ast.Float 0.30000000000000004)))
  (output (: true Bool)))

(case
  "print then read an Ast.Float at the maximum finite magnitude round-trips"
  (doc
    "The large-magnitude extreme: `1.7976931348623157e308` (≈ Float64.max) round-trips through the text
           path — `read(Ast.print v) == v`. Pins the shortest-form renderer handles the top of the exponent range
           bit-exactly, not only the `1e10` mid-range exponent case.")
  (input
    (=
      (Ast.read
        (Ast.print
          (Ast.Float
            179769313486231570000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000.0)))
      (Ast.Float
        179769313486231570000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000.0)))
  (output (: true Bool)))

(case
  "print then read an Ast.Float at the smallest subnormal round-trips"
  (doc
    "The tiny-magnitude extreme: `5e-324` (the smallest positive subnormal Float64) round-trips —
           `read(Ast.print v) == v`. Pins the text path preserves a subnormal (denormalized) value, the bottom of
           the magnitude range, which a renderer flushing subnormals to zero or mis-scaling the exponent
           would lose.")
  (input
    (=
      (Ast.read
        (Ast.print
          (Ast.Float
            0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005)))
      (Ast.Float
        0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000005)))
  (output (: true Bool)))

(case
  "two distinct doubles print to distinct text — 0.3 and 0.1+0.2 do not round-trip-collide"
  (doc
    "The INJECTIVITY the shortest-round-tripping contract requires: `0.3` and the `0.1 + 0.2` result
           (`0.30000000000000004`) are distinct Float64 values (differing in the last ULP). `print` must
           render them to DISTINCT text so `read` recovers each exactly — so `(= (Ast.read (Ast.print (Ast.Float 0.3)))
           (Ast.read (Ast.print (Ast.Float (+ 0.1 0.2)))))` is FALSE (0). A printer truncating both to `0.3` would
           collapse them to equal text and wrongly answer true — the precision-loss failure this rules out.")
  (input
    (do
      (def
        (main)
        (if
          (= (Ast.read (Ast.print (Ast.Float 0.3))) (Ast.read (Ast.print (Ast.Float (+ 0.1 0.2)))))
          1
          0))
      (export main)))
  (output (: 0 Int64)))

; --- `read` is TOTAL over its input: malformed text DECLINES, never traps or panics ------------------
; `read : String → Ast` parses the s-expression subset `print` emits (`lower_read`/`SexprReader`). The
; round-trip cases above only feed it WELL-FORMED text (`read(Ast.print v)`); none exercises the failure
; paths. `read` must be total the way the reader/lexer are "never panic" (syntax-vertical invariant) and
; the way `Ast.decode` is total over adversarial bytes — but `read` fails at COMPILE time (a constant-only
; fold), so a malformed input is a clean DECLINE (`Reject::decline`), not a runtime `Err` and never a
; trap/panic. These pin the three distinct decline arms in `lower_read`: text that is not a well-formed
; s-expression (the parser returns nothing), text with TRAILING content after the first node (the
; `at_end` check — a valid prefix must NOT be silently accepted), and an empty string. A reader change
; that panicked on unbalanced input, or that silently took the first node and dropped a trailing token,
; would break these. All → `(declines)`.
(case
  "read of text that is not a well-formed s-expression rejects with CDZ0201"
  (doc
    "`(Ast.read \"(((\")` — unbalanced open parens are not a well-formed s-expression over the Ast
           subset, so `read` REJECTS with CDZ0201 (`lower_read`'s coded parse-failure arm) rather than
           trapping or fabricating a partial AST. Malformedness is a permanent fact, so it is a coded
           rejection of ill-formed input, NOT a codeless decline. Pins that the reader is total on
           malformed input — the `read` companion of the adversarial-bytes `Ast.decode` totality cases,
           and of the parser/lexer never-panic invariant.")
  (input (Ast.read "((("))
  (error CDZ0201))

(case
  "read of text with trailing content after the first s-expression rejects with CDZ0201"
  (doc
    "`(Ast.read \"1 2\")` — a valid first node (`1`) FOLLOWED by more input (`2`). `read` must consume the
           WHOLE string (the `r.at_end()` check in `lower_read`), so trailing content is rejected with
           CDZ0201 rather than silently reading `1` and dropping `2`. The `read` parallel of the decode case
           where canonical bytes plus a trailing byte yield `Err`: a valid prefix is not a valid whole.")
  (input (Ast.read "1 2"))
  (error CDZ0201))

(case
  "read of the empty string rejects with CDZ0201 (no s-expression, coded like its malformed siblings)"
  (doc
    "`(Ast.read \"\")` — no s-expression at all. The empty string parses to no node, so `read` REJECTS with
           CDZ0201 (`lower_read`'s coded parse-failure arm) — never a trap or an empty/garbage AST. The
           zero-input edge of the reader's totality: like the unbalanced `\"(((\"` and trailing `\"1 2\"`
           siblings above, ill-formed/absent input is a coded rejection, not a codeless decline. (Corpus is
           the impl-independent spec: a user-facing reject asserts its code — v-corpus-harness spot-confirmed
           the same CDZ0201 + message as the sibling.)")
  (input (Ast.read ""))
  (error CDZ0201))

(case
  "read classifies a lone punctuation token as an Ast.Name and it round-trips"
  (doc
    "A bare non-numeric, non-keyword atom — even a punctuation/operator symbol like `.` — is read as an
           `Ast.Name` (the `SexprReader`'s atom fallthrough: not a number, not `true`/`false`, so a name). It
           is a WELL-FORMED single node (does NOT decline), and prints back to `\".\"` so `read(Ast.print v) == v`.
           Pins the reader's atom-vs-structure boundary at an operator-symbol token — the sound companion of
           the alphanumeric-name / keyword-collision cases, and of the decline cases above (a lone `.` is a
           valid name atom, not malformed input).")
  (input (= (Ast.print (Ast.read ".")) "."))
  (output (: true Bool)))

; The `#`-prefixed-atom companion of the operator-symbol-token case above (v-syntax authoritative, their
; lane): the reader treats `#` as a Sym/char SIGIL only when the next byte is `"` (`#"foo"` → a Symbol
; VALUE leaf) or `\` (a char); for any OTHER following byte — including an identifier char — a bare `#`
; reads as an ORDINARY atom, so `#foo` lexes as a single `Ast.Name` whose text INCLUDES the `#` (byte-len
; 4). So `(quote #foo)` = `(Ast.Name "#foo")`, a `#`-prefixed NAME — DISTINCT from `(quote foo)` (the bare
; name) and from `#"foo"` (a symbol value, which has no `Ast` variant and declines). Pins that the `#`
; stays part of the name text through quote, not stripped or treated as a separate token.
(case
  "a hash-prefixed atom quotes to an Ast.Name carrying the hash in its text"
  (doc
    "`(quote #foo)` is `(Ast.Name \"#foo\")` — the reader's `#` sigil is active only before `\"`/`\\`
           (symbol/char), so a `#` before an identifier char is part of an ordinary name atom; the `#` is
           carried in the `Ast.Name` text. Companion of the lone-punctuation-token case; the NAME half of
           the `#foo` (name) vs `#\"foo\"` (symbol value) distinction.")
  (input (= (quote #foo) (Ast.Name "#foo")))
  (output (: true Bool)))

(case
  "a hash-prefixed name is distinct from the bare name without the hash"
  (doc
    "The discriminating companion: `(quote #foo)` and `(quote foo)` quote to DIFFERENT `Ast.Name`
           values (`\"#foo\"` vs `\"foo\"`), so `=` is FALSE — the `#` is a significant part of the name
           text, not stripped. Guards against a reader that dropped a leading `#` and collapsed the two.")
  (input (= (quote #foo) (quote foo)))
  (output (: false Bool)))

(case
  "an active unquote of a float literal lifts to an Ast.Float node"
  (doc
    "`` `(f ,2.5) `` embeds the float literal `2.5` as the `Ast.Float` leaf its value denotes — the
           same node `(quote (f 2.5))` builds. The float companion of the literal Int/Bool/Str cases.")
  (input (= (quasiquote (f (unquote 2.5))) (Ast.List #list((Ast.Name "f") (Ast.Float 2.5)))))
  (output (: true Bool)))

(case
  "an active unquote of a let-bound float lifts to Ast.Float by inferred type"
  (doc
    "A RUNTIME float operand lifts by its inferred type: `x : Float64` → `Ast.Float`. `(let ((x 4.5))
           `(f ,x))` builds `(Ast.List (list (Ast.Name \"f\") (Ast.Float 4.5)))` — the `ast-lift` path.")
  (input
    (let
      ((x 4.5))
      (= (quasiquote (f (unquote x))) (Ast.List #list((Ast.Name "f") (Ast.Float 4.5))))))
  (output (: true Bool)))

(case
  "a quoted compound form equals the same AST built by the Ast.List constructor"
  (doc
    "The list companion, and the sharpest case: `(quote (+ 1 2))` is
           `(Ast.List (list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2)))` — the very value form the FIRST
           case in this file records as `(quote (+ 1 2))`'s output. So comparing the quote against that
           hand-built Ast.List MUST be true (core-semantics.md #Equality Is Structural), because they
           are the same sum value. This equality is the compiler's own idiom: it builds an instruction
           AST by quasiquote and compares it against an expected AST built by constructors.")
  (input (= (quote (+ 1 2)) (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))))
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
(case
  "a built-in Ast constructor applied to a wrong-type payload is a type error"
  (doc
    "`Ast.Int`'s payload type is Int64 (the built-in `Ast` is an ordinary sum type, a variant per
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
  (input (Ast.Int "x"))
  (error CDZ0201))

(case
  "quote-vs-constructor AST equality holds regardless of operand order"
  (doc
    "The order-flipped companion: `(= (Ast.Int 42) (quote 42))` is the same comparison of one
           sum value against itself and MUST be true. Pins that neither operand order (constructor-built
           vs quote-built) is treated as a distinct type — structural equality is symmetric over the
           one AST value form.")
  (input (= (Ast.Int 42) (quote 42)))
  (output (: true Bool)))

; --- Structural equality DISCRIMINATES by list order/length and by variant tag ---------------------
; The equality cases above pin that quote-built and constructor-built forms of the SAME tree are EQUAL.
; The dual guard is that `=` DISTINGUISHES trees that differ — otherwise a `=` that ignored element order,
; ignored list length, or collapsed a leaf by its numeric value would wrongly answer `true`. Three cases
; pin the discrimination: `Ast.List` equality is element-ORDER sensitive and element-COUNT sensitive (the
; list walk compares positionally and length-first, per the encoding's canonical byte form — two trees
; equal only if identical), and an `Ast.Bool` is distinct from an `Ast.Int` by VARIANT TAG, never by the
; boolean's numeric value (a `=` that compared payloads across variants would equate `true` with `1`).
; Companions of the cross-variant Str-vs-Name (above) and Float-vs-Int distinctness cases.
(case
  "structural equality on an Ast.List is element-ORDER sensitive"
  (doc
    "Two `Ast.List` values with the SAME elements in a DIFFERENT order are NOT equal: `(Ast.List
           (list (Ast.Int 1) (Ast.Int 2)))` vs `(Ast.List (list (Ast.Int 2) (Ast.Int 1)))` → `=` is
           false. Pins that the list-equality walk compares elements POSITIONALLY (per the encoding's
           canonical byte form), so a `=` that compared list contents order-insensitively is caught.")
  (input (= (Ast.List #list((Ast.Int 1) (Ast.Int 2))) (Ast.List #list((Ast.Int 2) (Ast.Int 1)))))
  (output (: false Bool)))

(case
  "structural equality on an Ast.List is element-COUNT sensitive"
  (doc
    "A one-element `Ast.List` is NOT equal to a two-element one even when the shorter is a PREFIX of
           the longer: `(Ast.List (list (Ast.Int 1)))` vs `(Ast.List (list (Ast.Int 1) (Ast.Int 2)))` →
           `=` is false. Pins that list equality is length-sensitive (a prefix is not a match), so a `=`
           that ignored the trailing elements is caught.")
  (input (= (Ast.List #list((Ast.Int 1))) (Ast.List #list((Ast.Int 1) (Ast.Int 2)))))
  (output (: false Bool)))

(case
  "an Ast.Bool and an Ast.Int are distinct by variant tag not by numeric value"
  (doc
    "`(Ast.Bool true)` and `(Ast.Int 1)` are DIFFERENT `Ast` sum variants, so `=` is false — the
           compare discriminates by VARIANT TAG first, never collapsing a boolean to its numeric value
           (`true`→1). The cross-variant-distinctness companion of Str-vs-Name and Float-vs-Int: a `=`
           that compared payloads across variants would wrongly equate `Ast.Bool true` with `Ast.Int 1`.")
  (input (= (Ast.Bool true) (Ast.Int 1)))
  (output (: false Bool)))

(case
  "a runtime-constructed Ast equals its quoted twin by structural content"
  (doc
    "The runtime face of construction-path irrelevance: `(Ast.List (list (Ast.Name \"+\") (Ast.Int
           n) (Ast.Int 2)))` — the payload a boundary PARAMETER, so the tree is assembled at run time —
           compared against the quote-built `(quote (+ 5 2))`. Equal exactly when n=5 (1), different leaf
           at n=6 (0). One compiled compare walks both spines; the const equality cases above could in
           principle fold, so this pins the structural walk as residual code over a runtime-built Ast.")
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (=
            (Ast.List #list((Ast.Name "+") (Ast.Int (BigInt.of n)) (Ast.Int (BigInt.of 2))))
            (quote (+ 5 2)))
          1
          0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "quoted Asts as SET elements dedup by structural content"
  (doc
    "Asts as CHAMP elements: `{(quote (+ 1 2)), (quote (+ 1 2)), (quote (* 1 2))}` — the two
           identical quotes collapse to one slot (the champ hash/compare walks the Ast spine by content)
           and the `*`-headed tree stays distinct → len 2. The collection face of Ast equality: a
           memoizing pass keyed by expression shape rests on exactly this. (wasm computes; rust declines
           the Ast-typed champ element — the same class as the Symbol-key decline, per-target baselined.)")
  (input
    (do
      (def (main (: n Int64)) (Set.len #set((quote (+ 1 2)) (quote (+ 1 2)) (quote (* 1 2)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "an Ast as a MAP key looks up by structural content"
  (doc
    "The map twin: insert 42 under the key `(quote (+ 1 2))`, look up with a SEPARATELY-quoted
           equal tree — the hash and compare must both walk structure, so the lookup hits (42). The
           rewrite-cache idiom (memoize by sub-tree). (wasm computes; rust declines — same per-target
           class as the set-element case.)")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m (Map.insert Map.empty (quote (+ 1 2)) 42)))
          (match (Map.lookup m (quote (+ 1 2))) ((Some v) v) ((None u) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

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
(case
  "encoding and decoding a constructor-built AST round-trips to an equal value"
  (doc
    "`(Ast.Int 7)` is an AST value (the same form `(quote 7)` produces); encoding then decoding it
           MUST yield an equal AST (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t).
           The quote-built round-trip is witnessed earlier in this file; a constructor-built AST is the
           same value form, so it round-trips identically — the encoder bridges an applied `Ast.*`
           constructor to the AST value it denotes. `Ast.decode : Bytes → Result<Ast, _>` is total
           (value-interchange.md — decode of possibly-external bytes yields the error case, never traps),
           so the round-trip matches the `Ok` arm and equates its payload.")
  (input (match (Ast.decode (Ast.encode (Ast.Int 7))) ((Ok a) (= a (Ast.Int 7))) ((Err _) false)))
  (output (: true Bool)))

; --- The Int payload is a NON-LOSSY sign + magnitude: negatives and the range boundary round-trip ---
; ast-encoding.md: an `Ast.Int n` encodes as tag 0x00 + 1 sign byte + a 4-byte LE magnitude length + the
; big-endian minimal magnitude (`encode_ast_value`/`decode_ast_value` in lower.rs) — a NON-LOSSY form
; (operator directive: parametric integers must never truncate), so the magnitude is arbitrary-precision,
; not a fixed 8-byte i64. The round-trip case above uses only the small positive `7`, which never
; exercises the sign byte or a multi-byte magnitude — so a decoder that dropped the sign, or mis-read the
; length, would pass `7` yet corrupt a negative or large value. These pin the SIGNED boundary: `i64::MIN`
; (-9223372036854775808 — its magnitude is not representable as a positive i64, catching a
; negate-during-decode bug), and that `-1` and `1` encode to DISTINCT bytes (they differ only in the sign
; byte — a decoder that dropped it collapses them). A negative nested in a compound pins the same through
; the recursive encoder. Promoted from passing probes (breaker rule: pin the invariant so a future codec
; change can't quietly flip it).
(case
  "the Int codec round-trips i64::MIN — the two's-complement boundary"
  (doc
    "`Ast.Int -9223372036854775808` (i64::MIN) encodes+decodes to an equal AST. Its magnitude has no
           positive i64 representation, so a decoder that negated during decode, or dropped the sign byte,
           corrupts it. The negative companion of the `Ast.Int 7` round-trip: pins the sign+magnitude
           contract at a hard value. `Ast.decode` is total, so the round-trip matches the `Ok` arm.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Int -9223372036854775808)))
      ((Ok a) (= a (Ast.Int -9223372036854775808)))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "a negative and its positive twin encode to distinct bytes"
  (doc
    "`Ast.encode (Ast.Int -1)` ≠ `Ast.encode (Ast.Int 1)`: the sign is carried in the two's-
           complement byte form (all-ones vs a single low bit), so a codec that dropped or ignored the
           sign would collapse them to equal bytes. Pins that the encoding distinguishes sign — the
           byte-level companion of the i64::MIN round-trip.")
  (input (= (Ast.encode (Ast.Int -1)) (Ast.encode (Ast.Int 1))))
  (output (: false Bool)))

(case
  "a negative integer nested in a compound round-trips by byte identity"
  (doc
    "`(quote (f -42 \"s\"))` — a negative Int leaf beside a string, inside a list — decodes to an
           AST that re-encodes to identical bytes (the bijection's byte-identity face). Pins that the
           RECURSIVE encoder threads the signed payload through a compound, not only a bare leaf.")
  (input
    (match
      (Ast.decode (Ast.encode (quote (f -42 "s"))))
      ((Ok a) (= (Ast.encode a) (Ast.encode (quote (f -42 "s")))))
      ((Err _) false)))
  (output (: true Bool)))

; The canonical cdzast CONTAINER HEADER on the encode side — the wire-format contract (operator ruling
; OPTION A, 2026-08-15: no bespoke formats; `Ast.encode`/`Ast.decode` use the SINGLE canonical codec, the
; same form the kernel `decode_shell_pipeline`/`codec::decode` read). The encoded bytes OPEN with the
; container version header `cdzast\x00\x01` for EVERY variant; the variant is discriminated DEEP in the
; arena (its leaf kind tag), NOT at byte 0 — so the old per-variant-tag-at-byte-0 property no longer exists.
(case
  "Ast.encode emits the canonical cdzast container header (byte 0 is 'c')"
  (doc
    "`Ast.encode` serializes through the single canonical cdzast codec (`crate::codec`), so its bytes
           OPEN with the container version header `cdzast\\x00\\x01` — byte 0 is `c` (0x63 = 99) for EVERY
           variant (the variant lives deep in the arena's leaf kind tag, not at byte 0; this REPLACES the
           removed per-variant tag-at-byte-0 property the bespoke format had). Pins that the guest emits
           exactly the canonical header the kernel parses. Int shown; every variant opens identically.")
  (input (match (Bytes.at (Ast.encode (Ast.Int 5)) 0) ((Option.Some b) (Int64.of b)) (_ -1)))
  (output (: 99 Int64)))

(case
  "encoding and decoding a constructor-built compound AST round-trips"
  (doc
    "The compound companion: a hand-built `(Ast.List (list (Ast.Name \"g\") (Ast.Int 5)))` MUST
           encode and decode back to an equal AST, exactly as a quote-built list does. Pins that the
           bijection round-trip reaches a constructor-built compound AST, not only a leaf node. `Ast.decode`
           is total (`Bytes → Result<Ast, _>`), so the round-trip matches the `Ok` arm.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.List #list((Ast.Name "g") (Ast.Int 5)))))
      ((Ok a) (= a (Ast.List #list((Ast.Name "g") (Ast.Int 5)))))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "Ast.decode decodes bytes that arrive as a function argument"
  (doc
    "`Ast.decode : Bytes → Result<Ast, _>` MUST decode bytes whatever their PROVENANCE — a literal,
           or (here) bytes passed in as a function ARGUMENT. The round-trip cases above decode a literal in
           tail position; this decodes `b`, a parameter of `dec`, which is how a program that reads its
           input decodes it (a compiler receives its program bytes as an argument, not a literal). The
           result is the same total `Result<Ast, _>`, so `(dec (Ast.encode (Ast.Int 42)))` matches the `Ok`
           arm and yields 42. A generation that realizes `Ast.decode` only over a compile-time-constant
           argument (folding it away) declines the runtime-argument form (\"unsupported dotted-application\")
           — but decode is an ordinary total operation on a runtime `Bytes` value, so it MUST run here.")
  (input
    (do
      (def (main) (dec (Ast.encode (Ast.Int 42))))
      (def
        (dec b)
        (match (Ast.decode b) ((Ok a) (match a ((Ast.Int n) n) (other -1N))) ((Err _) -2N)))
      (export main)))
  (output (: 42 BigInt)))

(case
  "a quote-built and constructor-built AST of the same tree encode to identical bytes"
  (doc
    "ast-encoding.md #The Encoding Is A Bijection With One Canonical Byte Form: \"Two abstract
           syntax trees that are equal MUST have identical binary encodings.\" `(quote 42)` and
           `(Ast.Int 42)` are the same AST (the equality cases above), so their encodings MUST be
           byte-identical. This is the encode-path witness of the one-canonical-byte-form requirement:
           the encoder must produce the same bytes for the one AST value however it was constructed. The
           seed declines the constructor-built operand, so it cannot yet witness the agreement.")
  (input (= (Ast.encode (quote 42)) (Ast.encode (Ast.Int 42))))
  (output (: true Bool)))

(case
  "a quote-built and constructor-built FLOAT AST encode to identical bytes"
  (doc
    "The float companion of the byte-identity case: `(quote 1.5)` and `(Ast.Float 1.5)` are the same
           AST value, so their encodings MUST be byte-identical (ast-encoding.md #The Encoding Is A
           Bijection With One Canonical Byte Form). Pins that the `Ast.Float` leaf's canonical bytes (the
           f64 bit pattern) are the same however the value is constructed.")
  (input (= (Ast.encode (quote 1.5)) (Ast.encode (Ast.Float 1.5))))
  (output (: true Bool)))

(case
  "unquote-splicing splices list elements into parent"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,@<list-expr> evaluates <list-expr> to a list and splices its elements into the parent,
           not nested. `(+ ,@args) with args=(list 1 2 3) produces AST for (+ 1 2 3), not (+ (1 2 3)).")
  (input (let ((args #list(1 2 3))) (quasiquote (+ (unquote-splicing args)))))
  (output (: (Ast.List #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))) Ast)))

(case
  "splice flattens where unquote nests"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , nests the value; ,@ splices it. `(f ,x) embeds x as one element; `(f ,@x) with
           x=(list 1 2) splices to produce (f 1 2).")
  (input (let ((x #list(1 2))) (= (quasiquote (f (unquote-splicing x))) (quasiquote (f 1 2)))))
  (output (: true Bool)))

; --- Splice-lift is type-directed across the scalar leaves (not Int-only) -------------------
; `,@<list>` lifts each element of a compile-time-constant list into the `Ast` leaf its value kind
; denotes — Int64→`Ast.Int`, Float64→`Ast.Float`, Bool→`Ast.Bool`, String→`Ast.Str`. Earlier the
; splice fold wrapped every element in `Ast.Int` unconditionally, so a non-Int constant list declined
; ("needs a compile-time-constant Int64 list"). These pin that the lift now dispatches by element kind
; (the constant companion of the active-unquote `ast-lift`, which agrees on the same leaf set), so a
; Float/Bool/String list splices to the correctly-tagged nodes. A non-scalar element (a nested list)
; still declines rather than mis-lifting — reject-don't-miscompile.
(case
  "unquote-splicing lifts a float list to Ast.Float leaves"
  (doc
    "The float companion of the Int splice: `,@` of a constant `(List Float64)` lifts each
           element to an `Ast.Float` node (not `Ast.Int`), so `(f ,@xs)` with xs=(list 1.5 2.5)
           reifies to `(Ast.List (Ast.Name f) (Ast.Float 1.5) (Ast.Float 2.5))`. Pins the type-directed
           splice-lift over Float64 — this declined before the lift dispatched by element kind.")
  (input (let ((xs #list(1.5 2.5))) (quasiquote (f (unquote-splicing xs)))))
  (output (: (Ast.List #list((Ast.Name "f") (Ast.Float 1.5) (Ast.Float 2.5))) Ast)))

(case
  "unquote-splicing lifts a boolean list to Ast.Bool leaves"
  (doc
    "The boolean companion: `,@` of a constant `(List Bool)` lifts each element to an `Ast.Bool`
           node, so `(f ,@xs)` with xs=(list true false) reifies to `(Ast.List (Ast.Name f)
           (Ast.Bool true) (Ast.Bool false))`. Pins the Bool arm of the type-directed splice-lift.")
  (input (let ((xs #list(true false))) (quasiquote (f (unquote-splicing xs)))))
  (output (: (Ast.List #list((Ast.Name "f") (Ast.Bool true) (Ast.Bool false))) Ast)))

(case
  "unquote-splicing lifts a string list to Ast.Str leaves"
  (doc
    "The string companion: `,@` of a constant `(List String)` lifts each element to an `Ast.Str`
           node (a string LITERAL leaf, distinct from a Name), so `(f ,@xs)` with xs=(list \"a\" \"bb\")
           reifies to `(Ast.List (Ast.Name f) (Ast.Str \"a\") (Ast.Str \"bb\"))`. Pins the Str arm.")
  (input (let ((xs #list("a" "bb"))) (quasiquote (f (unquote-splicing xs)))))
  (output (: (Ast.List #list((Ast.Name "f") (Ast.Str "a") (Ast.Str "bb"))) Ast)))

(case
  "unquote-splicing a list of Ast values splices by identity"
  (doc
    "An element already of type `Ast` needs no wrapping — it splices AS-IS (identity), the same
           identity the active-unquote `ast-lift` gives an already-`Ast` operand. Splicing a list of
           PRE-BUILT AST fragments `(list (Ast.Int 7) (Ast.Int 8))` into `(f ,@xs)` reifies to
           `(Ast.List (Ast.Name f) (Ast.Int 7) (Ast.Int 8))` — the fragments appear unchanged, not
           re-wrapped in another leaf. Pins the `(List Ast)` identity arm of the splice-lift.")
  (input (let ((xs #list((Ast.Int 7) (Ast.Int 8)))) (quasiquote (f (unquote-splicing xs)))))
  (output (: (Ast.List #list((Ast.Name "f") (Ast.Int 7) (Ast.Int 8))) Ast)))

(case
  "the element list of an Ast.List escapes as a (List Ast) value and renders its structure"
  (doc
    "Reaching into a compound and handing back its element LIST yields a `(List Ast)` value that
           ESCAPES the boundary in its canonical rendered form — not a length count, not a bool. Matching
           `(quote (+ 1 2))` on `((Ast.List elems) elems)` returns `elems`, a `(List Ast)` whose elements
           are the operator name and both argument nodes: `(list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int
           2))`. Pins the ESCAPING/render boundary form for a `(List Ast)` collection value (distinct from
           the `Ast.List` sum-value cases above, which escape a single `Ast`): the element list itself
           crosses the boundary and reads back structurally. This is the value-face the surface examples
           rely on — showing the real child nodes rather than `(List.len elems)` = 3. The `(_ (list))`
           wildcard makes the match exhaustive over the `Ast` sum (a non-`List` head returns the empty
           `(List Ast)`); a match returning an `Ast`/`(List Ast)` value MUST cover Int/Float/Bool/Str/Name
           or it is CDZ0210 non-exhaustive.")
  (input (match (quote (+ 1 2)) ((Ast.List elems) elems) (_ #list())))
  (output (: #list((Ast.Name "+") (Ast.Int 1) (Ast.Int 2)) (List Ast))))

(case
  "unquote-splicing a list with a RUNTIME Ast element splices it by identity at runtime"
  (doc
    "The identity splice arm works at RUNTIME, not only for constants: a fixed-length list whose
           element is a runtime-built `Ast` node (`(Ast.Int n)` with n a boundary parameter) splices
           each element as-is. `(main n) = `(f ,@(list (Ast.Int n) (Ast.Int 8)))` then reads back child
           1's Int payload — with n=5 the spliced node is `(Ast.Int 5)`, so the payload is 5. Exercises
           the runtime path (the module carries a real value-heap build, not a folded constant), pinning
           that the `(List Ast)` identity does not require compile-time-constant elements.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (quasiquote (f (unquote-splicing #list((Ast.Int (BigInt.of n)) (Ast.Int 8)))))
          ((Ast.List ys) (match (List.at ys 1) ((Option.Some (Ast.Int v)) v) (_ 0N)))
          (_ 0N)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 BigInt))
  (live-objects known-leak))

(case
  "unquote-splicing a list of nested lists lifts each list element into an Ast.ListCtor"
  (doc
    "Splicing `xs = (list (list 1) (list 2))` into `` `(f ,@xs) `` splices xs's two ELEMENTS into `(f …)`;
           each element is a CONSTANT list value, RECURSIVELY reifiable to its dedicated `Ast.ListCtor`
           exactly as `(quote #list(1))` reflects (see \"a quoted collection or member access equals the node
           built by its dedicated Ast ctor\" above). So the splice builds `(Ast.List (Ast.Name \"f\")
           (Ast.ListCtor (Ast.Int 1)) (Ast.ListCtor (Ast.Int 2)))` — the same shape a scalar splice builds,
           one level deeper. The splice-lift recurses into a constant list value (each element lifted through
           the same node-builder, so arbitrary nesting depth works), the recursive companion of
           quote-of-collections. Was the LAST bare `(declines)` corpus-wide (a scalar-only splice-lift
           declined it); converting it to this value-assertion discharged the (declines)-deprecation
           (v-deferral-declines, 2026-08-31), and the recursive lift now flips it to PASS.")
  (input
    (=
      (let ((xs #list(#list(1) #list(2)))) (quasiquote (f (unquote-splicing xs))))
      (Ast.List
        #list((Ast.Name "f") (Ast.ListCtor #list((Ast.Int 1))) (Ast.ListCtor #list((Ast.Int 2)))))))
  (output (: true Bool)))

; --- Splicing requires a list --------------------------------------------------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation (witnessed above): `,@`
; "evaluates <list-expr> to a LIST and splices its elements into the parent." So splicing a value
; that is NOT a list — a scalar, a tuple, a string — has no elements to splice and is ill-typed:
; the compiler MUST reject it (CDZ0201) with the splice's non-list operand named — `unquote-splicing`
; is a recognized form, not an unbound name. A generation that does not yet check the splice operand's
; list type declines rather than running the program (reject-don't-miscompile).
(case
  "splicing a non-list value into a quasiquote is a type error"
  (doc
    "`,@` splices the ELEMENTS of a list; splicing a non-list has no elements to splice.
           `(f ,@x)` with x bound to the Int64 `5` is ill-typed — the compiler MUST reject it
           (CDZ0201, metaprogramming.md: ,@ evaluates its operand to a LIST). A generation that does
           not yet check the splice operand's list type declines rather than running the program.")
  (input (let ((x 5)) (quasiquote (f (unquote-splicing x)))))
  (error CDZ0201))

(case
  "splicing an integer literal directly is a type error"
  (doc
    "The directly-written companion: `(unquote-splicing 5)` inside a quasiquote splices the
           literal `5`, which is not a list — a type error (CDZ0201). The rejection names the splice's
           non-list operand; `unquote-splicing`/`quasiquote` are recognized forms, not names, so this
           is not an unbound-name failure.")
  (input (quasiquote ((unquote-splicing 5) 3)))
  (error CDZ0201))

(case
  "splicing a string value into a quasiquote is a type error (a string is not a list to splice)"
  (doc
    "The STRING companion of the non-list splice reject: `(f ,@\"ab\")` splices a String, which is a
           byte sequence, not a `(List _)` whose elements splice — so it is ill-typed, CDZ0201, exactly as
           the Int64 and integer-literal non-list splices above. Pins that the splice-operand list check is
           by TYPE (String is not a list), not only by scalar-vs-compound shape.")
  (input (do (def (main) (quasiquote (f (unquote-splicing "ab")))) (export main)))
  (error CDZ0201))

(case
  "an unbound unquote-splicing operand keeps its own unbound-name error, not a spurious non-list reject"
  (doc
    "The splice operand is EVALUATED (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation), so an unbound name in it is the ordinary scope error CDZ0101 — PRIMARY, not shadowed
           by a spurious 'not a list' CDZ0201 layered on top. `(f ,@zzz)` with `zzz` unbound is CDZ0101, the
           splice twin of the unbound-inside-an-unquote case.")
  (input (do (def (main) (quasiquote (f (unquote-splicing zzz)))) (export main)))
  (error CDZ0101))

(case
  "quasiquote nests with inner unquote evaluated"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           quasiquote nests, so ``(+ ,,x) evaluates the inner , to produce `(+ ,<x-value>).
           With x=2, ``(+ ,,x) constructs an AST representing `(+ ,2).")
  (input (let ((x 2)) (quasiquote (quasiquote (+ (unquote (unquote x)))))))
  (output
    (:
      (Ast.List
        #list((Ast.Name "quasiquote")
          (Ast.List #list((Ast.Name "+") (Ast.List #list((Ast.Name "unquote") (Ast.Int 2)))))))
      Ast)))

(case
  "nested quasiquote embeds a FLOAT via the inner unquote"
  (doc
    "The float companion of nested-quasiquote: `` ``(+ ,,x) `` with x=2.5 evaluates the inner `,` and
           embeds the float, producing the AST of `` `(+ ,2.5) ``. The lifted value is an `(Ast.Float 2.5)`
           node inside the inert `unquote` structure. Pins that the active-unquote float lift composes with
           quasiquote NESTING (depth tracking) — the inner `,` fires at depth 1 as it does for an integer.")
  (input (let ((x 2.5)) (quasiquote (quasiquote (+ (unquote (unquote x)))))))
  (output
    (:
      (Ast.List
        #list((Ast.Name "quasiquote")
          (Ast.List #list((Ast.Name "+") (Ast.List #list((Ast.Name "unquote") (Ast.Float 2.5)))))))
      Ast)))

(case
  "unquote outside quasiquote is a syntax error"
  (doc
    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , and ,@ are only valid inside quasiquote context. Bare ,x is a syntax error — there's
           no quasiquote template to insert into — so the compiler rejects it at parse time (CDZ0003,
           the syntax-band unquote-outside-quasiquote code) rather than running the program.")
  (input (unquote x))
  (error CDZ0003))

(case
  "unquote-splicing outside quasiquote is a syntax error"
  (doc
    "The `,@` companion of the bare-`,` case above: metaprogramming.md #Quasiquote Constructs AST With
           Selective Evaluation says BOTH unquote and unquote-SPLICING outside a quasiquote context MUST be a
           syntax error. `(unquote-splicing x)` with no enclosing `` ` `` has no template to splice into, so
           the compiler rejects it at PARSE time (CDZ0003, the syntax-band unquote-outside-quasiquote code) —
           the same band as `,`, not a name/type error — rather than running the program. Pins the splicing
           half of the MUST that the bare-unquote case leaves untested (a distinct parse path).")
  (input (unquote-splicing x))
  (error CDZ0003))

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
(case
  "an unquote nested inside a plain quote is a syntax error, not an active unquote"
  (doc
    "`(quote (g ,x))` places an unquote inside a PLAIN quote — still outside any quasiquote
           context (a quote body is inert data, not a selective-evaluation template), so it is the same
           `,`-outside-quasiquote syntax error the bare `,x` case pins, rejected CDZ0003. metaprogramming.md
           #Quote Produces An AST Value forbids `quote` from evaluating its body; a compiler that treats
           plain quote as an active quasiquote level evaluates `,x` and makes `(quote (g ,x))` behave as
           the quasiquote `` `(g ,x) ``. The bug: the active-unquote test fires at quote's own nesting
           level rather than only inside a quasiquote (spec/learnings/2026-07-07-plain-quote-evaluated-a-nested-unquote-instead-of-treating-it-as-inert.md).")
  (input (quote (g (unquote x))))
  (error CDZ0003))

; `unquote` takes EXACTLY ONE operand — the expression to evaluate and embed. `(unquote 1 2)` supplies
; two, so it is malformed and the compiler MUST reject it (CDZ0201), never index just the first and
; drop the rest. The same arity check applies to an unquote encountered during quasiquote expansion as
; to one outside a quasiquote, so `` `(unquote 1 2) `` is rejected rather than silently truncated to
; `(Ast.Int 1)`. (Same class as over-applying a constructor `(Some 1 2)`, here for the `unquote` form
; inside a template.)
(case
  "unquote with more than one operand inside a quasiquote is malformed"
  (doc
    "`(unquote 1 2)` inside a quasiquote gives `unquote` two operands where it takes exactly one —
           a malformed form the compiler MUST reject at compile time (CDZ0201) rather than silently
           take the first operand and drop the rest to yield `(Ast.Int 1)`. The same arity check
           applies during quasiquote expansion as outside a quasiquote.")
  (input (quasiquote (unquote 1 2)))
  (error CDZ0201))

(case
  "quasiquote makes AST construction readable"
  (doc
    "Witnesses compiler-pipeline.md #The Compiler Constructs AST Values Via Quasiquote:
           building an AST value via quasiquote is readable. Compare `(op-const ,n) vs
           (Ast.List (list (Ast.Name \"op-const\") n)) — quasiquote reads like the form it builds.
           This is the frontend/macro role quasiquote serves; the compiler's instruction backend
           instead uses a dedicated typed instruction sum built by ordinary constructors and matched
           to bytes (compiler-pipeline.md #The Compiler Operates On AST Values). Note: dotted names
           like i64.const expand to member access; hyphenated names avoid this.")
  (input (let ((n 42)) (quasiquote (op-const (unquote n)))))
  (output (: (Ast.List #list((Ast.Name "op-const") (Ast.Int 42))) Ast)))

(case
  "Ast.decode converts bytes to an AST sum type value"
  (doc
    "Witnesses compiler-pipeline.md #The Compiler Operates On AST Values: the compiler receives
           a program as binary bytes and decodes it to an AST sum type value. `Ast.decode : Bytes →
           Result<Ast, _>` is total over possibly-external bytes (value-interchange.md — it never traps),
           so the compiler matches the `Ok` arm and then pattern-matches the AST within it.")
  (input (match (Ast.decode (Ast.encode (quote 42))) ((Ok (Ast.Int n)) n) (_ 0N)))
  (output (: 42 BigInt)))

(case
  "Ast.encode and Ast.decode round-trip"
  (doc
    "Witnesses contracts/ast-encoding.md: encoding an AST to binary and decoding it back
           produces the same AST value. The compiler relies on this: it decodes the input, operates
           on AST values, and the encoding is faithful. `Ast.decode` is total (`Bytes → Result<Ast, _>`),
           so the round-trip matches the `Ok` arm and equates its payload to the original.")
  (input
    (match (Ast.decode (Ast.encode (quote (+ 1 2)))) ((Ok a) (= a (quote (+ 1 2)))) ((Err _) false)))
  (output (: true Bool)))

(case
  "a const Ast-list FILTERED by a nested recursive comment-peeler const-evaluates at compile time"
  (doc
    "The general const-EVALUATOR (DESIGN-general-const-eval.md, Stage b) interprets a total function
           applied to compile-time-constant AST values to a constant value, so a real self-reflection
           transform composes and folds. This is the `collect-types` shape a comment-tolerant contract-id
           transform needs: `collect` recurses down a `const (List Ast)`, and for EACH form binds `g = peel h`
           — where `peel` is ITSELF a recursive comment-unwrapper (`(comment … form)` → its wrapped form) that
           calls `child`/`head-name`/`name-of` (Ast destructors over `Ast.List`/`Ast.Name` + `List.at`'s
           `Option`) — then FILTERS: it prepends `g` only when its head is `type`, else drops it. Every
           sub-recursion (peel, collect) and destructor composes as VALUES: the const-demanding `Ast.encode`
           forces the whole thing to a constant, so `Bytes.len` of the encoded filtered list is positive. The
           unroll-and-refold could not fold this (a recursion consuming another recursion's const result, a
           let-bound nested-recursion result carried through a filter); the evaluator does.")
  (input
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def (peel (const (: x Ast))) (if (= (head-name x) "comment") (peel (child x 2)) x))
      (def
        (collect (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t))
            (let
              ((g (peel h)) (tail (collect t)))
              (if (= (head-name g) "type") (List.prepend tail g) tail)))))
      (def
        (main)
        (>
          (Bytes.len
            (Ast.encode
              (Ast.List
                (collect
                  #list((Ast.List
                      #list((Ast.Name "comment")
                        (Ast.Str "c")
                        (Ast.List #list((Ast.Name "type") (Ast.Name "A")))))
                    (Ast.List #list((Ast.Name "type") (Ast.Name "B"))))))))
          0))
      (export main)))
  (output (: true Bool)))

(case
  "a const-folded RECORD descriptor holds a recursive-transform result, projected at compile time"
  (doc
    "The operator's record-API shape (`contract(m) -> Record(id, …, types)`, caller reads a field):
           the general const-evaluator handles RECORD construction + FIELD PROJECTION as values, so a
           descriptor whose field is computed by a recursive Ast transform folds and the projection reads it
           at compile time. `build` returns a record with a `types` field and a `count` field — the
           `types` field is the recursive comment-peeling type-filter over a `const (List Ast)`. Projecting
           `.types` and const-encoding it yields positive bytes: the record, the recursion inside it, and the
           projection all const-evaluate. This is the substrate for a userspace contract descriptor built
           from a self-reflected module (the field is read at compile time, no record built at run time).")
  (input
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def (peel (const (: x Ast))) (if (= (head-name x) "comment") (peel (child x 2)) x))
      (def
        (collect (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t))
            (let
              ((g (peel h)) (tail (collect t)))
              (if (= (head-name g) "type") (List.prepend tail g) tail)))))
      (def (build (const (: xs (List Ast)))) #record((= types (Ast.List (collect xs))) (= count 7)))
      (def
        (main)
        (>
          (Bytes.len
            (Ast.encode
              (.
                (build
                  #list((Ast.List
                      #list((Ast.Name "comment")
                        (Ast.Str "c")
                        (Ast.List #list((Ast.Name "type") (Ast.Name "A")))))
                    (Ast.List #list((Ast.Name "type") (Ast.Name "B")))))
                types)))
          0))
      (export main)))
  (output (: true Bool)))

; The RUNTIME counterpart of the compile-time projected-descriptor case above: a record carrying an `Ast`-typed
; field can be returned as a WHOLE runtime value, not merely projected away at compile time. An `Ast` value is
; runtime-representable (an ordinary sum value the runtime holds and `match` reads), so the descriptor record
; materializes at run time. These pin the record-return invariant the operator's uniform contract descriptor
; depends on — returning the whole descriptor, not just a projected field, compiles and runs.
(case
  "a record carrying an Ast-typed field materializes as a WHOLE runtime value and its Ast field is matched"
  (doc
    "The record-return invariant the operator's uniform contract descriptor depends on: a record whose
           field is `Ast`-typed can be built and returned as a WHOLE RUNTIME value (not merely projected away
           at compile time), because an `Ast` value is runtime-representable — it is an ordinary sum value the
           runtime holds and `match` reads. A runtime `Bool` selects between two records so neither the record
           nor its `Ast` field can be folded away; the selected record is bound whole (`r`), and its `Ast` field
           is projected (`. r input`) and MATCHED at run time. `f true` builds the first record and its
           `Ast.Name` field matches → 1. Pins that returning a record with an `Ast` field materializes at
           runtime (the shape that formerly only survived via a compile-time projection fold).")
  (input
    (do
      (def
        (f (: b Bool))
        (let
          ((r
              (if
                b
                #record((= name "a") (= input (Ast.Name "i")))
                #record((= name "b") (= input (Ast.List #list()))))))
          (match r.input ((Ast.Name _) 1) (_ 0))))
      (def (main (: b Bool)) (f b))
      (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "the full contract-descriptor record (Bytes/String/Ast fields) materializes whole at runtime; scalar fields read"
  (doc
    "The full self-describing descriptor shape `Record(id: Bytes, name: String, input: Ast, output: Ast,
           types: Ast)` returned as a WHOLE runtime value: a runtime `Bool` picks between two fully-populated
           descriptors so the record materializes rather than folding, then its runtime-representable scalar
           fields are projected at run time — `Bytes.len (. r id)` + `String.byte-len (. r name)`. `f true`
           reads `b\"idA\"` (len 3) and `\"nameA\"` (byte-len 5) → 8; `f false` reads `b\"idBB\"` (4) and
           `\"nameBB\"` (6) → 10. Pins that the mixed-field descriptor record (including the `Ast`-typed
           input/output/types) is a legal runtime value, so a consumer that returns the whole descriptor — not
           just a projected field — compiles and runs.")
  (input
    (do
      (def
        (desc (: b Bool))
        (if
          b
          #record((= id b"idA")
            (= name "nameA")
            (= input (Ast.Name "i"))
            (= output (Ast.Name "o"))
            (= types (Ast.List #list())))
          #record((= id b"idBB")
            (= name "nameBB")
            (= input (Ast.Name "j"))
            (= output (Ast.Name "p"))
            (= types (Ast.List #list((Ast.Name "z")))))))
      (def (main (: b Bool)) (let ((r (desc b))) (+ (Bytes.len r.id) (String.byte-len r.name))))
      (export main)))
  (call main (: true Bool))
  (output (: 8 Int64))
  (call main (: false Bool))
  (output (: 10 Int64)))

(case
  "the operator's uniform all-Bytes/String contract descriptor materializes whole at runtime"
  (doc
    "The operator's target uniform contract descriptor: ONE record of purely runtime-representable
           fields — `Record(id: Bytes, name: String, encodedInput: Bytes, encodedAst: Bytes)` — where the
           encoded fields are `Ast.encode` of the module's own reflected AST (compile-time-const, so they fold
           to `Bytes` constants). A runtime `Bool` selects between the real descriptor and an empty stub so the
           whole record materializes as a runtime value rather than folding to a projected constant; its fields
           are then projected at run time and their byte/char lengths summed. The check is content-agnostic:
           the real descriptor's summed length exceeds the fixed non-encoded part (id 3 bytes + name 12 chars),
           since `Ast.encode(Ast.module)` folds to a non-empty canonical `cdzast` document, so `main true`
           yields `true`; the empty stub sums to 0, so `main false` yields `false`. Pins that the uniform,
           all-runtime-representable descriptor record is returnable as a WHOLE value — the shape v-platform's
           uniform-descriptor redesign depends on.")
  (input
    (do
      (def
        (descriptor)
        #record((= id b"\x01ID")
          (= name "temp.celsius")
          (= encodedInput (Ast.encode (Ast.List #list((Ast.Name "input") (Ast.Name "Temp")))))
          (= encodedAst (Ast.encode Ast.module))))
      (def (stub) #record((= id b"") (= name "") (= encodedInput b"") (= encodedAst b"")))
      (def
        (main (: b Bool))
        (let
          ((r (if b (descriptor) (stub))))
          (>
            (+
              (+ (Bytes.len r.id) (String.byte-len r.name))
              (+ (Bytes.len r.encodedInput) (Bytes.len r.encodedAst)))
            15)))
      (export main)))
  (call main (: true Bool))
  (output (: true Bool))
  (call main (: false Bool))
  (output (: false Bool))
  (live-objects 0))

(case
  "a recursive transform composed over Ast.module const-evaluates at compile time"
  (doc
    "The general const-evaluator evaluates a total function applied to `Ast.module` — the reflected
           enclosing module `Ast` — to a constant, so a self-reflection transform that RECURSES over the
           module's own forms folds. `Ast.module` folds (via `core_of`) to a constant `Ast.List` of the
           module's forms; the evaluator converts that constant to a value, so `collect` (a recursive filter
           that calls the recursive comment-peeler `peel` per form) composes over it and the const-encode
           forces the whole thing to a constant — the `Ast.module`-SOURCE depth gap that declined before
           (the SAME transform folds over a plain literal but declined when the source was `Ast.module`-
           derived, because the reflected forms did not propagate as values through the nested recursion).
           This is the compiler capability a userspace contract-id transform needs: run a recursive transform
           over the module's OWN reflected AST at compile time. Content-agnostic: the encoded filtered form
           list has a non-negative length regardless of what this module contains, so a `> -1` check folds to
           `true` — and the point is that it FOLDS (a non-folding transform would decline at `Ast.encode`).")
  (input
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def (peel (const (: x Ast))) (if (= (head-name x) "comment") (peel (child x 2)) x))
      (def
        (collect (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t))
            (let
              ((g (peel h)) (tail (collect t)))
              (if (= (head-name g) "type") (List.prepend tail g) tail)))))
      (def (forms-of (const (: mm Ast))) (match mm ((Ast.List fs) fs) (_ (: #list() (List Ast)))))
      (def (main) (> (Bytes.len (Ast.encode (Ast.List (collect (forms-of Ast.module))))) -1))
      (export main)))
  (output (: true Bool)))

(case
  "a TWO-PASS transform over Ast.module — one recursion consuming another's result — const-evaluates"
  (doc
    "The sharpest form of the `Ast.module`-source composition: the result of ONE recursion over the
           reflected module forms feeds a SECOND recursion. `unwrap-all` rebuilds the forms, peeling each
           through the recursive `unwrap` (pass 1); `keep-types` then FILTERS that result to the `type`
           declarations (pass 2). Because `Ast.module` const-evaluates to a value (not just an encodable
           constant), the pass-1 result flows as a fully-const value into pass 2 — the exact case that
           declined before (`keep-types(unwrap-all(Ast.module-forms))` folds over a plain literal but declined
           over `Ast.module`, because a recursion-result rooted at `Ast.module` was not recognized as a
           constant by the second recursion). This two-pass-over-the-reflected-module is the composition a
           comment-tolerant contract-id transform performs; the const-encode forces it to a constant.
           Content-agnostic (`> -1` → `true`): the point is that it FOLDS, not the exact byte length.")
  (input
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def (peel (const (: x Ast))) (if (= (head-name x) "comment") (peel (child x 2)) x))
      (def
        (unwrap-all (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t)) (List.prepend (unwrap-all t) (peel h)))))
      (def
        (keep-types (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t))
            (if (= (head-name h) "type") (List.prepend (keep-types t) h) (keep-types t)))))
      (def (forms-of (const (: mm Ast))) (match mm ((Ast.List fs) fs) (_ (: #list() (List Ast)))))
      (def
        (main)
        (> (Bytes.len (Ast.encode (Ast.List (keep-types (unwrap-all (forms-of Ast.module)))))) -1))
      (export main)))
  (output (: true Bool)))

(case
  "decoding bytes that are not a canonical AST yields the error case, not a trap"
  (doc
    "contracts/value-interchange.md #Decode Inverts Serialize And Refuses Otherwise + #A Decode Over
           External Bytes Is Total: `Ast.decode` consumes bytes that may come from an EXTERNAL source, so it
           MUST be total — a byte sequence that is not the canonical encoding of any AST yields the error
           case (`Err`), NOT a trap. `(Bytes.of (list 255 255 255))` is not a valid AST encoding, so the
           decode returns `Err` and the program handles it as an ordinary value. This is the fallible-reader
           discipline (like `String.from-bytes`), not reject-don't-miscompile: malformed EXTERNAL input is a
           handleable condition, not a program bug that traps.")
  (input (match (Ast.decode (Bytes.of #list(255 255 255))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decoding canonical bytes followed by a trailing byte yields the error case"
  (doc
    "contracts/deterministic-value-form.md #Decoding Is The Inverse Of The Canonical Byte Form: a byte
           sequence that is valid canonical bytes FOLLOWED BY additional bytes MUST NOT decode as the value
           the valid prefix encodes — the trailing byte is a detected error, not silently ignored. So
           `Ast.decode` of `(encode (Ast.Int 7)) ++ [99]` yields `Err`, not `Ok (Ast.Int 7)`. The total-decode
           companion of the round-trip cases: decode consumes the WHOLE input or reports an error, so a
           truncated or concatenated external input is caught rather than half-read.")
  (input
    (match
      (Ast.decode (Bytes.concat (Ast.encode (Ast.Int 7)) (Bytes.of #list(99))))
      ((Ok _) 1)
      ((Err _) 0)))
  (output (: 0 Int64)))

; Ast.decode stays TOTAL (Err, never a trap) on adversarial bytes that specifically exercise EACH decode
; arm's bounds/finite check: FLOAT (tag 0x05, 8-byte f64), STR (tag 0x04, len-prefixed UTF-8), LIST (tag
; 0x02, 4-byte-count-prefixed elements), INT (tag 0x00, fixed 8-byte payload), plus an unknown tag and an
; empty (no-tag) input. The generic non-canonical/trailing-byte cases above don't reach the individual
; arms; a change to ANY arm that dropped a bounds/length/finite check would pass those generic cases yet
; regress the matching per-arm case here. All → Err. (The LIST-count and INT-length arms are the
; companions of the Float/Str arms this vertical added — a dropped List count-bound would over-read or
; attempt a 4-billion-element build on `(2 ff ff ff ff)`, and a short Int payload would partial-read.)
(case
  "decode of a truncated Float tag yields the error case, not a trap"
  (doc
    "The Float decode arm reads 8 bytes after tag 0x05; a truncated payload (tag + only 3 bytes) is
           not a canonical encoding, so `Ast.decode` returns `Err` (value-interchange.md #A Decode Over
           External Bytes Is Total). Pins the length check on the Float arm — never a partial read or trap.")
  (input (match (Ast.decode (Bytes.of #list(5 1 2 3))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of a Float tag with a non-finite (NaN) bit pattern yields the error case"
  (doc
    "A Float payload whose 8 bytes are a NaN bit pattern (`7ff8…0001`) has no finite `Decimal` value
           form, so the decode reports `Err` rather than fabricating a non-finite `Ast.Float` — the decode
           arm rejects a non-finite double. Pins that the byte→Decimal step stays total on NaN/inf.")
  (input (match (Ast.decode (Bytes.of #list(5 1 0 0 0 0 0 248 127))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of a Str tag with an oversized length yields the error case"
  (doc
    "The Str decode arm reads a 4-byte length then that many UTF-8 bytes; a length (255) exceeding
           the bytes present is not a canonical encoding, so `Ast.decode` returns `Err`. Pins the Str arm's
           bounds check — never reads past the input.")
  (input (match (Ast.decode (Bytes.of #list(4 255 0 0 0))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an unknown tag byte yields the error case"
  (doc
    "A leading tag byte the encoding does not assign (0x09 — beyond Int/Name/List/Bool/Str/Float =
           0x00..0x05) is not a canonical AST, so `Ast.decode` returns `Err`. Pins that the tag dispatch's
           fallthrough is a clean decline, not a trap — total over ANY external byte.")
  (input (match (Ast.decode (Bytes.of #list(9 0 0 0 0))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of a List tag whose element count exceeds the bytes present yields the error case"
  (doc
    "The List decode arm reads a 4-byte element count then that many child nodes; a count (255) with
           NO element bytes following is not a canonical encoding, so `Ast.decode` returns `Err`. The
           companion of the Str-oversized-length case for the LIST arm (tag 0x02): pins that the count is
           bounds-checked against the remaining input — never an over-read past the end. A decode that
           trusted the count would read past the buffer (trap) or half-build a list.")
  (input (match (Ast.decode (Bytes.of #list(2 255 0 0 0))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of a List tag with an enormous element count does not attempt a giant build — yields the error case"
  (doc
    "A List count of 0xffffffff (~4.29 billion) with no element bytes present must be caught by the
           bounds check and reported `Err`, NOT drive a 4-billion-iteration build or an out-of-memory trap.
           Pins that the List arm validates the count against the ACTUAL remaining bytes before allocating
           or looping — total over an adversarial count, not merely a small-count truncation.")
  (input (match (Ast.decode (Bytes.of #list(2 255 255 255 255))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

; The Int payload is NON-LOSSY (operator directive: parametric integers must never silently truncate),
; encoded as tag 0x00 + 1 sign byte (0 non-negative, 1 negative) + a 4-byte LE u32 magnitude length + that
; many big-endian magnitude bytes (ast-encoding.md; same length-prefix framing as Str/List). `Ast.decode`
; stays TOTAL over this variable-length form, and enforces the canonical `IntValue` invariant so the
; encoding is a bijection: exactly these decode adversarial inputs to `Err`, never a panic or wrong value.
(case
  "decode of an Int tag with a truncated magnitude-length yields the error case"
  (doc
    "After tag 0x00 the Int arm reads a 4-byte LE magnitude length; `(0 1 2 3)` supplies the sign
           byte (1) then only TWO of the four length bytes, so the length prefix is truncated and
           `Ast.decode` returns `Err`. The variable-length successor of the old fixed-8-byte truncation
           case: pins that the length prefix itself is bounds-checked, never partial-read.")
  (input (match (Ast.decode (Bytes.of #list(0 1 2 3))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an Int tag with a maximal magnitude-length yields the error case without overflowing"
  (doc
    "`(0 0 255 255 255 255)` declares a magnitude length of 0xFFFFFFFF (~4.29 billion) with no
           magnitude bytes following. Computing the payload end as `4 + length` would OVERFLOW usize on a
           32-bit target (wasm32 — rcdzc self-hosts to wasm), panicking under overflow-checks and breaking
           the never-panic-on-untrusted-input contract; a `checked_add` returns `Err` instead. Pins that
           the length arithmetic is overflow-safe on the adversarial maximal length, not merely
           bounds-checked against present bytes (github-liaison/Copilot PR#747).")
  (input (match (Ast.decode (Bytes.of #list(0 0 255 255 255 255))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an Int tag whose magnitude is shorter than its declared length yields the error case"
  (doc
    "The Int arm reads `length` magnitude bytes after the 4-byte length; `(0 0 2 0 0 0)` declares a
           2-byte magnitude but supplies ZERO magnitude bytes, so `Ast.decode` returns `Err`. The
           magnitude-side companion of the truncated-length case — pins the bounds check on the magnitude
           read, mirroring the Str/List length-vs-present cases.")
  (input (match (Ast.decode (Bytes.of #list(0 0 2 0 0 0))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an Int tag with a leading-zero magnitude byte is non-canonical and yields the error case"
  (doc
    "The `IntValue` invariant is a MINIMAL big-endian magnitude (no leading zero bytes), so the
           encoding is a bijection with one canonical form. `(0 0 2 0 0 0 0 1)` (sign 0, length 2,
           magnitude `00 01`) has a leading zero byte — a non-canonical encoding of the value 1 — so
           `Ast.decode` returns `Err` rather than accepting a second spelling of the same value. Pins that
           decode rejects a non-minimal magnitude, keeping encode/decode a bijection.")
  (input (match (Ast.decode (Bytes.of #list(0 0 2 0 0 0 0 1))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an Int tag marked negative with a zero-length magnitude is non-canonical (no negative zero)"
  (doc
    "Zero's one canonical encoding is sign 0 + length 0 (no magnitude bytes); there is no negative
           zero. `(0 1 0 0 0 0)` marks the sign NEGATIVE with a zero-length magnitude — a negative zero,
           not canonical — so `Ast.decode` returns `Err`. Pins the signed-zero canonicalization on the
           decode side, the byte-codec companion of the text-path negative-zero float cases.")
  (input (match (Ast.decode (Bytes.of #list(0 1 0 0 0 0))) ((Ok _) 1) ((Err _) 0)))
  (output (: 0 Int64)))

(case
  "decode of an empty byte string (no tag) yields the error case"
  (doc
    "The zero-length input has no leading tag byte to dispatch on, so `Ast.decode` returns `Err`
           rather than trapping on an empty read — the total-decode contract holds at the degenerate
           boundary (value-interchange.md #A Decode Over External Bytes Is Total). Pins that the tag read
           itself is bounds-checked, not just the per-arm payloads.")
  (input (match (Ast.decode (Bytes.of #list())) ((Ok _) 1) ((Err _) 0)))
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
(case
  "a quote pattern binds an unquoted operand of a compound form"
  (doc
    "`` `(+ ,a ,b) `` in pattern position IS `(Ast.List (list (Ast.Name \"+\") a b))` as a pattern
           (options/quote-patterns/quasiquote-pattern.md): the literal head `+` matches `(Ast.Name \"+\")`
           by equality, and `,a`/`,b` bind the two operand sub-ASTs. Matching `(quote (+ 3 5))` binds
           a=`(Ast.Int 3)` and b=`(Ast.Int 5)`; the arm returns b, so the AST for 5. Pins the core
           destructuring: unquote is the binder, a literal subterm matches by equality. The catch-all
           `other` is an ordinary bare-name pattern — `,` is meaningful only inside a `` ` `` template.")
  (input (match (quote (+ 3 5)) ((quasiquote (+ (unquote a) (unquote b))) b) (other other)))
  (output (: (Ast.Int 5) Ast)))

(case
  "a quote pattern is equivalent to the Ast.* constructor pattern"
  (doc
    "A quote pattern lowers to the `Ast.*` constructor pattern, so the two spellings bind
           identically. `` `(+ ,a ,b) `` and `(Ast.List (list (Ast.Name \"+\") a b))` matched against the
           same `(quote (+ 1 2))` both bind a=`(Ast.Int 1)`; comparing the two bound values is true. Pins
           the equivalence the form rests on — the pattern adds a surface, not a second mechanism.")
  (input
    (=
      (match (quote (+ 1 2)) ((quasiquote (+ (unquote a) (unquote b))) a) (_ (Ast.Int 0)))
      (match (quote (+ 1 2)) ((Ast.List #list((Ast.Name "+") a b)) a) (_ (Ast.Int 0)))))
  (output (: true Bool)))

(case
  "a literal subterm in a quote pattern matches by equality"
  (doc
    "A literal head/subterm matches by equality — the direct analogue of a literal value pattern
           `(match 2 (2 \"two\") …)`. `` `(+ ,a ,b) `` matches only a form headed by `+`; against
           `(quote (- 3 5))`, whose head is `-`, it does NOT match, so control falls to the `other`
           catch-all. Pins that the literal name in the template constrains the head, not merely the
           arity.")
  (input (match (quote (- 3 5)) ((quasiquote (+ (unquote a) (unquote b))) 1) (other 0)))
  (output (: 0 Int64)))

; The literal-head case above constrains the HEAD. These pin a literal OPERAND subterm (a peephole-rewrite
; idiom, `` `(+ ,x 0) `` for `(+ x 0) ⇒ x`) matches with the same precision a value literal-pattern has: it
; is POSITION-sensitive (the `0` must be that operand, not a commuted one), matches ONLY the exact literal
; (not a different one), and respects the Int/Float literal DISTINCTION (an `(Ast.Int 0)` pattern does not
; match an `(Ast.Float 0.0)` subterm) — the metaprogramming analogue of the scalar-literal pattern's
; type/value exactness. A quote-pattern literal that commuted, matched loosely, or conflated Int/Float would
; make a rewrite rule fire on the wrong AST (an unsound refactor).
(case
  "a quote pattern's literal operand is position-sensitive"
  (doc
    "`` `(+ ,x 0) `` matches an addition whose SECOND operand is the literal `0`, binding the first.
           Against `(quote (+ 0 y))` — the `0` is the FIRST operand — it does NOT match (the pattern is not
           commuted), so `simp` leaves it unchanged. Pins that a literal operand constrains its POSITION, so
           a `(+ x 0) ⇒ x` rewrite does not misfire on `(+ 0 y)` (a different, non-simplifiable shape here).")
  (input
    (do
      (def (simp node) (match node ((quasiquote (+ (unquote x) 0)) x) (other other)))
      (def (main) (= (simp (quote (+ 0 y))) (quote (+ 0 y))))
      (export main)))
  (output (: true Bool)))

(case
  "a quote pattern's literal operand matches only the exact literal"
  (doc
    "`` `(+ ,x 0) `` matches only when the second operand is exactly `0`; against `(quote (+ y 1))` —
           second operand `1` ≠ `0` — it does NOT match, so `simp` leaves it unchanged. Pins the literal
           operand is an equality test on the exact value, the operand analogue of the literal-HEAD case
           above — a `(+ x 0) ⇒ x` rule does not fire on `(+ x 1)`.")
  (input
    (do
      (def (simp node) (match node ((quasiquote (+ (unquote x) 0)) x) (other other)))
      (def (main) (= (simp (quote (+ y 1))) (quote (+ y 1))))
      (export main)))
  (output (: true Bool)))

(case
  "a quote pattern's Int literal does not match a Float literal of the same value"
  (doc
    "The Int/Float literal distinction in a quote pattern: `` `(+ ,x 0) `` has an `(Ast.Int 0)` literal
           subterm, which does NOT match an `(Ast.Float 0.0)` subterm even though 0 and 0.0 are numerically
           equal — they are distinct AST leaves. Against `(quote (+ y 0.0))` the pattern does not match, so
           `simp` leaves it unchanged. Pins that a quote-pattern literal respects the Int-vs-Float leaf
           distinction (the metaprogramming analogue of the scalar-literal pattern's type exactness) — a
           rewrite keyed on the integer `0` must not misfire on the float `0.0`.")
  (input
    (do
      (def (simp node) (match node ((quasiquote (+ (unquote x) 0)) x) (other other)))
      (def (main) (= (simp (quote (+ y 0.0))) (quote (+ y 0.0))))
      (export main)))
  (output (: true Bool)))

(case
  "a quote pattern matches a fixed arity"
  (doc
    "A compound template `` `(f ,a ,b) `` matches an `Ast.List` of EXACTLY three elements — the
           reading of `(Ast.List (list (Ast.Name \"f\") a b))`, whose `(list …)` sub-pattern fixes
           length. `(quote (f 1 2 3))` has four elements, so it does NOT match the two-operand pattern and
           falls to the catch-all. Pins fixed arity: variable length is expressed only through `,@`.")
  (input (match (quote (f 1 2 3)) ((quasiquote (f (unquote a) (unquote b))) 2) (other 9)))
  (output (: 9 Int64)))

; The zero-arity end of the fixed-arity rule: the empty-compound quote pattern `` `() `` reads as
; `(Ast.List (list))` — an `Ast.List` whose `(list)` sub-pattern fixes length ZERO — so it matches ONLY a
; quoted empty compound and nothing else. The pattern-side companion of the construction case "quoting an
; empty compound produces an empty Ast.List": there `(quote ())` BUILDS the empty list; here `` `() ``
; MATCHES it. A quote pattern whose empty-list sub-pattern was lowered to a wildcard (or a >=0 rest) would
; wrongly match a non-empty form — these two cases pin the exact-zero-length discrimination.
(case
  "an empty-compound quote pattern matches a quoted empty compound"
  (doc
    "`` `() `` is the reading of `(Ast.List (list))`, so it matches an `Ast.List` of EXACTLY zero
           elements — the quoted empty compound `(quote ())`. Pins the zero-arity end of the fixed-arity
           quote-pattern rule (the empty case of the `(list …)` sub-pattern length fix).")
  (input (match (quote ()) ((quasiquote ()) 1) (other 0)))
  (output (: 1 Int64)))

(case
  "an empty-compound quote pattern does not match a non-empty form"
  (doc
    "The discriminating companion: `` `() `` fixes length zero, so a quoted ONE-element compound
           `(quote (a))` does NOT match it and falls to the catch-all. A lowering that treated the empty
           list sub-pattern as a wildcard or a zero-or-more rest would wrongly match here — this pins that
           `` `() `` is an EXACT-zero-length match, not a match-anything.")
  (input (match (quote (a)) ((quasiquote ()) 1) (other 0)))
  (output (: 0 Int64)))

(case
  "a nested unquote pattern matches a sub-AST by shape"
  (doc
    "`,<pattern>` nests an ordinary pattern at the sub-AST's position, so `` `(+ ,(Ast.Int n) ,b) ``
           matches only an addition whose first operand is an INTEGER LITERAL and binds its value to n.
           Against `(quote (+ 7 x))`, the first operand `(Ast.Int 7)` matches `(Ast.Int n)` binding n=7;
           the arm returns n. Pins that unquote takes a full pattern, not only a bare name.")
  (input (match (quote (+ 7 x)) ((quasiquote (+ (unquote (Ast.Int n)) (unquote b))) n) (other 0N)))
  (output (: 7 BigInt)))

; A nested unquote pattern matches ANY Ast leaf variant, not just Int — the Float, Str, Bool, and Name
; variants (the leaves this vertical realized) destructure by shape exactly as `Ast.Int` does. These pin
; the interaction between the quote-pattern surface and those leaves: a `,(Ast.Float n)` matches only a
; float operand and binds its value; a `,(Ast.Str s)` matches only a string operand; a `,(Ast.Bool b)`
; matches only a boolean operand; a `,(Ast.Name n)` matches only an identifier operand and binds its
; spelling. A change to either the quote-pattern lowering or a leaf variant that broke this cross-feature
; match would flip these.
(case
  "a nested unquote pattern matches a Float sub-AST by shape"
  (doc
    "`` `(f ,(Ast.Float n)) `` matches only a compound headed `f` whose operand is a FLOAT literal,
           binding its value. Against `(quote (f 2.5))` the operand `(Ast.Float 2.5)` matches `(Ast.Float
           n)` binding n=2.5, and `= n 2.5` is true. Pins that a quote pattern destructures the `Ast.Float`
           leaf (the float companion of the Int nested-unquote-pattern case above).")
  (input (match (quote (f 2.5)) ((quasiquote (f (unquote (Ast.Float n)))) (= n 2.5)) (other false)))
  (output (: true Bool)))

(case
  "a nested unquote pattern matches a Str sub-AST by shape"
  (doc
    "The string companion: `` `(f ,(Ast.Str s)) `` matches only a compound headed `f` whose operand
           is a STRING literal, binding it. Against `(quote (f \"hi\"))` the operand `(Ast.Str \"hi\")`
           matches, and `String.byte-len s` is 2. Pins that a quote pattern destructures the `Ast.Str` leaf
           (distinct from `Ast.Name` — a string operand, not an identifier).")
  (input
    (match (quote (f "hi")) ((quasiquote (f (unquote (Ast.Str s)))) (String.byte-len s)) (other 0)))
  (output (: 2 Int64)))

(case
  "a nested unquote pattern matches a Bool sub-AST by shape"
  (doc
    "The boolean companion, completing the leaf set: `` `(f ,(Ast.Bool b)) `` matches only a compound
           headed `f` whose operand is a BOOLEAN literal, binding it. Against `(quote (f true))` the operand
           `(Ast.Bool true)` matches `(Ast.Bool b)` binding b=true, so the arm returns true. Pins that a
           quote pattern destructures the `Ast.Bool` leaf exactly as it does Int/Float/Str — the last
           realized leaf in the nested-unquote-pattern family.")
  (input (match (quote (f true)) ((quasiquote (f (unquote (Ast.Bool b)))) b) (other false)))
  (output (: true Bool)))

(case
  "a nested unquote Bool pattern does not match a non-boolean operand"
  (doc
    "The discriminator companion: `` `(f ,(Ast.Bool b)) `` matches ONLY a boolean operand, so against
           `(quote (f 3))` — an INTEGER operand — the quote-pattern arm does NOT fire and control falls to
           the catch-all (→ 0). Pins that the nested-unquote leaf pattern is shape-SELECTIVE (a leaf pattern
           that matched any operand would wrongly bind the Int here), the negative face of the match cases.")
  (input (match (quote (f 3)) ((quasiquote (f (unquote (Ast.Bool b)))) 1) (other 0)))
  (output (: 0 Int64)))

(case
  "a nested unquote pattern binds an Ast.Name operand's identifier"
  (doc
    "The Name companion, completing the leaf set (Int/Float/Str/Bool/Name): `` `(f ,(Ast.Name n)) ``
           matches only a compound headed `f` whose OPERAND is an identifier, binding its spelling to `n`.
           Against `(quote (f g))` the operand `(Ast.Name \"g\")` matches `(Ast.Name n)` binding n=\"g\", so
           `String.byte-len n` is 1. Distinct from the head-by-equality cases (which match a LITERAL name
           `(Ast.Name \"+\")`): here the unquote BINDS the operand name's string. Pins that a quote pattern
           destructures the `Ast.Name` leaf in operand position.")
  (input
    (match (quote (f g)) ((quasiquote (f (unquote (Ast.Name n)))) (String.byte-len n)) (other 0)))
  (output (: 1 Int64)))

(case
  "a final unquote-splice binds the remaining elements as a list"
  (doc
    "A final `,@<name>` binds the remaining list elements as a LIST (never a single element), the
           pattern-position dual of splicing construction. `` `(f ,@args) `` against `(quote (f 1 2 3))`
           binds args to the list `(Ast.Int 1) (Ast.Int 2) (Ast.Int 3)`; `List.len` of it is 3. Pins the
           tail splice binds the rest and that the elements are a list.")
  (input
    (match (quote (f 1 2 3)) ((quasiquote (f (unquote-splicing args))) (List.len args)) (other 0)))
  (output (: 3 Int64)))

(case
  "a quote pattern used to recognize a compiler form reads as that form"
  (doc
    "The self-hosting payoff (options/quote-patterns/quasiquote-pattern.md #Why This Matters For
           Self-Hosting): the compiler's core is a `match` over the decoded AST, and a quote-pattern arm
           reads as the surface it lowers. Here a tiny `lower` distinguishes `(+ …)` from everything else
           by quote pattern; against `(quote (+ 4 6))` it selects the add arm and returns the first
           operand's node. Mirrors the construction idiom `` `(op-const ,n) `` on the pattern side.")
  (input
    (match
      (quote (+ 4 6))
      ((quasiquote (+ (unquote a) (unquote b))) a)
      ((quasiquote (- (unquote a) (unquote b))) b)
      (other other)))
  (output (: (Ast.Int 4) Ast)))

; An Ast match whose arms are only quote patterns does not cover the AST sum — a different head, a
; different arity, or a leaf scrutinee all fail to match — so it is non-exhaustive and rejected CDZ0210,
; exactly as a sum match missing a variant (core-semantics.md #Matching Is Exhaustive Or Rejected). A
; bare-name pattern (equivalently `_`) matches any AST and is the catch-all, so its ABSENCE is what makes
; the match non-exhaustive. Quote matching reuses exhaustiveness rather than adding a rule.
(case
  "a quote-pattern match with no catch-all is non-exhaustive"
  (doc
    "`` `(+ ,a ,b) `` covers only additions; an Ast scrutinee can be a name, an integer, or a
           differently-headed list, none of which it matches. With no bare-name/`_` catch-all arm the
           match does not cover the AST sum and is rejected CDZ0210 — the same rejection a sum match
           missing a variant gets. Pins that quote matching reuses the existing exhaustiveness rule.")
  (input (match (quote (+ 1 2)) ((quasiquote (+ (unquote a) (unquote b))) a)))
  (error CDZ0210))

; `,@` binds the REST and so is only meaningful as the final element of its template: a `,@` before other
; elements would match a variable-length gap in the middle of a fixed sequence, turning a single
; positional scan into a search. That is an ill-formed quote pattern, rejected CDZ0221 (the CDZ02xx
; types-and-patterns band, the quote-pattern companion of the binary-form CDZ0220). Mirrors `bin`, where
; an unsized `(bytes rest)` is legal only as the final segment.
(case
  "a non-final unquote-splice in a quote pattern is ill-formed"
  (doc
    "`,@<name>` binds the remaining elements, so it is meaningful only as the FINAL element of a
           template. `` `(f ,@init ,last) `` puts `,@init` before `,last`, requiring a variable-length gap
           flanked by a fixed tail — an ill-formed quote pattern, rejected CDZ0221
           (options/quote-patterns/quasiquote-pattern.md #Tail Splice Is Final-Position Only). Mirrors the
           binary-form rule that an unsized `bytes` segment is legal only last.")
  (input
    (match
      (quote (f 1 2 3))
      ((quasiquote (f (unquote-splicing init) (unquote last))) last)
      (other other)))
  (error CDZ0221))

; The FLAGSHIP idiom the quote-pattern surface enables: a RECURSIVE evaluator that destructures its `Ast`
; argument by quote patterns and recurses on the bound sub-ASTs. The unquote binders `,x`/`,y` bind the
; operand sub-trees, and `eval-expr` calls itself on them — an interpreter written by pattern-matching over
; the AST, the metaprogramming payoff of quote-in-pattern position. Every case above pins ONE facet (a single
; bind, a leaf shape, a splice, the arity/head constraints); this composes them into the real use — a
; multi-level recursive descent over a built tree. (metaprogramming.md #Quote Produces An AST Value + the
; pattern-position dual; the runtime-Ast destructuring this needs landed on trunk — v-patterns batch-87 +
; runtime-string-pattern `eb0a9f0548`.)
(case
  "a recursive evaluator destructures a built Ast by quote patterns and recurses"
  (doc
    "The interpreter idiom: `eval-expr` matches an `Ast` by quote patterns — `(Ast.Int n)` → the value,
           `` `(+ ,x ,y) `` → `eval-expr(x) + eval-expr(y)`, `` `(* ,x ,y) `` → the product — recursing on the
           unquote-bound sub-ASTs. Over `(quote (* (+ 1 2) 4))` it descends two levels: `(+ 1 2)`→3, then
           `3 * 4`→12. Pins the flagship quote-pattern use (a recursive AST evaluator), composing the
           single-facet cases above into the real multi-level recursive descent a macro/interpreter runs.
           `(Ast.Int n)` binds `n : BigInt` (lossless AST-int storage), so `eval-expr`'s result — and the
           recursive `+`/`*` over it — is `BigInt` (12N); the recursive-call result grounds to that BigInt
           return type through the self-calls.")
  (input
    (do
      (def
        (eval-expr (: a Ast))
        (match
          a
          ((Ast.Int n) n)
          ((quasiquote (+ (unquote x) (unquote y))) (+ (eval-expr x) (eval-expr y)))
          ((quasiquote (* (unquote x) (unquote y))) (* (eval-expr x) (eval-expr y)))
          (_ 0N)))
      (def (main) (eval-expr (quote (* (+ 1 2) 4))))
      (export main)))
  (output (: 12 BigInt))
  (live-objects known-leak))

(case
  "a variadic Ast form is folded via a tail-splice rest-binder over its operands"
  (doc
    "The n-ary / variadic-macro idiom, using the FINAL `,@rest` splice binder: `` `(f ,@rest) `` binds
           the arbitrary-length operand list of an `f`-headed form, and a helper `sum-args` recursively folds
           it. Over `(quote (f 10 20 30))` the rest-binder captures the three `Ast.Int` operands and their
           sum is 60. Pins the tail-splice-binder companion of the fixed-arity recursive-eval case above — a
           macro/interpreter destructuring a form of UNKNOWN arity by binding the rest, then walking it. (The
           `,@rest` pattern binder over a runtime Ast is the capability v-patterns' note flagged as landed.)")
  (input
    (do
      (def
        (sum-args (: xs (List Ast)))
        (match xs (#list() 0N) (#list(h (.. t)) (+ (match h ((Ast.Int n) n) (_ 0N)) (sum-args t)))))
      (def
        (sum-form (: a Ast))
        (match a ((quasiquote (f (unquote-splicing rest))) (sum-args rest)) (_ -1N)))
      (def (main) (sum-form (quote (f 10 20 30))))
      (export main)))
  (output (: 60 BigInt))
  (live-objects known-leak))

(case
  "a recursive Ast walk via a List.fold closure over the sub-trees DECLINES cleanly (no compile overflow)"
  (doc
    "The DECLINE-GUARD companion of the two working walks above. The idiomatic fold shape — a recursive
           `count` re-entered inside a `List.fold` closure whose element is an `Ast` sub-tree bound from an
           `Ast.List` match — once HUNG the compiler (unbounded monomorphization of the recursive closure over
           the recursive `Ast` type; breaker-found). It is now bounded: the compile TERMINATES and DECLINES
           cleanly rather than hanging or overflowing its stack (a compiler must never overflow — the
           inter-procedural follow-depth guard, self-hosting-and-bootstrap.md). This pins the NON-REGRESSION —
           the shape may not yet compile to a value (the generic recursive-closure-over-recursive-sum driver is
           a later increment, v-inference's lane), but it must never again HANG. The explicit-recursion and
           tail-splice-rest walks above are the working companions; when the driver lands this can flip to a
           computed `output`.")
  (input
    (do
      (def
        (count (: node Ast))
        (match node ((Ast.List es) (List.fold es 1 (fn (acc e) (+ acc (count e))))) (_ 1)))
      (def (main) (count (quote (f 1))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

; --- eval drives CONTROL FLOW reified from quoted source (the Ast.Bool integration faces) ---------
; The Ast.Bool cases above pin the leaf (quote/match/eval/encode/print of a bare boolean); these pin
; the boolean leaf DOING ITS JOB inside evaluated control flow — an `if` whose condition arrives
; through the AST, both as a quoted literal and as a quoted comparison the evaluator must first
; reduce. A leaf realization that round-trips standalone but mis-tags inside a List ast (or an eval
; that reads the payload byte incorrectly) picks the wrong branch here.
(case
  "eval of a quoted conditional takes the branch its boolean literal selects"
  (doc
    "`(eval (quote (if false 10 20)))` = 20: the quoted `if` reifies as a List ast whose condition
           element is an `Ast.Bool false` leaf; eval reconstructs the conditional and the false condition
           selects the else branch. Pins the Bool leaf composing INSIDE an evaluated compound — the
           branch-selection companion of the standalone `eval (quote true)` case above (an eval that
           mis-read the payload byte, or a quote that mis-tagged the leaf inside a List ast, answers 10).")
  (input (do (def (main (: d Int64)) (eval (quote (if false 10 20)))) (export main)))
  (call main (: 0 Int64))
  (output (: 20 Int64)))

(case
  "eval of a quoted conditional reduces its comparison condition first"
  (doc
    "`(eval (quote (if (= 1 1) 7 8)))` = 7: the quoted condition is not a Bool LEAF but a
           comparison FORM the evaluator must reduce to a boolean before branching — the produced
           boolean exists only inside eval (no Ast.Bool node in the input tree; `(= 1 1)` reifies as a
           List ast). Pins that eval's boolean values and its branch dispatch agree end-to-end, not only
           when the boolean was quoted literally.")
  (input (do (def (main (: d Int64)) (eval (quote (if (= 1 1) 7 8)))) (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

; eval folds the ORDERING comparisons and the boolean CONNECTIVES too, not only `=`: a quoted `(< 3 5)`
; reduces to a boolean the branch consumes, and a quoted `(and true false)` short-circuits. The `=`-condition
; case above pins equality; these pin that eval reconstructs + reduces the OTHER boolean-producing operator
; families (an ordering relop, a logical connective), so an evaluator that handled `=` but mis-reduced `<` or
; `and` would flip them.
(case
  "eval of a quoted ordering comparison drives the branch"
  (doc
    "`(eval (quote (if (< 3 5) 1 0)))` = 1: the quoted condition `(< 3 5)` is an ORDERING relop (not
           `=`); eval reduces it to `true` and the branch selects 1. Pins that eval folds ordering
           comparisons, the relop companion of the `=`-condition case above.")
  (input (eval (quote (if (< 3 5) 1 0))))
  (output (: 1 Int64)))

; The comparison cases above embed the relop as an `if` CONDITION (its boolean drives a branch); these pin
; that eval of a BARE comparison yields the boolean VALUE directly — the relop is a first-class boolean-
; producing form, not only a branch selector. `=` (equality) → true and `<` (ordering) → false give a
; discriminating pair (both relop families, both truth values). A folder that only reduced a comparison in
; condition position would pass the branch cases yet leave a bare `(= 3 3)` unreduced here.
(case
  "eval of a quoted equality yields the boolean value directly"
  (doc
    "`(eval (quote (= 3 3)))` = true: eval reduces a BARE equality to the boolean value, not only
           when it heads an `if` condition. The direct-value companion of the `=`-drives-a-branch case.")
  (input (eval (quote (= 3 3))))
  (output (: true Bool)))

(case
  "eval of a quoted ordering comparison yields the boolean value directly"
  (doc
    "`(eval (quote (< 5 2)))` = false: eval reduces a BARE ordering relop to its boolean value (here
           FALSE — 5 is not < 2). The ordering + false-valued companion of the equality case above; together
           they pin both relop families and both truth values as first-class eval results.")
  (input (eval (quote (< 5 2))))
  (output (: false Bool)))

(case
  "eval of a quoted boolean connective short-circuits"
  (doc
    "`(eval (quote (and true false)))` = false: the quoted `and` connective reduces over its operands.
           Pins that eval reconstructs + folds a logical connective (distinct from a comparison or arithmetic
           form) — the boolean-algebra companion of the ordering/equality cases.")
  (input (eval (quote (and true false))))
  (output (: false Bool)))

; The `and` case above pins one connective; these complete the boolean-form family. `or` is the
; disjunction complement of `and` (short-circuits to true on a true operand), and `not` is a structurally
; DISTINCT unary form (a single operand, its own reader/resolve arm — `resolve_not`, not the binary
; connective path). Together with the `and` case they pin that eval reconstructs + folds every boolean
; operator, not just conjunction.
(case
  "eval of a quoted disjunction short-circuits to true"
  (doc
    "`(eval (quote (or false true)))` = true: the quoted `or` connective reduces over its operands,
           yielding true once a true operand is reached. The disjunction complement of the `and`-connective
           case above (which folds to false).")
  (input (eval (quote (or false true))))
  (output (: true Bool)))

(case
  "eval of a quoted not inverts its boolean operand"
  (doc
    "`(eval (quote (not false)))` = true: eval reconstructs + folds the UNARY `not` (a single-operand
           form with its own resolve arm, distinct from the binary `and`/`or` connectives) and inverts its
           operand. Completes the boolean-operator eval family (conjunction / disjunction / negation).")
  (input (eval (quote (not false))))
  (output (: true Bool)))

; eval reconstructs + folds every CORE CONTROL FORM, not only arithmetic and `if`: a quoted `let` (a binder-
; introducing form), a quoted `match` (a scrutinee + arms), and a quoted lambda APPLICATION all reconstruct
; to the source they denote and fold through the ordinary compile-time path (`metaprogramming.md` §Eval Is
; Optional / §Compile-Time Evaluation Is One Tier). The existing eval cases exercise `(+ …)` and `if`; these
; pin that `reconstruct` (recursively rebuilding an `Ast.List` as `(<recon e>…)`) preserves the meaning of a
; binding form, a match, and an applied `fn` — so a reconstruction that mishandled one of these control
; heads (e.g. dropped a `let` binder or a match arm) would flip them.
(case
  "eval of a quoted let-binding form folds"
  (doc
    "`(eval (quote (let ((x 4)) (+ x 6))))` = 10: the quoted `let` reconstructs to the binding form it
           denotes, whose `x` binds 4 and body folds to 10. Pins that eval preserves a BINDER-introducing
           control form (the `let`'s bound name resolves in the reconstructed body), not just flat
           arithmetic.")
  (input (eval (quote (let ((x 4)) (+ x 6)))))
  (output (: 10 Int64)))

(case
  "eval of a quoted match form folds to the selected arm"
  (doc
    "`(eval (quote (match 7 (0 100) (n n))))` = 7: the quoted `match` reconstructs to the match it
           denotes; the scrutinee 7 misses the `0` arm and binds the catch-all `n`, so the arm returns 7.
           Pins that eval reconstructs a MATCH (scrutinee + arms + a binding pattern), not only expression
           forms.")
  (input (eval (quote (match 7 (0 100) (n n)))))
  (output (: 7 Int64)))

(case
  "an evaluated quasiquote builds a HEAP value the surrounding code consumes"
  (doc
    "The runtime-splice-to-heap face of eval: the quasiquote's holes splice a RUNTIME k, eval
           reconstructs + compiles the (list …) construction, and the surrounding match consumes the
           resulting HEAP list as an ordinary value (len 2, element 1 = 2k). The eval-of-lambda pin
           beside this is quote-const; this pins the spliced template flowing into the value heap.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (eval (quasiquote #list((unquote k) (* (unquote k) 2))))
          (xs (+ (List.len xs) (* 10 (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 102 Int64)))

(case
  "eval of a quoted lambda application folds"
  (doc
    "`(eval (quote ((fn (x) (* x x)) 5)))` = 25: the quoted form is a lambda APPLIED to 5; eval
           reconstructs the `fn` and its application, β-reduces `x`↦5, and folds `(* 5 5)` to 25. Pins that
           eval reconstructs and folds an applied anonymous function (the `fn` head + its param + the
           argument), the higher-order control-form companion of the `let`/`match` cases.")
  (input (eval (quote ((fn (x) (* x x)) 5))))
  (output (: 25 Int64)))

; The eval-fold family above covers CONTROL forms (if/let/match/lambda); these cover DATA-CONSTRUCTION
; forms — a quoted `(list …)` / `(tuple …)` eval'd folds to the runtime COLLECTION it builds, not just a
; scalar. Pins that eval reconstructs a collection constructor (the `list`/`tuple` head + its element
; forms) and produces a first-class runtime value observable by `List.len` / a tuple pattern — the
; companion of the arithmetic/control eval cases for the compound-VALUE construction path.
(case
  "eval of a quoted list-construction form folds to the runtime list"
  (doc
    "`(eval (quote (list 1 2 3)))` reconstructs the `list` constructor form and folds it to the
           runtime three-element list, so `List.len` reads 3. Pins that eval handles a COLLECTION
           construction form (not only scalars/control) — the reconstructed `(list …)` produces a
           first-class runtime list, the data-construction companion of the arithmetic/control cases.")
  (input (List.len (eval (quote #list(1 2 3)))))
  (output (: 3 Int64)))

(case
  "eval of a quoted tuple-construction form folds to the runtime tuple"
  (doc
    "`(eval (quote (tuple 7 5)))` reconstructs the `tuple` constructor and folds it to the runtime
           2-tuple, destructured by `(tuple a b)` to `7 + 5 = 12`. Pins that eval builds a runtime tuple
           from a quoted tuple form — the fixed-arity-product companion of the list-construction case.")
  (input (match (eval (quote #tuple(7 5))) (#tuple(a b) (+ a b))))
  (output (: 12 Int64)))

; The construction cases above build a collection; these eval a quoted OPERATION that CONSUMES one — a
; String accessor and a List indexing op. eval reconstructs the whole applied form (the op head + its
; collection/string argument + the index) and folds it through the ordinary compile-time path, producing
; the operation's result (a scalar / an Option), not just a rebuilt collection. The consuming-operation
; companion of the collection-construction cases.
(case
  "eval of a quoted String.byte-len folds the string operation"
  (doc
    "`(eval (quote (String.byte-len \"hello\")))` = 5: eval reconstructs the `String.byte-len`
           application over the string literal and folds it. Pins that eval handles a quoted OPERATION
           over a string (not only constructing/executing a bare string), the string-accessor companion
           of the collection cases.")
  (input (eval (quote (String.byte-len "hello"))))
  (output (: 5 Int64)))

(case
  "eval of a quoted List.at folds the indexing operation to its Option result"
  (doc
    "`(eval (quote (List.at (list 10 20 30) 1)))` reconstructs the `List.at` indexing over a
           quoted list-construction and folds it to `(Option.Some 20)` — element 1. Pins that eval folds
           a quoted operation whose result is an Option (a bounds-checked accessor), reconstructing both
           the operation and its collection argument. The consuming-operation companion of the
           list-construction case.")
  (input (match (eval (quote (List.at #list(10 20 30) 1))) ((Option.Some v) v) (_ 0)))
  (output (: 20 Int64)))

(case
  "a constructed Ast.Bool leaf drives an evaluated conditional"
  (doc
    "`(if (eval (Ast.Bool true)) 5 6)` = 5: the Bool leaf is CONSTRUCTED (not quoted), evaluated
           to its payload, and the resulting runtime boolean drives an ORDINARY (non-reified) `if`.
           Closes the loop the constructor case above opens: a hand-built leaf's eval result is a
           first-class Bool usable in real control flow, not merely printable/encodable.")
  (input (do (def (main (: d Int64)) (if (eval (Ast.Bool true)) 5 6)) (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64)))

(case
  "a quoted form's head reifies as a Name while its string argument is a Str"
  (doc
    "`(quote (f \"s\"))` — ONE form carrying both String-payload leaf variants: the head `f` is an
           identifier reference (Ast.Name) and the argument a string literal (Ast.Str), same payload
           TYPE, different variants. The nested element match takes the Name arm for the head (→ 2) and
           a Str head-pattern does not fire (→ not 1). Pins the reifier keys the variant on the
           SYNTACTIC role, not the payload type — a quote that tags every string-payload leaf uniformly
           collapses call-heads and literals, and eval would then look up string literals as names.")
  (input
    (do
      (def
        (main (: d Int64))
        (match
          (quote (f "s"))
          ((Ast.List #list((Ast.Str _) (.. _))) 1)
          ((Ast.List #list((Ast.Name _) (.. _))) 2)
          (_ 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

; `quote` is HEAD-AGNOSTIC: it reifies a parenthesized form to an `Ast.List` whose head is an `Ast.Name`
; whatever the head SPELLS — a grammar keyword (`if`), a collection constructor (`list`/`tuple`/`record`),
; or an ordinary identifier all reify identically (head → `Ast.Name`, elements → their leaves). `quote`
; produces syntax WITHOUT interpreting it (metaprogramming.md #Quote Produces An AST Value), so it must NOT
; special-case a keyword or constructor head — a quote that did would tag `(if …)`'s head differently from
; `(f …)`'s and leak grammar semantics into the reified structure. Pins that the reifier keys on syntactic
; ROLE (head position → Name), never on the head's MEANING.
(case
  "quote reifies a grammar-keyword head as an ordinary Ast.Name"
  (doc
    "`(quote (if a b c))` reifies to an `Ast.List` whose head is `(Ast.Name \"if\")` — `quote` does
           NOT interpret `if` as a conditional, it is a bare name in head position like any other. Matching
           the head binds the Name and `String.byte-len` is 2. Pins that `quote` is head-agnostic (syntactic,
           not semantic): a keyword head reifies exactly as an ordinary identifier head does, so no grammar
           meaning leaks into the AST value.")
  (input
    (match (quote (if a b c)) ((Ast.List #list((Ast.Name h) (.. _))) (String.byte-len h)) (_ -1)))
  (output (: 2 Int64)))

; The `if` case above pins head-agnostic quote for a CONTROL keyword; these extend it to the BINDER-
; introducing keywords `let` and `fn`, where a compiler that interpreted the head during quote would do
; something worse than mis-branch — it would establish a `let` scope or bind an `fn` parameter, potentially
; capturing/renaming the body's names. Quote is purely syntactic: `let`/`fn` reify as bare `Ast.Name` heads
; and the body (including the binder `x`) is inert structure, no scope entered, no binding performed.
(case
  "quote reifies a let form's head as an ordinary Ast.Name without establishing scope"
  (doc
    "`(quote (let ((x 1)) x))` reifies to an `Ast.List` whose head is `(Ast.Name \"let\")` — quote
           does NOT enter a `let` scope or bind `x`; `let` is a bare name head like any other and the whole
           body stays inert structure. Matching the head binds the Name (= \"let\"). The binder-form
           companion of the `if` head-agnostic case: a quote that interpreted `let` would risk scoping the
           body's names, so this pins that no binding happens.")
  (input (match (quote (let ((x 1)) x)) ((Ast.List #list((Ast.Name h) (.. _))) h) (_ "?")))
  (output (: "let" String)))

(case
  "quote reifies an fn form's head as an ordinary Ast.Name without binding its parameter"
  (doc
    "`(quote (fn (x) x))` reifies to an `Ast.List` headed `(Ast.Name \"fn\")` — quote does NOT bind
           the parameter `x` or build a closure; `fn` is a bare name head and the param list + body are
           inert structure. Matching the head binds the Name (= \"fn\"). The lambda-form companion of the
           `let` case: a quote that interpreted `fn` would bind the parameter, so this pins it does not.")
  (input (match (quote (fn (x) x)) ((Ast.List #list((Ast.Name h) (.. _))) h) (_ "?")))
  (output (: "fn" String)))

(case
  "quote of a nested quote form is inert — the inner quote is not evaluated"
  (doc
    "The self-referential head-agnostic edge: `(quote (quote x))` reifies to `(Ast.List (Ast.Name
           \"quote\") (Ast.Name \"x\"))` — `quote` treats the metaprogramming keyword `quote` as an ORDINARY
           head and does NOT recursively evaluate or special-case the inner quote; it stays inert structure.
           The match confirms head = `\"quote\"` and binds the inner `x` (a bare `Ast.Name`, byte-len 1). Pins
           that a plain quote evaluates NOTHING in its body (core-semantics.md — quote produces the AST
           without evaluating), even when the body is itself quote/metaprogramming syntax.")
  (input
    (match
      (quote (quote x))
      ((Ast.List #list((Ast.Name h) (Ast.Name inner)))
        (if (= h "quote") (String.byte-len inner) -2))
      (_ -1)))
  (output (: 1 Int64)))

; The CONSTRUCTION side of the eval-of-a-control-form cases: quoting a binder-introducing form (`let`) is
; head-agnostic like any compound — it reifies to an `Ast.List` whose head is `(Ast.Name "let")` and whose
; binding group + body are ordinary reified sub-trees — and it round-trips through the byte codec. Pairs with
; `(eval (quote (let …)))` → 10 (the eval side): `quote` builds the control-form AST as inert data, `eval`
; reconstructs and runs it. A reifier that special-cased `let` (or dropped its binding group) would flip this.
(case
  "quote of a let reifies as an Ast.List with a let head and round-trips"
  (doc
    "`(quote (let ((x 1)) x))` reifies to an `Ast.List` headed `(Ast.Name \"let\")` (byte-len 3) — the
           binder-introducing form is inert data, its head a bare name like any other — and encode/decode
           round-trips it to an equal AST. The construction companion of the `eval (quote (let …))` fold:
           `quote` builds a control-form AST without interpreting it, and the codec preserves it whole.")
  (input
    (match
      (Ast.decode (Ast.encode (quote (let ((x 1)) x))))
      ((Ok (Ast.List #list((Ast.Name h) (.. _)))) (String.byte-len h))
      ((Ok _) -2)
      ((Err _) -1)))
  (output (: 3 Int64)))

(case
  "three leaf variants in one quoted form each dispatch their own tag"
  (doc
    "`(quote (\"s\" 5 true))` reifies a list whose three elements are DISTINCT leaf variants —
           Ast.Str, Ast.Int, Ast.Bool — bound by one list pattern and classified by a shared `kind`
           match: 1·100 + 2·10 + 3 = 123. The all-variants integration pin: each leaf realization was
           landed separately (Int/Name first, then Bool, then Str), and this case fails if ANY leaf's
           tag collides with another's inside a compound reification (a mis-tagged element shifts one
           digit of the answer, naming the culprit).")
  (input
    (do
      (def (kind (: a Ast)) (match a ((Ast.Str _) 1) ((Ast.Int _) 2) ((Ast.Bool _) 3) (_ 9)))
      (def
        (main (: d Int64))
        (match
          (quote ("s" 5 true))
          ((Ast.List #list(a b c)) (+ (+ (* 100 (kind a)) (* 10 (kind b))) (kind c)))
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 123 Int64)))

(case
  "a spliced boolean drives an evaluated conditional through the lifted leaf"
  (doc
    "`(eval `(if ,false 10 20))` = 20 — the active unquote lifts `false` to an Ast.Bool inside
           the template, and eval's branch dispatch consumes that lifted leaf. The lift cases above
           pin node identity (unquote == quote); this pins the lifted node WORKING in eval'd control
           flow — a lift that built the right-looking node with a wrong payload byte answers 10.")
  (input (do (def (main (: d Int64)) (eval (quasiquote (if (unquote false) 10 20)))) (export main)))
  (call main (: 0 Int64))
  (output (: 20 Int64)))

(case
  "a spliced string participates in an evaluated equality"
  (doc
    "`(eval `(if (= ,\"x\" \"x\") 7 8))` = 7 — the spliced Ast.Str leaf, reconstructed by eval,
           compares content-equal to the quoted literal it sits beside. The Str companion of the
           spliced-bool eval case: the lift must produce a leaf whose eval'd value round-trips into
           the ordinary string-equality path.")
  (input
    (do (def (main (: d Int64)) (eval (quasiquote (if (= (unquote "x") "x") 7 8)))) (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "mixed bool and int splices evaluate in one template"
  (doc
    "`(eval `(if ,true (+ ,3 1) 0))` = 4 — two active unquotes of DIFFERENT value kinds (a Bool
           and an Int) lift in one template, and eval consumes both: the bool selects the branch, the
           int feeds the arithmetic. Pins the per-kind dispatch (`reify_active`) applying the right
           lift per operand within a single quasiquote, not latching one kind for the template.")
  (input
    (do
      (def (main (: d Int64)) (eval (quasiquote (if (unquote true) (+ (unquote 3) 1) 0))))
      (export main)))
  (call main (: 0 Int64))
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
(case
  "an active unquote lifts a RUNTIME integer operand, not only a constant"
  (doc
    "`(main n) = (eval `(+ ,n 1))` called with n=41 → 42. `n` is a runtime parameter (arrives via
           the `(call)`, so it is NOT compile-time-constant), and the active unquote lifts its live value
           through `ast-lift` — the runtime lift path (`lower_ast_lift`), distinct from the literal/let-
           const cases above which fold away before the runtime lift runs. Pins that the lift is a real
           runtime operation: a reversion to a constant-only reify declines this, and a lift that mis-
           wraps the non-constant Int64 payload computes garbage instead of 42.")
  (input (do (def (main (: n Int64)) (eval (quasiquote (+ (unquote n) 1)))) (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))

(case
  "an active unquote lifts a RUNTIME boolean operand, not only a constant"
  (doc
    "The Bool arm of the runtime lift: `(main b) = (eval `(if ,b 10 20))` called with b=false → 20.
           `b` is a runtime parameter, so the `Ast.Bool` wrap is built on a NON-constant payload and eval's
           branch dispatch consumes the lifted leaf at run time. Companion of the runtime-int case: pins the
           `Bool→Ast.Bool` arm of `lower_ast_lift` over a live operand (a const-only reify declines it; a
           mis-wrapped bool payload selects the wrong branch and answers 10).")
  (input (do (def (main (: b Bool)) (eval (quasiquote (if (unquote b) 10 20)))) (export main)))
  (call main (: false Bool))
  (output (: 20 Int64)))

(case
  "an active unquote lifts a RUNTIME float operand, not only a constant"
  (doc
    "The Float arm of the runtime lift: `(main x) = (eval `(+ ,x 1.5))` called with x=2.5 → 4.0. `x`
           is a runtime Float64 parameter, so the `Ast.Float` wrap carries a NON-constant payload that
           eval reconstructs into ordinary float arithmetic. Companion of the runtime-int case for the
           `Float64→Ast.Float` arm (a payload mis-read as the i64 bit pattern computes garbage, not 4.0).")
  (input (do (def (main (: x Float64)) (eval (quasiquote (+ (unquote x) 1.5)))) (export main)))
  (call main (: 2.5 Float64))
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
(case
  "a user function may be named quote — its signature is a binding form, not a quote"
  (doc
    "Witnesses that `quote`/`quasiquote` are grammar heads in EXPRESSION position only, not
           reserved definition names — like `if`/`match`, a user may `def quote(x) = x + 2`. The
           def signature `(quote x)` MUST NOT be reified by the quote pre-pass (which would erase the
           parameter binder and report the body's `x` as CDZ0101 unbound); it scans as an ordinary
           function named `quote`. Referenced as a first-class value through `apply1`, it computes
           `quote(5) = 7`.")
  (input
    (do
      (def (quote (: x Int64)) (+ x 2))
      (def (apply1 (: f (-> Int64 Int64)) (: n Int64)) (f n))
      (def (main (: d Int64)) (apply1 quote 5))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "eval of a quoted float feeds ordinary float arithmetic"
  (doc
    "`(* (eval (quote 2.5)) 2.0)` = 5.0 — the reconstructed float literal is a first-class
           Float64 in downstream arithmetic (a payload mis-read as the i64 bit pattern computes
           garbage). The arithmetic-consumption companion of the eval-to-value case above.")
  (input (do (def (main (: d Int64)) (* (eval (quote 2.5)) 2.0)) (export main)))
  (call main (: 0 Int64))
  (output (: 5.0 Float64)))

(case
  "all four leaf kinds in one quoted form dispatch their own tags"
  (doc
    "`(quote (\"s\" 5 true 2.5))` — Str, Int, Bool, and Float leaves in ONE reified list, each
           classified by a shared match: 1·1000 + 2·100 + 3·10 + 4 = 1234. The full-leaf-set
           integration pin: any mis-tagged element shifts one digit, naming the culprit.")
  (input
    (do
      (def
        (kind (: a Ast))
        (match a ((Ast.Str _) 1) ((Ast.Int _) 2) ((Ast.Bool _) 3) ((Ast.Float _) 4) (_ 9)))
      (def
        (main (: d Int64))
        (match
          (quote ("s" 5 true 2.5))
          ((Ast.List #list(a b c e))
            (+ (+ (+ (* 1000 (kind a)) (* 100 (kind b))) (* 10 (kind c))) (kind e)))
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1234 Int64)))

; --- Codec bijection over rich trees: the composition faces of encode/decode -----------------------
; The per-leaf round-trips and the adversarial-bytes totality pins grade single nodes; these grade
; the BIJECTION contract (ast-encoding.md — one canonical byte form) over structurally rich trees,
; promoted from passing breaker probes after the non-minimal-varint reject (codec bijection).
(case
  "a deep four-leaf tree round-trips through encode and decode"
  (doc
    "`(quote (f (g 1 true) \"s\" 2.5))` — three nesting levels carrying all four leaf kinds —
           encodes and decodes to an EQUAL tree. The composition face of the per-leaf round-trips:
           length-prefixed lists nest, and each leaf's payload survives inside the compound framing
           (a framing error corrupts everything after the first nested list).")
  (input
    (match
      (Ast.decode (Ast.encode (quote (f (g 1 true) "s" 2.5))))
      ((Ok a) (= a (quote (f (g 1 true) "s" 2.5))))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "quote-built and constructor-built equal trees encode byte-identically"
  (doc
    "`(quote (f 1))` and `(Ast.List (list (Ast.Name \"f\") (Ast.Int 1)))` are ONE value built
           two ways; the bijection contract (one canonical byte form per tree) means their encodings
           are byte-EQUAL, not merely decode-equivalent. A codec with construction-dependent framing
           (or the non-minimal varints just rejected) breaks exactly this equality.")
  (input (= (Ast.encode (quote (f 1))) (Ast.encode (Ast.List #list((Ast.Name "f") (Ast.Int 1))))))
  (output (: true Bool)))

(case
  "one encode-decode cycle is byte-stable"
  (doc
    "encode(decode(encode t)) = encode(t) — the decoded tree re-encodes to the SAME bytes (the
           bijection composed both directions). Catches a decoder that normalizes or a codec pair
           that round-trips values while drifting bytes (legal under decode-equality, illegal under
           the canonical-byte-form contract).")
  (input
    (match
      (Ast.decode (Ast.encode (quote (f (g 1 true) "s" 2.5))))
      ((Ok a) (= (Ast.encode a) (Ast.encode (quote (f (g 1 true) "s" 2.5)))))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "a runtime-assembled tree round-trips equal to its quoted twin"
  (doc
    "`` `(f ,(+ 1 2)) `` — the tree is ASSEMBLED at run time (an active unquote splicing a
           computed 3) — encodes/decodes equal to the statically-quoted `(quote (f 3))`. Pins the
           codec over a runtime-built tree (the constant cases could fold end-to-end; a splice's
           lifted leaf must serialize identically to a quoted one).")
  (input
    (match
      (Ast.decode (Ast.encode (quasiquote (f (unquote (+ 1 2))))))
      ((Ok a) (= a (quote (f 3))))
      ((Err _) false)))
  (output (: true Bool)))

; --- Ast-valued unquote splicing: the operand-binding faces -----------------------------------------
; The ast-lift intrinsic splices a COMPUTED Ast subtree into a quasiquote template (the RESOLVED
; splice gap; its pin covers a param-bound operand matched structurally). These pin the other
; operand bindings and the identity contract, promoted from passing breaker probes.
(case
  "a let-bound Ast splices into a template"
  (doc
    "`(let ((sub (quote (* 2 3)))) `(+ ,sub 1))` — the spliced operand is a LET binding (the
           resolved pin covers a param). The grafted template is a 3-element list. Pins the splice
           over a local binding (an ast-lift keyed to param slots misses the local).")
  (input
    (do
      (def
        (main (: d Int64))
        (let
          ((sub (quote (* 2 3))))
          (match (quasiquote (+ (unquote sub) 1)) ((Ast.List es) (List.len es)) (_ -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

(case
  "a grafted template is structurally equal to the directly-quoted tree"
  (doc
    "`` `(+ ,(quote (* 2 3)) 1) `` = `(quote (+ (* 2 3) 1))` — the identity contract of the
           splice: inserting an Ast RESULT means grafting the node AS-IS, so the assembled tree is
           byte-for-byte the tree the plain quote builds (structural equality over the two). A
           re-wrapping splice (the old Ast.Int(...) coercion) or a copy that perturbs the subtree
           breaks the equality.")
  (input (= (quasiquote (+ (unquote (quote (* 2 3))) 1)) (quote (+ (* 2 3) 1))))
  (output (: true Bool)))

(case
  "a RUNTIME Ast structural equality compares by value (Ast.Int scalar leaf and Ast.List compound)"
  (doc
    "The case above compares two COMPILE-TIME quotes (const-folds). This pins the RUNTIME path: an
           `Ast` value built from a boundary `Int64` parameter `n` (no fold), compared with `=`. `Ast` is a
           user sum whose leaves span a scalar payload (`Ast.Int n`) AND a compound payload (`Ast.List [Ast.Int
           n]`), so the runtime `=` must walk the sum structurally — element-wise, not the physical byte walk
           (an `Ast.List` payload is a `List`, element- but not shape-canonical; an `Ast.Float` leaf is
           non-orderable). Over `n`: `(= (Ast.Int n) (Ast.Int 3))` AND `(= (Ast.List [Ast.Int n]) (Ast.List
           [Ast.Int 3]))`, encoded `10·intEq + listEq`. n=3 → both equal → 11; n=5 → both unequal → 0. wasm
           computes this via the descriptor-guided value-eq-shaped walk; the RUST backend does not yet render
           runtime structural `=` over this compound (a coverage `todo`, not a miscompile — it declines
           cleanly). Regression witness that a runtime Ast `=` (the shape a self-hosted pass comparing syntax
           trees relies on) computes on wasm.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (* 10 (if (= (Ast.Int (BigInt.of n)) (Ast.Int 3)) 1 0))
          (if (= (Ast.List #list((Ast.Int (BigInt.of n)))) (Ast.List #list((Ast.Int 3)))) 1 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 11 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a recursive CONSTANT-FOLD pass rewrites int-add subtrees bottom-up and counts folds"
  (doc
    "A whole COMPILER PASS in miniature over runtime Ast values: a bottom-up recursive rewrite that
           folds `(+ <int> <int>)` subtrees to their sums and THREADS A FOLD COUNT through a
           mutually-recursive list walk (fold / fold-list, tuple-carried accumulator). mode 1 folds two
           nested add-sites under distinct heads (count 2); mode 2 has no foldable site (count 0).
           Historically rust-blocked by the E0382 recursive-fold finding (fixed f4ae338d1) — this is the
           independent breaker witness that exercises the fixed shape through Ast payloads.")
  (input
    (do
      (def
        (fold node)
        (match
          node
          ((Ast.List xs)
            (match
              (fold-list xs #list() 0)
              (#tuple(xs2 k)
                (match
                  xs2
                  (#list((Ast.Name op) (Ast.Int a) (Ast.Int b))
                    (if (= op "+") #tuple((Ast.Int (+ a b)) (+ k 1)) #tuple((Ast.List xs2) k)))
                  (_ #tuple((Ast.List xs2) k))))))
          (other #tuple(other 0))))
      (def
        (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64))
        (match
          xs
          (#list() #tuple(acc k))
          (#list(h (.. t))
            (match (fold h) (#tuple(h2 k2) (fold-list t (List.push acc h2) (+ k k2)))))))
      (def
        (main (: mode Int64))
        (do
          (def
            t
            (if
              (= mode 1)
              (Ast.List
                #list((Ast.Name "f")
                  (Ast.List #list((Ast.Name "+") (Ast.Int 2) (Ast.Int 3)))
                  (Ast.List
                    #list((Ast.Name "g") (Ast.List #list((Ast.Name "+") (Ast.Int 4) (Ast.Int 5)))))))
              (Ast.List #list((Ast.Name "f") (Ast.Int 7)))))
          (match
            (fold t)
            (#tuple(t2 k)
              (+
                (* k 10)
                (if
                  (=
                    t2
                    (if
                      (= mode 1)
                      (Ast.List
                        #list((Ast.Name "f")
                          (Ast.Int 5)
                          (Ast.List #list((Ast.Name "g") (Ast.Int 9)))))
                      (Ast.List #list((Ast.Name "f") (Ast.Int 7)))))
                  1
                  0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "an eval splice consumes a value extracted from a CHAMP map at run time"
  (doc
    "The splice pins feed literals and locals; this operand comes OUT of a Map — `(Map.lookup m k)`
           through Option.expect — before `(unquote v)` splices it into the quoted `(+ _ 5)` and eval
           computes (15 at k=1, 25 at k=2). Pins that the quote/eval machinery accepts a heap-collection-
           EXTRACTED runtime value as a splice operand — a splice path that only recognized
           literal/binder operands (or re-evaluated the lookup inside the quote's phase) misfires.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 10) (= 2 20)))
          (def v (Option.expect (Map.lookup m k) "present"))
          (Int64.of (eval (quasiquote (+ (unquote v) 5))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 15 Int64))
  (call main (: 2 Int64))
  (output (: 25 Int64)))

(case
  "eval materializes a heap LIST from a spliced quasiquote"
  (doc
    "The eval pins return scalars; this eval's result is a COLLECTION — `(list 1 (unquote k) 3)`
           crosses the Ast→value boundary as a heap list the caller measures and indexes (len 3,
           slot 1 = the spliced k: 307 at k=7, 300 at k=0). An eval that boxed the list as an Ast
           node (or materialized only the spliced leaf) fails the len or the projection.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def xs (eval (quasiquote #list(1 (unquote k) 3))))
          (+ (* 100 (List.len xs)) (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 307 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "an eval match binder shadowing a HEAP-typed outer name leaves the heap value intact"
  (doc
    "The hygiene-collision pins cover scalar-over-scalar shadows; here the evaled match binds `m`
           — a SCALAR 5 — while the OUTER `m` is a heap Map. The quote's arm computes with its own m
           (5+2 = 7 → 700) and the outer map must survive the collision un-clobbered (lookup reads k:
           703 at k=3, 700 at k=0). A hygiene table that resolved the collision by SLOT rather than by
           scope would either read the map handle as a scalar in the arm (garbage arithmetic) or write
           the scalar 5 over the map handle (corrupting the later lookup).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 k)))
          (def r (eval (quasiquote (match 5 (m (+ m 2)) (_ 0)))))
          (+ (* 100 (Int64.of r)) (match (Map.lookup m 1) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 703 Int64))
  (call main (: 0 Int64))
  (output (: 700 Int64)))

(case
  "eval does not execute an Ast selected from a collection at run time"
  (doc
    "The COLLECTION entry path to the no-runtime-AST-interpreter line (nested-eval and
           spliced-Ast-operand are pinned above; this reaches it via `List.at`): the quasiquotes
           build fine as Ast VALUES in a list, but `(eval (Option.expect (List.at asts 1) ...))`
           hands eval a runtime SELECTION — not a compile-time-visible `(quote …)`/`Ast.*`
           construction — so it rejects CDZ0101. A coded reject: an eval that silently ran the
           selected node (un-analyzed AST) would flip this to 12 and trip the gate.")
  (input
    (do
      (def
        (main)
        (do
          (def asts #list((quasiquote (+ 1 2)) (quasiquote (* 3 4))))
          (Int64.of (eval (Option.expect (List.at asts 1) "present")))))
      (export main)))
  (error CDZ0101))

(case
  "eval of a NAME bound to a quote is rejected — the binding hides the construction"
  (doc
    "The BINDING entry path to the no-runtime-AST-interpreter line (nested-eval, spliced-Ast
           and collection-selection are pinned above): `(def adder (quote (+ 20 22)))` then
           `(eval adder)` — the eval's argument is a NAME REFERENCE, not the compile-time-visible
           `(quote …)` construction itself, so the reconstructor refuses (CDZ0101) even though the
           binding is initialized by a quote in the SAME unit. The same holds for an IMPORTED
           Ast binding (checked while building this pin). Fourth entry path, same spec line: eval
           sees through NO indirection — not another eval, not a splice, not a collection slot,
           not a let/def binding. A future eval that chased the binding to its quote would flip
           this to 42 and needs a deliberate ruling first.")
  (input
    (do
      (def adder (quote (+ 20 22)))
      (def (main (: k Int64)) (+ (Int64.of (eval adder)) k))
      (export main)))
  (error CDZ0101))

; The read primitive SKIPS ; line-comments in program text (concierge-ruled (a); self-hosting-surface
; :63 'a reader converts the text of a PROGRAM' + front-end-reader consistency + round-trip). Was a
; silent mis-parse (tokenized ; as a Name → wrong AST). Fixed by v-metaprogramming ee58991cb
; (skip_ws skips ;-to-EOL; ;-in-string preserved). breaker-routed.
(case
  "the read primitive skips a line comment inside program text"
  (input
    (do
      (def
        (main (: mode Int64))
        (+
          (* 10 (if (= (Ast.read "(+ 1 ; a comment
 2)") (quote (+ 1 2))) 1 0))
          (if (= (Ast.read "; leading
(f 3)") (quote (f 3))) 1 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

; A malformed read (unterminated / trailing content) is a CODED REJECT, not a not-yet-reducible
; decline — malformedness is a PERMANENT fact about the input (concierge rider ruling; the reject-not-
; decline discipline for permanent facts, 27-des:5120 class reversed). Fixed by v-metaprogramming
; ee58991cb: malformed read → CDZ0201 'not a well-formed s-expression'. breaker-routed.
(case
  "a malformed read is a coded reject, not a not-yet-reducible decline"
  (input (do (def (main) (if (= (Ast.read "(+ 1") (quote (+ 1 2))) 1 0)) (export main)))
  (error CDZ0201))

; A runtime-selected Ast payload inside a rebuilt Ast, compared (=) against a const-read result, MUST
; build + compute. Regression: reify_read_ast typed the Ast.Int payload Ty::int64() while the payload
; is boxed-BigInt post-flip, so a read-Ast carried a raw-i64 rep vs the declared boxed-BigInt — the =
; composition (rebuilt-Ast vs const-read) forced the equality lowering to reconcile mismatched reps →
; wasm invalid-module + rust E0308. Fixed by v-metaprogramming 191e65164 (retype to Ty::BigInt,
; matching decode_ast_value). breaker-routed (#36). (The read-Int-as-map-key consumer face is kept
; HELD separately — still todo on rust.)
(case
  "a runtime-selected Name payload inside a rebuilt Ast compares against a read result"
  (input
    (do
      (def
        (main (: mode Int64))
        (match
          (Ast.read "(defn add 1)")
          ((Ast.List parts)
            (match
              parts
              (#list((Ast.Name _kw) rest (.. more))
                (if
                  (=
                    (Ast.List
                      (List.prepend
                        (List.prepend more rest)
                        (Ast.Name (if (= mode 1) "defx" "defy"))))
                    (Ast.read "(defx add 1)"))
                  1
                  0))
              (_ -2)))
          (_ -3)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64)))

(case
  "eval does not execute the result of read — even over a constant string"
  (doc
    "The FIFTH entry path to the no-runtime-AST-interpreter line (nested-eval, spliced-Ast,
           collection-selection, and bound-name are pinned): `(eval (Ast.read \"(+ 20 22)\"))` rejects
           CDZ0101 even though the string is CONSTANT and `read` itself folds (print∘read and
           read-equality both compute in the pins above). eval's visible set is exactly `(quote …)`
           / literal `Ast.*` — a READ application is not in it, so text does not become executable
           by way of read. The gate this pins: read-then-eval is the classic injection shape
           (text → AST → execute); making it work would need a deliberate ruling, not a fold
           side-effect.")
  (input (do (def (main (: k Int64)) (+ (Int64.of (eval (Ast.read "(+ 20 22)"))) k)) (export main)))
  (error CDZ0101))

(case
  "a read result destructures through nested Ast patterns — the analyze path computes"
  (doc
    "The WORKING consumption of read (its eval is the pinned reject; MATCH is the legitimate
           tooling path): the parsed `(defn add (a b) (+ a b))` destructures through two levels of
           Ast.List patterns — keyword name compared as a STRING (`Ast.Name` payloads are String),
           the param list and body list measured (2/3), all in one arm (231). text → AST → ANALYZE
           is what a linter/codemod does; a reify that boxed nested lists opaquely (or a Name
           payload that compared by identity instead of content) breaks the arm and falls to -2.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (Ast.read "(defn add (a b) (+ a b))")
          ((Ast.List parts)
            (match
              parts
              (#list((Ast.Name kw) (Ast.Name fname) (Ast.List params) (Ast.List body))
                (+ (* 100 (List.len params)) (+ (* 10 (List.len body)) (if (= kw "defn") 1 0))))
              (_ -2)))
          (_ -3)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 231 Int64)))

(case
  "quote-built and guest-built Asts compare across const/runtime payload boundaries"
  (doc
    "The WORKING side of the read-reify rep finding: a `(quote (f 42))` Ast and a fully
           guest-built `(Ast.List ...)` with a RUNTIME `(Ast.Int (BigInt.of k))` payload compare
           correctly (hit at k=42, miss at 7 — 10/00 encoded as 2 rows), and the same holds for
           guest-built vs guest-built. Pins that quote's reify and Ast.* construction share ONE rep
           the mixed-payload equality bridges — the READ-reify divergence is the filed bug; this
           guards the reps that already agree from a fix that unified in the wrong direction.")
  (input
    (do
      (def
        (main (: k Int64))
        (+
          (*
            10
            (if (= (quote (f 42)) (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of k))))) 1 0))
          (if
            (=
              (Ast.List #list((Ast.Name "g") (Ast.Int (BigInt.of k))))
              (Ast.List #list((Ast.Name "g") (Ast.Int 42))))
            1
            0)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 11 Int64))
  (call main (: 7 Int64))
  (output (: 0 Int64)))

(case
  "Ast values key a map across quote and Ast.* construction routes"
  (doc
    "The rep-agreement matrix's WORKING half on the CHAMP surface (read-built keys are the
           filed invalid-module face): an `Ast.List`-BUILT key is found by its built twin (mode 1)
           AND by the `(quote (f 1))` spelling (mode 2) — both 42, so quote's reify and Ast.*
           construction share one hashable rep on the key path exactly as they do under `=`.
           Together with the eval-splice pins this fixes the WHOLE quote/Ast.* rep contract;
           the read-reify fix must join THIS rep (a miss here post-fix means it unified the
           wrong way).")
  (input
    (do
      (def
        (main (: mode Int64))
        (match
          (Map.lookup
            (Map.insert Map.empty (Ast.List #list((Ast.Name "f") (Ast.Int 1))) 42)
            (if (= mode 1) (Ast.List #list((Ast.Name "f") (Ast.Int 1))) (quote (f 1))))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 2 Int64))
  (output (: 42 Int64)))

; --- Splice POSITION faces: empty-mid gap-close, singleton fill, double-splice index shift. ---
(case
  "an EMPTY splice mid-list closes the gap, a singleton fills it, and two splices around a literal both land"
  (doc
    "The POSITION faces of ,@ (the existing splice pins are tail-position, non-empty): an EMPTY splice MID-list must close the gap (`(f 1 ,@() 2)` = `(f 1 2)` — an off-by-one keeps a hole or eats the 2); a singleton fills the same slot; and TWO splices around a literal name (`(f ,@xs mid ,@ys)`) exercise the index bookkeeping after the first splice shifts positions. Structural = against directly-written quasiquotes.")
  (input
    (do
      (def
        (main)
        (+
          (*
            100
            (if
              (=
                (let ((xs #list())) (quasiquote (f 1 (unquote-splicing xs) 2)))
                (quasiquote (f 1 2)))
              1
              0))
          (+
            (*
              10
              (if
                (=
                  (let ((xs #list(7))) (quasiquote (f 1 (unquote-splicing xs) 2)))
                  (quasiquote (f 1 7 2)))
                1
                0))
            (if
              (=
                (let
                  ((xs #list(7)) (ys #list(8 9)))
                  (quasiquote (f (unquote-splicing xs) mid (unquote-splicing ys))))
                (quasiquote (f 7 mid 8 9)))
              1
              0))))
      (export main)))
  (output (: 111 Int64)))

; --- Runtime-selected splices in a non-commutative form. ---
(case
  "two runtime-selected splices keep their operand ORDER in a non-commutative form"
  (doc
    "Splice-slot discipline under a NON-commutative operator: two unquotes whose values are
           runtime-SELECTED (a/b swap by k) splice into `(- ,a ,b)` — the subtraction's sign proves
           which value landed in which slot (-10 at k=1, +10 otherwise). A splice numbering that
           bound operands by evaluation order rather than POSITION (or cached one unquote's
           reconstruction for both) flips or zeroes the sign. The two-splice ordered face of the
           computed-unquote pins.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def a (if (= k 1) 10 20))
          (def b (if (= k 1) 20 10))
          (Int64.of (eval (quasiquote (- (unquote a) (unquote b)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: -10 Int64))
  (call main (: 2 Int64))
  (output (: 10 Int64)))

(case
  "a runtime-woven Ast and its quote twin resolve to ONE Map key"
  (doc
    "The KEY face of cross-construction-path Ast identity (structural EQ is pinned above at the
           runtime-woven-vs-quote case; the Set/Map pins above use quote-built keys on BOTH sides): a Map
           keyed by the reader-built `(quote (+ 5 2))` is probed by a CONSTRUCTOR-woven tree whose Int leaf
           is a runtime BigInt (`(Ast.Int (BigInt.of a))` — forcing the live deep hash/eq walk, nothing
           folds). a=5 → the woven tree is content-identical → 42; a=6 → one leaf differs → -1. A CHAMP
           hash computed over construction PROVENANCE (or a quote interned to a distinct identity) would
           miss the a=5 lookup.")
  (input
    (do
      (def
        (main (: a Int64))
        (match
          (Map.lookup
            (Map.insert Map.empty (quote (+ 5 2)) 42)
            (Ast.List #list((Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2))))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64))
  (call main (: 6 Int64))
  (output (: -1 Int64)))

(case
  "a template EXPANSION is a CHAMP Map key found by its directly-woven twin"
  (doc
    "The template face of cross-construction-path Ast key identity (the quote-vs-weave face is
           pinned above; this is EXPANSION-vs-weave): a Map keyed by the directly-woven
           `(+ <a> 2)` tree is hit by a TAG EXPANSION producing the same tree from a hole — the
           expansion output must hash/compare as an ordinary Ast value with no expansion-provenance
           residue. A tag-output interning (or a wrapper node left on the expansion) would miss the
           lookup.")
  (input
    (do
      (def
        (mk chunks holes)
        (match holes (#list(h) (Ast.List #list((Ast.Name "+") h (Ast.Int 2)))) (_other (Ast.Int 0))))
      (def
        (main (: a Int64))
        (match
          (Map.lookup
            (Map.insert
              Map.empty
              (Ast.List #list((Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2)))
              42)
            (tagged-template mk (chunks "" "") (holes (Ast.Int (BigInt.of a)))))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64)))

; --- Ast.Char / Ast.Symbol — the syntax-leaf variants that make quote/reflection TOTAL ---------------
; The built-in Ast sum carries a variant for EVERY syntax leaf, including `#\a` char and `#"x"` symbol
; literals (operator directive: reflection/quote must never decline on a well-formed leaf). A quote of a
; char/symbol reifies to `Ast.Char`/`Ast.Symbol`, and both round-trip through the canonical codec.
(case
  "a quote of a char literal reifies to an Ast.Char value"
  (doc "\a")
  (input (= (quote #\a) (Ast.Char #\a)))
  (output (: true Bool)))

(case
  "an Ast.Char node round-trips through encode and decode"
  (doc
    "`Ast.encode`/`Ast.decode` are a bijection over the char leaf (codec `KIND_CHAR`): encode the
           char node to canonical bytes, decode back to an EQUAL tree. `Ast.decode` is total.")
  (input
    (match (Ast.decode (Ast.encode (Ast.Char #\z))) ((Ok a) (= a (Ast.Char #\z))) ((Err _) false)))
  (output (: true Bool)))

(case
  "an Ast.Symbol node round-trips through encode and decode"
  (doc
    "`Ast.encode`/`Ast.decode` are a bijection over the symbol leaf (codec `KIND_SYM`): encode the
           symbol node to canonical bytes, decode back to an EQUAL tree.")
  (input
    (match
      (Ast.decode (Ast.encode (Ast.Symbol #"add")))
      ((Ok a) (= a (Ast.Symbol #"add")))
      ((Err _) false)))
  (output (: true Bool)))

(case
  "Ast.module reflects the enclosing module as an Ast.List value"
  (doc
    "`Ast.module` is the self-reflection intrinsic (a member of the built-in `Ast` sum): it reflects the
           AST of the module the occurrence is in, type-directed through ordinary resolution. A module body
           is a `(do …)`, which reflects to an `Ast.List`, so matching `Ast.module` against the `Ast.List`
           variant is true. More general than importing one's own `__ast__` — any code can reflect its
           containing module without naming its own path.")
  (input (do (def (main) (match Ast.module ((Ast.List _) true) (_ false))) (export main)))
  (output (: true Bool)))

; -- runtime Ast.print: a RUNTIME AST value renders to canonical s-expr text — scalar + NESTED list (breaker batch 393; the #3560/#3621/#3627 op-92 arc acceptance pair) --
(case
  "cj03r Ast.print of a RUNTIME AST value renders"
  (input
    (do
      (def (main (: k Int64)) (String.byte-len (Ast.print (Ast.Int (BigInt.of k)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64)))

(case
  "cj03n Ast.print of a runtime NESTED Ast renders the canonical s-expr text"
  (input
    (do
      (def
        (main (: k Int64))
        (String.byte-len (Ast.print (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of k)))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 5 Int64)))

(case
  "a runtime BigInt wrapped in Ast.Int round-trips and reclaims the boxed payload on drop (no live objects)"
  (doc
    "`(+ (BigInt.of k) 1N)` is a RUNTIME BigInt (entry param k drives it, so it heap-allocs a Big and
           does NOT const-fold), wrapped in `(Ast.Int ...)`, matched, and narrowed back with `Int64.of`.
           main(41) = (41+1) narrowed = 42. Dropping the Ast.Int must cascade the reclaim into the boxed
           BigInt payload -- net 0 live cells (a leaked payload = the sum drop not cascading).")
  (input
    (do
      (def
        (main (: k Int64))
        (let ((x (+ (BigInt.of k) 1N))) (match (Ast.Int x) ((Ast.Int n) (Int64.of n)) (_ -1))))
      (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64))
  (live-objects 0))

; -- runtime Ast.encode (op 93, the #3653 emit): encode-alone runs, the const twin folds, and RUNTIME == CONST-FOLDED length (the consistency witness) (breaker batch 398; decode-side emit pending for the round-trip pair) --
(case
  "ce93s Ast.encode ALONE over a runtime AST yields canonical bytes"
  (input
    (do (def (main (: k Int64)) (Bytes.len (Ast.encode (Ast.Int (BigInt.of k))))) (export main)))
  (call main (: 7 Int64))
  (output (: 16 Int64)))

(case
  "ce93c CONST twin: Ast.encode of the same constant AST folds to the same length"
  (input (Bytes.len (Ast.encode (Ast.Int (BigInt.of 7)))))
  (output (: 16 Int64)))

(case
  "ce93s2 runtime encode length EQUALS the const-folded length (consistency witness)"
  (input
    (do
      (def
        (main (: k Int64))
        (if
          (=
            (Bytes.len (Ast.encode (Ast.Int (BigInt.of k))))
            (Bytes.len (Ast.encode (Ast.Int (BigInt.of 7)))))
          1
          0))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64)))

; -- breaker batch 406 (2026-08-26): runtime NON-FINITE Ast.Float encode faces (#3711 op93 tags
; 17/18/19, same-hour probe). A runtime-computed NaN/inf payload now ENCODES: nfe1 non-empty bytes,
; nfe2 sibling-tag equal lengths, nfe3 +inf/-inf encodes differ (distinct tags, via the tuple-walk
; compare), nfe4 NaN encodes byte-identical (canonical form). wasm pass / rust todo (rust op93 body
; not yet updated). Compile-fold flip (encode_ast_value) still pending with v-cp; bare runtime Bytes
; equality over encode results is a separate filed decline (flat-operands-only gate).
(case
  "nfe1 runtime +inf Ast.Float encodes to non-empty bytes (op93 non-finite tag path)"
  (input
    (do
      (def (f (: x Float64)) (if (> (Bytes.len (Ast.encode (Ast.Float (/ x 0.0)))) 0) 1 0))
      (export f)))
  (call f (: 1.0 Float64))
  (output (: 1 Int64)))

(case
  "nfe2 +inf and -inf runtime encodes have EQUAL byte length (sibling tags 18/19)"
  (input
    (do
      (def
        (f (: x Float64))
        (if
          (=
            (Bytes.len (Ast.encode (Ast.Float (/ x 0.0))))
            (Bytes.len (Ast.encode (Ast.Float (/ (- 0.0 x) 0.0)))))
          1
          0))
      (export f)))
  (call f (: 1.0 Float64))
  (output (: 1 Int64)))

(case
  "nfe3 tuple-walk equality says +inf and -inf encodes DIFFER (distinct tags)"
  (input
    (do
      (def
        (f (: x Float64))
        (if
          (=
            #tuple(1 (Ast.encode (Ast.Float (/ x 0.0))))
            #tuple(1 (Ast.encode (Ast.Float (/ (- 0.0 x) 0.0)))))
          0
          1))
      (export f)))
  (call f (: 1.0 Float64))
  (output (: 1 Int64)))

(case
  "nfe4 two runtime NaN encodes are byte-identical via the tuple walk (canonical NaN form)"
  (input
    (do
      (def
        (f (: x Float64))
        (if
          (=
            #tuple(1 (Ast.encode (Ast.Float (- (/ x 0.0) (/ x 0.0)))))
            #tuple(1 (Ast.encode (Ast.Float (- (/ (* x 2.0) 0.0) (/ (* x 2.0) 0.0))))))
          1
          0))
      (export f)))
  (call f (: 1.0 Float64))
  (output (: 1 Int64)))

; -- breaker batch 414 (2026-08-26): Ast.print of a RUNTIME AST value renders (base face; the
; nested sibling cj03n and the render-text face cj03r pinned earlier). wasm pass / rust todo.
(case
  "cj03 Ast.print of a RUNTIME AST value renders"
  (input
    (do
      (def (main (: k Int64)) (String.byte-len (Ast.print (Ast.Int (BigInt.of k)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64)))

; -- breaker batch 429 (2026-08-26): Ast.print RENDERS of the runtime non-finite Float leaves
; (#3711's tags reach print): +inf -> "inf.0", NaN -> "NaN.0", -inf inside a list -> "-inf.0" —
; the float-leaf '.' suffix convention applied to non-finites. Deterministic and value-distinct.
; NOTE: these renders are not re-readable source text (no non-finite literal exists) — the
; print/read round-trip question is filed with the non-finite surface ruling (v-inference thread).
; wasm pass / rust todo (runtime print path pending on rust).
(case
  "nfp1 Ast.print renders a runtime +inf Ast.Float as inf.0"
  (input (do (def (main (: x Float64)) (Ast.print (Ast.Float (/ x 0.0)))) (export main)))
  (call main (: 1.0 Float64))
  (output (: "inf.0" String))
  (live-objects known-leak))

(case
  "nfp2 Ast.print renders a runtime NaN Ast.Float as NaN.0"
  (input
    (do (def (main (: x Float64)) (Ast.print (Ast.Float (- (/ x 0.0) (/ x 0.0))))) (export main)))
  (call main (: 1.0 Float64))
  (output (: "NaN.0" String))
  (live-objects known-leak))

(case
  "nfp3 Ast.print renders a -inf leaf inside a list as -inf.0"
  (input
    (do
      (def
        (main (: x Float64))
        (Ast.print (Ast.List #list((Ast.Name "f") (Ast.Float (/ (- 0.0 x) 0.0))))))
      (export main)))
  (call main (: 1.0 Float64))
  (output (: "(f -inf.0)" String))
  (live-objects known-leak))

; -- a quasiquote PATTERN dispatches on the head symbol of a RUNTIME Ast (built via Ast.List/Ast.Name so it
; is not a constant); migration from rcdzc a_runtime_string_pattern_dispatches_by_content, 2026-08-27. The
; head-symbol match is a runtime String content compare, so it exercises the runtime string-pattern path.
(case
  "a quasiquote pattern dispatches on the head symbol of a runtime Ast"
  (input
    (do
      (def
        (op (: a Ast))
        (match
          a
          ((quasiquote (+ (unquote x) (unquote y))) 100)
          ((quasiquote (* (unquote x) (unquote y))) 200)
          (_ 0)))
      (def (main) (op (Ast.List #list((Ast.Name (String.concat "+" "")) (Ast.Int 1) (Ast.Int 2)))))
      (export main)))
  (call main)
  (output (: 100 Int64)))

; -- eval of a hand-built / nested AST (migrated from rcdzc eval_of_a_compile_time_ast_executes_it_as_code;
; the (eval (quote (+ 1 2)))=3 base is covered above): eval reconstructs the source form an AST denotes and
; folds it through the ordinary path — a HAND-BUILT Ast.* tree reconstructs identically to a quoted one,
; and a nested compound reconstructs+folds.
(case
  "eva1 eval of a hand-built Ast.List reconstructs and executes it identically to a quoted form"
  (doc
    "`(eval (Ast.List (list (Ast.Name \"+\") (Ast.Int 4) (Ast.Int 5))))` = 9 — a hand-built AST value
           (not via quote) reconstructs to `(+ 4 5)` and folds, the same path a quoted argument takes.")
  (input (eval (Ast.List #list((Ast.Name "+") (Ast.Int 4) (Ast.Int 5)))))
  (output (: 9 Int64)))

(case
  "eva2 eval of a nested quoted form reconstructs the compound and folds"
  (doc
    "`(eval (quote (+ (* 2 3) 4)))` = 10 — the reconstructed form is itself compound `(+ (* 2 3) 4)`
           and folds through the ordinary tier.")
  (input (eval (quote (+ (* 2 3) 4))))
  (output (: 10 Int64)))

; -- eval of a quasiquote splicing a compile-time value (migrated from rcdzc eval_of_a_quasiquote_splices_
; a_compile_time_known_value; the let-bound + multi-unquote splices are covered @459/@260 — these pin the
; two distinct variants): an active unquote lifts a compile-time-known operand (here a MODULE-CONST and a
; COMPUTED value) into the reconstructed source, and an eval-of-splice composes inside a larger expression.
(case
  "qqs1 eval of a quasiquote splicing a module-const unquote reconstructs and folds"
  (doc
    "`(def x 3)` then `(eval (quasiquote (+ (unquote x) 4)))` = 7 — a non-literal (module-const)
           unquote splices its value 3 into the reconstructed `(+ 3 4)` and folds (the case that once left
           eval un-desugared as an unbound-name error).")
  (input (do (def x 3) (def (main) (eval (quasiquote (+ (unquote x) 4)))) (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "qqs2 an eval-of-splice composes inside a larger expression"
  (doc
    "`(let ((x 5)) (+ 1 (eval (quasiquote (* (unquote x) 2)))))` = 11 — the eval splices x=5 into
           `(* 5 2)`=10 and the surrounding `(+ 1 …)` folds to 11.")
  (input (do (def (main) (let ((x 5)) (+ 1 (eval (quasiquote (* (unquote x) 2)))))) (export main)))
  (call main)
  (output (: 11 Int64)))

; ── breaker batch 593: quasiquote SPLICE census (the 12-file pins level-machine VALUES; this is
; the runtime-tree-BUILD census face). A quasiquote splicing a runtime Ast value into a template
; per frame builds a real tree; the value is exact (depth 2 x 50 = 100) and the built trees +
; walks leak LINEARLY (10/frame: 100@n10, 500@n50) — the quasiquote face of the walk-leak family
; alongside aq (hoisted quote), ac (constructor build), stt (tree eval). Flips with the reclaim arc.
(case
  "qqb1 fifty quasiquote-spliced runtime-Ast trees are value-exact and leak linearly (the splice-build face)"
  (input
    (do
      (def
        (depth (: node Ast))
        (match
          node
          ((Ast.List es) (match es (#list() 1) (#list(h (.. rest)) (+ 1 (depth h)))))
          (_ 1)))
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (depth (let ((x (Ast.Int (BigInt.of k)))) (quasiquote (f (g (unquote x))))))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 100 Int64))
  (live-objects known-leak))

(case
  "eqmr1 a map-REST pattern inside a QUOTED match reifies OPEN and matches like its direct twin"
  (doc
    "The #6896 fence (breaker counterexample 2026-08-31 vs #6855, v-deferral HIGH-SEV route):
     `(eval (quote (match #map((= 1 10)) (#map((= 1 v) (.. _r)) v) (_ -1))))` must fold to v=10
     exactly as the direct (unquoted) match does — pre-fix the reified map-rest marker closed the
     pattern (fell to the catch-all -1, a wrong-VALUE compile-time fold, worse than the decline it
     replaced). Isolation at filing: quoted #map WITHOUT rest folded correctly, quoted #set WITH
     rest folded correctly — only the map-rest reify was broken. The weighted pair pins the quoted
     face against the direct twin so any future divergence shows as a pair-split.")
  (input
    (do
      (def
        (quoted (: n Int64))
        (+ (eval (quote (match #map((= 1 10)) (#map((= 1 v) (.. _r)) v) (_ -1)))) n))
      (def
        (direct (: n Int64))
        (+ (match #map((= 1 10) (= 2 20)) (#map((= 1 v) (.. _r)) v) (_ -1)) n))
      (def (main (: n Int64)) (+ (* 100 (quoted n)) (direct n)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1111 Int64)))

(case
  "eqmr2 a record-REST pattern inside a QUOTED match reifies OPEN (the #6896 record twin)"
  (doc
    "The record sibling of eqmr1, fixed by the same #6896 open-reify: a quoted match whose arm
     carries `#record((= a v) (.. _r))` folds to the named field's value (v=5 for {a=5,b=6}; main = v+n = 6 at n=1), not the catch-all.")
  (input
    (do
      (def
        (main (: n Int64))
        (+ (eval (quote (match #record((= a 5) (= b 6)) (#record((= a v) (.. _r)) v) (_ -1)))) n))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6 Int64)))

; A misspelled Ast-module member is rejected CDZ0201 with a PREFIX-rank did-you-mean: `Sym` is a PREFIX of
; `Symbol`, and a prefix-extension candidate must LEAD the closest-matches (ranks above an edit-distance hit)
; — #7733 (before, the prefix candidate was dropped). Portable conformance guard, off the rust-#[test].
(case
  "a misspelled Ast module member Sym is rejected CDZ0201 with Symbol leading the did-you-mean (prefix-rank)"
  (doc
    "`Ast.Sym` is not a member of the `Ast` module → CDZ0201 with a `closest matches: …` list, and the
        PREFIX-extension candidate `Symbol` (`Sym` is a prefix) must appear — a prefix hit ranks above an
        edit-distance hit, the #7733 fix. The `(message …)` substrings pin the stable lead `closest matches:`
        and the presence of `Symbol` in the list (both front-end diagnostic, backend-independent).")
  (input (do (def (main) (Ast.Sym 5)) (export main)))
  (error CDZ0201 (message "closest matches:") (message "Symbol")))

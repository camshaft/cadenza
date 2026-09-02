; Macros — a macro is an ORDINARY FUNCTION over the AST (metaprogramming.md §Macros). No `defmacro`: a
; parameter marked `(quote p)` receives its argument UNEVALUATED, as the reflected `Ast` of the syntax
; written at the call site; the function returns an `Ast`; and the compiler EXPANDS the call — splices the
; returned syntax in the call's position and type-checks it as if written directly (§Expansion Precedes And
; Feeds The Core Guarantees). Expansion PRECEDES type checking (§Expansion Runs In Phases To A Fixpoint), so
; the call takes the EXPANSION's type, not the macro's declared `Ast` return. DESIGN-macro-system.md.
(case
  "a quote-parameter macro receives its argument as unevaluated Ast and returns it (identity expansion)"
  (doc
    "The minimal macro: `(def (q (quote x)) x)` — the `(quote x)` parameter binds the ARGUMENT'S SYNTAX
           (an `Ast`), and the body returns it. `(q 42)` reflects the literal `42` to its `Ast`, the macro
           returns that `Ast`, and the compiler SPLICES it back as the source `42`, which evaluates to `42 :
           Int64`. Pins that a macro is an ordinary function over the AST (metaprogramming.md §A Macro Is
           Dispatched By Binding), that a `quote` parameter is call-by-AST (§Expansion Operates On The
           Canonical Representation), and — crucially — that the CALL takes the EXPANSION's `Int64` type, not
           the macro's declared `Ast` return (expansion precedes type checking, §Expansion Runs In Phases).")
  (input (do (def (q (quote x)) x) (def (main) (q 42)) (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro builds new syntax from its argument via quasiquote and the expansion is type-checked directly"
  (doc
    "`(def (twice (quote x)) (quasiquote (+ (unquote x) (unquote x))))` builds the syntax `(+ x x)` with
           the argument's reflected AST spliced at each `(unquote x)`. `(twice 5)` expands to `(+ 5 5)` and
           evaluates to `10 : Int64` — the expansion is ordinary code, type-checked as if written directly
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). A macro-body literal and a
           spliced-argument literal must ground to the SAME `Int64` (no BigInt-vs-Int64 mismatch), so the
           `(+ 5 5)` type-checks — witnessing that expansion feeds ordinary inference cleanly.")
  (input
    (do
      (def (twice (quote x)) (quasiquote (+ (unquote x) (unquote x))))
      (def (main) (twice 5))
      (export main)))
  (call main)
  (output (: 10 Int64)))

(case
  "a macro mixing a body literal with an argument grounds both to Int64 (unless as a plain function)"
  (doc
    "The classic `unless`, as a plain function: `(def (unless (quote c) (quote body)) (quasiquote (if
           (unquote c) 0 (unquote body))))`. `(unless false 7)` expands to `(if false 0 7)` → `7`. Pins that
           a macro-BODY literal (`0`) and a spliced-ARGUMENT literal (`7`) both ground to `Int64` in the
           expansion (they are the two `if` branches — a BigInt-vs-Int64 grounding mismatch would reject
           CDZ0203), so a macro that mixes its own literals with the caller's arguments type-checks as
           ordinary code.")
  (input
    (do
      (def (unless (quote c) (quote body)) (quasiquote (if (unquote c) 0 (unquote body))))
      (def (main) (unless false 7))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a macro may INTRODUCE a local binding in its expansion and reference it within that expansion"
  (doc
    "An expansion is ordinary code, so it may DECLARE a new binding, not only splice into an expression
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). `(def (double-via-local
           (quote e)) (quasiquote (do (def t (unquote e)) (+ t t))))` builds `(do (def t E) (+ t t))` — a
           do-local `(def t …)` whose value is the spliced argument, referenced twice by the macro's OWN
           `(+ t t)`. `(double-via-local 21)` expands to `(do (def t 21) (+ t t))` and evaluates to `42 :
           Int64`. Pins that a macro-INTRODUCED binder resolves for a reference in the SAME expansion: the
           expander must SEED the spliced subtree's lexical scope so the do-local `def` binds `t` for the
           following `(+ t t)` — a binding introduced BY the macro (both binder and its references are
           macro-internal here, so no caller interaction / hygiene question arises). Without scope seeding
           for the spliced-in binder, the `t` references would spuriously unbind (CDZ0101).")
  (input
    (do
      (def (double-via-local (quote e)) (quasiquote (do (def t (unquote e)) (+ t t))))
      (def (main) (double-via-local 21))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro-introduced SIBLING def with a caller-spliced name binds visibly at the enclosing scope"
  (doc
    "An expansion may introduce a TOP-LEVEL sibling def, not only a do-local one. `(def (mkdef (quote nm))
           (quasiquote (def (unquote nm) 43)))` expands `(mkdef answer)` to `(def answer 43)` spliced beside
           `main` in the root `do`. Because the def's NAME is SPLICED FROM A CALLER ARGUMENT (`,nm` = the
           caller's `answer`, use-site identity — the dir-1 unquoted-var rule), the introduced def BINDS
           VISIBLY in the enclosing scope: `(def (main) answer)` resolves to it → `43`. Pins the v-spec-oracle
           gap#4 ruling (visible half): a root-`do` sibling def whose name is caller-spliced is registered in
           the top-level def index after expansion (the load-time scan froze before macros ran, so without
           the post-expansion registration `answer` spuriously unbinds CDZ0101).")
  (input
    (do
      (def (mkdef (quote nm)) (quasiquote (def (unquote nm) 43)))
      (mkdef answer)
      (def (main) answer)
      (export main)))
  (call main)
  (output (: 43 Int64)))

(case
  "a macro-introduced SIBLING def with a macro-internal name stays hygienic-local, not enclosing-visible"
  (doc
    "The hygiene half of the gap#4 ruling: a sibling def whose NAME is a MACRO-TEMPLATE LITERAL (NOT
           spliced from a caller argument) does NOT bind visibly at the caller's enclosing scope — it stays
           hygienic-local, exactly as a template binder is preserve-by-default hygienic. `(def (mkfixed
           (quote _u)) (quasiquote (def fixedName 7)))` splices `(def fixedName 7)` beside `main`, but
           `fixedName` is a template literal, so a caller reference `(def (main) fixedName)` does NOT resolve
           to it — CDZ0101 unbound. Pins that the post-expansion top-level registration is GATED on
           caller-origin provenance (only a caller-spliced name is registered); a macro-internal name is
           never made caller-visible. Contrast the caller-spliced case above (`answer` → 43).")
  (input
    (do
      (def (mkfixed (quote _u)) (quasiquote (def fixedName 7)))
      (mkfixed zzz)
      (def (main) fixedName)
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro-introduced SIBLING FN def with a caller-spliced name binds visibly (callable) at the enclosing scope"
  (doc
    "The fn-def counterpart of the caller-spliced sibling-def case: the spliced def's NAME sits in the
           SIGNATURE list `((unquote nm))`, one level deeper than a value def's bare-name signature. `(def
           (mkfn (quote nm)) (quasiquote (def ((unquote nm)) 42)))` expands `(mkfn answer)` to a nullary fn
           def `(def (answer) 42)` spliced beside `main`. Because the name `answer` is CALLER-SPLICED (from
           the signature-list head), the fn def binds visibly in the enclosing scope: the call `(answer)`
           resolves to it → `42`. Pins that the post-expansion top-level registration reaches the fn/nullary
           shape too (its `register_reduced_callables` wiring only indexes the recursive-self/callee body,
           not the top-level NAME — so without registering the signature-list-head name `(answer)` unbinds).")
  (input
    (do
      (def (mkfn (quote nm)) (quasiquote (def ((unquote nm)) 42)))
      (mkfn answer)
      (def (main) (answer))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro-introduced SIBLING FN def with a macro-internal name stays hygienic-local, not enclosing-callable"
  (doc
    "The hygiene half for the fn-def shape: a spliced fn/nullary def whose NAME is a MACRO-TEMPLATE
           LITERAL (not spliced from a caller arg) does NOT bind callably at the caller's enclosing scope.
           `(def (mkfnI (quote _u)) (quasiquote (def (fixedFn) 7)))` splices `(def (fixedFn) 7)` beside
           `main`, but `fixedFn` is a template literal, so a caller call `(fixedFn)` does NOT resolve —
           CDZ0101 unbound. Pins that the caller-origin provenance gate applies to the signature-list-head
           name too (only a caller-spliced fn name is registered; a macro-internal one stays hygienic-local).")
  (input
    (do
      (def (mkfnI (quote _u)) (quasiquote (def (fixedFn) 7)))
      (mkfnI z)
      (def (main) (fixedFn))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro emitting a wrapping (do def…) at a statement position SPLICE-FLATTENS its sibling defs (multi-def idiom)"
  (doc
    "The multi-definition macro idiom (Scheme top-level begin-splice): a macro carries SEVERAL sibling
           defs through its single-`Ast` result by WRAPPING them in a `(do …)`, which — spliced at a
           STATEMENT position — flattens into the enclosing sequence rather than nesting as a scoped block.
           `(def (mkmulti (quote na)) (quasiquote (do (def helper 7) (def (unquote na) helper) helper)))`
           expands `(mkmulti answer)` to `(do (def helper 7) (def answer helper) helper)`; the caller-spliced
           `answer` binds VISIBLY top-level (→ `main`=`answer`=`helper`=7), while the macro-internal `helper`
           stays hygienic-local — usable by the expansion's OWN def (`answer`'s init `helper` resolves) but
           not by the caller. The N-def generalization of the single-def sibling rule under the same per-name
           provenance gate; an expression-position `do` stays a scoped block (unaffected). (The trailing
           `helper` is a sequenced value tail — the flatten registers the DEFS regardless.)")
  (input
    (do
      (def (mkmulti (quote na)) (quasiquote (do (def helper 7) (def (unquote na) helper) helper)))
      (mkmulti answer)
      (def (main) answer)
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a macro-internal name in a splice-flattened wrapping (do def…) stays hygienic-local"
  (doc
    "The hygiene half of the multi-def splice: in `(do (def helper 7) (def NAME helper) helper)` the child
           `helper` is a MACRO-TEMPLATE literal, so it does NOT leak to the caller's enclosing scope even
           though the wrapping `do` splice-flattens — a caller reference `helper` is CDZ0101 unbound. Only the
           caller-spliced child (`NAME`) binds visibly; each flattened child follows its OWN name's
           provenance (per-name gate, exactly as the single-def rule).")
  (input
    (do
      (def (mkmulti (quote na)) (quasiquote (do (def helper 7) (def (unquote na) helper) helper)))
      (mkmulti answer)
      (def (main) helper)
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro splicing a wrapping (do def def) introduces SEVERAL caller-named sibling defs, all visible"
  (doc
    "The N-def case: a two-quote-param macro emits a wrapping `(do (def A 10) (def B 20) 0)` whose BOTH
           child def names are caller-spliced, so both bind visibly top-level — `(mktwo x y)` then
           `(+ x y)` = 30. Confirms splice-flatten registers EACH caller-origin child of a statement-position
           `do`, not just the first, completing the single-def sibling rule to the multi-def idiom. (The
           trailing `0` is a sequenced value tail.)")
  (input
    (do
      (def
        (mktwo (quote a) (quote b))
        (quasiquote (do (def (unquote a) 10) (def (unquote b) 20) 0)))
      (mktwo x y)
      (def (main) (+ x y))
      (export main)))
  (call main)
  (output (: 30 Int64)))

(case
  "a macro-introduced SIBLING type with a caller-spliced name binds visibly; its qualified ctor T.V rides on it"
  (doc
    "A macro may introduce a TOP-LEVEL `(type …)` at a statement position, not only a def. `(def (mktype
           (quote name)) (quasiquote (type (unquote name) (Mk Int64))))` expands `(mktype W)` to `(type W
           (Mk Int64))` spliced beside `main`. Because the TYPE NAME `W` is CALLER-SPLICED, the type binds
           VISIBLY: the QUALIFIED constructor `W.Mk` (member projection on the type) RIDES ON `W`'s
           visibility — `(match (W.Mk 5) (((. W Mk) x) x))` → `5`. Pins the v-spec-oracle gap#7 ruling: a
           caller-spliced type name is registered in the type/ctor index after expansion (the load-time
           synthesis froze before macros ran), and the qualified member path follows structurally. Without
           the post-expansion type registration, `W` / `W.Mk` spuriously unbind CDZ0101.")
  (input
    (do
      (def (mktype (quote name)) (quasiquote (type (unquote name) (Mk Int64))))
      (mktype W)
      (def (main) (match (W.Mk 5) ((W.Mk x) x)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a macro-introduced SIBLING type with a macro-internal name stays hygienic-local"
  (doc
    "The hygiene half: a spliced `(type Hidden …)` whose type NAME is a MACRO-TEMPLATE LITERAL (not
           spliced from a caller argument) does NOT bind visibly at the caller's enclosing scope — a caller
           reference to `Hidden` / `Hidden.Mk` is CDZ0101 unbound. Pins that the post-expansion type
           registration is GATED on caller-origin provenance (only a caller-spliced type name is registered);
           a macro-internal type name is never made caller-visible. Contrast the caller-spliced `W` above.")
  (input
    (do
      (def (mkt (quote _u)) (quasiquote (type Hidden (Mk Int64))))
      (mkt z)
      (def (main) (match (Hidden.Mk 5) ((Hidden.Mk x) x)))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro-internal type is intra-expansion visible to a sibling def's body (type analog of gf6b)"
  (doc
    "Intra-expansion mutual visibility applies UNIFORMLY to types, not just defs (v-spec-oracle type-gf6b
           ruling): a macro-internal `(type W …)` MUST be usable by the expansion's OWN sibling bindings, exactly
           as a macro-internal DEF is (gf6b). `(mk getit)` splices `(type W (Mk Int64))` and a caller-named def
           `getit` whose body `(match (W.Mk 7) ((W.Mk x) x))` references W — both W and the reference are in the
           SAME expansion → W.Mk resolves (qualified, rides on the intra-expansion-visible W) → `(getit)` = 7. The
           type analog of gf6b's intra-expansion visibility; orthogonal to caller visibility (next case). NOTE:
           currently CDZ0101 — type resolution has no structural do-local channel (unlike defs' do_local_binds), so
           a macro-internal type is not yet intra-expansion visible; asserts the idealistic value so it flips to
           PASS when the intra-expansion type registration lands (impl behind inc-3, coordinated w/ v-inference).")
  (input
    (do
      (def (mk (quote nm)) (quasiquote (do (type W (Mk Int64)) (def ((unquote nm)) (match (W.Mk 7) ((W.Mk x) x))) 0)))
      (mk getit)
      (def (main) (getit))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a macro-internal type stays caller-hygienic-local even as it becomes intra-expansion visible (orthogonal axes)"
  (doc
    "The orthogonal caller-visibility half of the type-gf6b ruling (the constraint the intra-expansion fix must
           NOT violate): a macro-internal type W becomes intra-expansion visible (prior case) but stays
           caller-hygienic-local — the CALLER's own def `main` referencing `W.Mk` is CDZ0101 unbound. Pins that the
           two axes are orthogonal (intra-expansion visibility must not leak W to the caller), and specifically
           guards against a single-file type-index registration accidentally making W caller-visible. Passes today
           (W is not caller-visible) and MUST keep passing once the intra-expansion fix lands.")
  (input
    (do
      (def (mk (quote nm)) (quasiquote (do (type W (Mk Int64)) (def ((unquote nm)) (match (W.Mk 7) ((W.Mk x) x))) 0)))
      (mk getit)
      (def (main) (match (W.Mk 9) ((W.Mk x) x)))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro-introduced type's BARE variant ctor follows the ctor name's own provenance (macro-internal is qualified-only)"
  (doc
    "Per-name provenance applies to the CONSTRUCTOR too: in `(type (unquote name) (Mk Int64))` the type
           name `W` is caller-spliced (so `W` + qualified `W.Mk` are visible) but the variant name `Mk` is a
           MACRO-TEMPLATE literal, so the BARE constructor `Mk` stays HYGIENIC-LOCAL — a bare `(Mk 5)` is
           CDZ0101 unbound (reachable only qualified, `W.Mk`). The v-spec-oracle ruling's mixed case: the
           qualified member path rides on the type's visibility (structural), while a bare ctor is a
           separately-gated binding that leaks only when the ctor NAME is itself caller-spliced.")
  (input
    (do
      (def (mktype (quote name)) (quasiquote (type (unquote name) (Mk Int64))))
      (mktype W)
      (def (main) (match (Mk 5) ((W.Mk x) x)))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro-introduced type with a caller-spliced bare ctor name binds the bare constructor visibly"
  (doc
    "The visible half of the bare-ctor provenance: when BOTH the type name and the variant name are
           caller-spliced — `(def (mk2 (quote t) (quote c)) (quasiquote (type (unquote t) ((unquote c)
           Int64))))` then `(mk2 Box Wrap)` → `(type Box (Wrap Int64))` — the BARE constructor `Wrap` binds
           visibly (its name is caller-origin), so `(Wrap 5)` constructs and `(match (Wrap 5) (((. Box Wrap)
           x) x))` → `5`. Completes the per-name gate: a caller-spliced ctor name enters the bare index, a
           macro-internal one does not.")
  (input
    (do
      (def (mk2 (quote t) (quote c)) (quasiquote (type (unquote t) ((unquote c) Int64))))
      (mk2 Box Wrap)
      (def (main) (match (Wrap 5) ((Box.Wrap x) x)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "one macro splices a TYPE and a fn over it via a wrapping (do …) — the type splice-flattens alongside the def"
  (doc
    "The intersection of the multi-def splice-flatten (a statement-position wrapping `(do …)` flattens its
           child bindings) and macro-spliced type registration: a SINGLE macro emitting BOTH a `(type …)` and
           a `(def …)` over it through one wrapping `do` must register the TYPE too, not only the def. `(def
           (mktf (quote tn) (quote fn)) (quasiquote (do (type (unquote tn) (Mk Int64)) (def ((unquote fn) (:
           v Int64)) ((. (unquote tn) Mk) v)) 0)))` — `(mktf T mk)` splices `(type T (Mk Int64))` + `(def (mk
           v) (T.Mk v))`; both the caller-spliced type `T` (so the def body's `T.Mk` resolves) and the fn
           `mk` bind, so `(match (mk 5) ((T.Mk x) x))` → `5`. Pins that a spliced statement-position `do`
           flattens TYPE children as well as def children (each per its own caller-origin provenance) — the
           def alone flattened before this, leaving `T` CDZ0101 unbound.")
  (input
    (do
      (def
        (mktf (quote tn) (quote fn))
        (quasiquote
          (do
            (type (unquote tn) (Mk Int64))
            (def ((unquote fn) (: v Int64)) ((. (unquote tn) Mk) v))
            0)))
      (mktf T mk)
      (def (main) (match (mk 5) ((T.Mk x) x)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a macro-internal type in a wrapping (do …) splice stays hygienic-local"
  (doc
    "The hygiene half of the compositional case: when the wrapping-do's `(type Hid …)` has a
           MACRO-TEMPLATE-literal name (not caller-spliced), the type stays hygienic-local — a caller
           reference `Hid` / `Hid.Mk` is CDZ0101 unbound, even though a caller-spliced def in the same
           wrapping do would flatten visibly. Confirms the type-child flatten is GATED on the type name's
           own caller-origin provenance, exactly like the def-child and the direct-type cases.")
  (input
    (do
      (def
        (mktf (quote fn))
        (quasiquote (do (type Hid (Mk Int64)) (def ((unquote fn) (: v Int64)) (Hid.Mk v)) 0)))
      (mktf mk)
      (def (main) (match (Hid.Mk 5) ((Hid.Mk x) x)))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "splice-flatten RECURSES through a nested statement-position (do …) — a def in a non-final inner do binds visibly"
  (doc
    "Splice-flatten applies RECURSIVELY, position-per-level (v-spec-oracle recursion ruling): a nested
           `(do …)` that is at a NON-FINAL (statement) position within a flattened outer do ITSELF flattens.
           `(def (mko (quote na)) (quasiquote (do (do (def (unquote na) 9) 0) 0)))` — `(mko answer)` splices
           `(do (do (def answer 9) 0) 0)`; the inner `(do (def answer 9) 0)` is the outer's NON-FINAL
           statement (the outer tail is the final `0`), so it flattens, and its non-final `(def answer 9)`
           registers → `answer` binds VISIBLY (`main`=`answer`=9). Pins recursion at every nesting level for a
           statement-position do; the final `0`s are discarded values.")
  (input
    (do
      (def (mko (quote na)) (quasiquote (do (do (def (unquote na) 9) 0) 0)))
      (mko answer)
      (def (main) answer)
      (export main)))
  (call main)
  (output (: 9 Int64)))

(case
  "splice-flatten does NOT descend a TAIL-position nested (do …) — its defs stay scoped (the boundary)"
  (doc
    "The tail boundary of recursive splice-flatten: a nested `(do …)` at the FINAL/TAIL (value/expression)
           position of a flattened do STAYS SCOPED — its bindings are do-local, not enclosing-visible.
           `(quasiquote (do 0 (do (def (unquote na) 9) 0)))` — the inner `(do (def answer 9) 0)` IS the outer
           do's tail (its value), so it does NOT flatten: a caller reference `answer` is CDZ0101 unbound.
           Pins the non-final=flatten / final=scoped rule per level — a macro wanting a scoped block returns a
           do in value position, and it is never spuriously flattened. Contrast the non-final case above
           (`answer`=9).")
  (input
    (do
      (def (mko (quote na)) (quasiquote (do 0 (do (def (unquote na) 9) 0))))
      (mko answer)
      (def (main) answer)
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro-internal helper introduced in a NESTED do is usable by an outer sibling (structural flatten, intra-expansion visibility)"
  (doc
    "Splice-flatten is STRUCTURAL, not registration-only (v-spec-oracle gf6b ruling): a non-final nested
           `(do …)` inlines its bindings into the enclosing sequence, so a helper introduced ONE nesting
           level down is a DIRECT sibling of the outer forms and usable by them — INTRA-EXPANSION mutual
           visibility (orthogonal to CALLER visibility). `(def (mk (quote nm)) (quasiquote (do (do (def
           (deep) 3) 0) (def ((unquote nm)) (deep)) 0)))` — `(mk outer)` splices a wrapping do whose FIRST
           statement is a nested `(do (def (deep) 3) 0)`; structural flatten inlines it so `deep` and the
           caller-named `outer` become siblings, and `outer`'s body `(deep)` resolves → `(outer)` = 3. Pins
           that a nested-do helper is usable by a same-expansion sibling (matching the direct-sibling case);
           without structural flatten `deep` was CDZ0101 from the outer sibling.")
  (input
    (do
      (def (mk (quote nm)) (quasiquote (do (do (def (deep) 3) 0) (def ((unquote nm)) (deep)) 0)))
      (mk outer)
      (def (main) (outer))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a macro-internal helper flattened from a nested do stays CALLER-hygienic-local (orthogonal to intra-expansion visibility)"
  (doc
    "The orthogonal CALLER-visibility half of gf6b: structural flatten makes a nested-do helper
           intra-expansion visible (usable by the expansion's own siblings), but per-name provenance still
           keeps a MACRO-INTERNAL name CALLER-hygienic-local — the caller cannot see `deep`. Same macro as
           above, but `main` (the caller) references `deep` directly → CDZ0101 unbound. Pins that the two
           levels are orthogonal: `deep` is usable WITHIN the expansion (prior case, `(outer)`=3) yet NOT
           caller-visible (here) — structural flatten governs intra-expansion, the provenance gate governs
           caller visibility.")
  (input
    (do
      (def (mk (quote nm)) (quasiquote (do (do (def (deep) 3) 0) (def ((unquote nm)) (deep)) 0)))
      (mk outer)
      (def (main) (deep))
      (export main)))
  (error CDZ0101 (message "unbound name")))

(case
  "a macro may introduce LET bindings in its expansion and reference them (a distinct binder form)"
  (doc
    "The macro-introduced binding need not be a do-local `def` — a `let` bindings-list works the same
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). `(def (let2 (quote a)
           (quote b)) (quasiquote (let ((u (unquote a)) (v (unquote b))) (+ u v))))` builds `(let ((u A) (v
           B)) (+ u v))` — TWO let binders whose inits are the spliced arguments, both referenced by the
           macro's own `(+ u v)`. `(let2 30 12)` expands to `(let ((u 30) (v 12)) (+ u v))` and evaluates to
           `42 : Int64`. Pins that the expander seeds the spliced subtree's scope for a `let` bindings-list
           (a DIFFERENT binder-candidate shape than a do-local `def`), so both introduced binders resolve
           for the macro's own references — the binders are macro-internal (no caller interaction / hygiene
           question). Without scope seeding for the spliced-in `let`, the `u`/`v` references would unbind
           (CDZ0101).")
  (input
    (do
      (def (let2 (quote a) (quote b)) (quasiquote (let ((u (unquote a)) (v (unquote b))) (+ u v))))
      (def (main) (let2 30 12))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro-introduced binder does NOT capture a caller identifier of the same name (hygiene, do-local def)"
  (doc
    "Macros are HYGIENIC — expansion PRESERVES the caller's bindings (metaprogramming.md §Macros Are
           Hygienic): a name a macro INTRODUCES in its expansion must not capture a same-named identifier
           the CALLER passed in. `(def (capture (quote body)) (quasiquote (do (def x 100) (unquote body))))`
           introduces a do-local `x`; `(capture x)` at a call site where the caller has its OWN `(def x 1)`
           passes the caller's `x` as `body`. Hygienically the caller's `x` still denotes the caller's `1`
           — the macro's introduced `x` is alpha-renamed so it does NOT shadow the spliced argument — so
           `(capture x)` evaluates to `1 : Int64`, NOT `100`. Pins capture-avoidance for a macro-introduced
           do-local `def` binder (a naive splice would produce `(do (def x 100) (+ 0 x))` reading the
           macro's `x` and wrongly yield 100). The `(+ 0 …)` wrapper keeps the caller's argument off the
           do's bare tail (an ML-surface round-trip detail) without changing the hygiene it witnesses.")
  (input
    (do
      (def (capture (quote body)) (quasiquote (do (def x 100) (+ 0 (unquote body)))))
      (def (main) (do (def x 1) (capture x)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a macro-introduced LET binder does NOT capture a caller identifier of the same name (hygiene)"
  (doc
    "Hygiene applies to every binder form, not only a do-local `def` (metaprogramming.md §Macros Are
           Hygienic). `(def (wrap (quote body)) (quasiquote (let ((x 100)) (unquote body))))` introduces a
           `let`-bound `x`; `(wrap x)` where the caller has its own `(def x 1)` passes the caller's `x` as
           `body`. The macro's `let`-bound `x` is alpha-renamed so it does not shadow the spliced argument,
           so the caller's `x` still denotes `1` and `(wrap x)` evaluates to `1 : Int64`, not `100`. Pins
           capture-avoidance for a macro-introduced `let` binder (parallel to the do-local `def` case).")
  (input
    (do
      (def (wrap (quote body)) (quasiquote (let ((x 100)) (unquote body))))
      (def (main) (do (def x 1) (wrap x)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a macro may introduce a HELPER FUNCTION and its parameter resolves in the function's body"
  (doc
    "An expansion may declare a nested FUNCTION, not only a value binding (metaprogramming.md §Expansion
           Precedes And Feeds The Core Guarantees). `(def (via-fn (quote arg)) (quasiquote (do (def (helper
           p) (+ p 100)) (helper (unquote arg)))))` introduces a do-local function `helper` whose body
           `(+ p 100)` references its OWN parameter `p`, then calls it with the spliced caller argument.
           `(via-fn 5)` expands to `(do (def (helper p) (+ p 100)) (helper 5))` and evaluates to `105 :
           Int64`. Pins that a macro-introduced function-def's PARAMETER resolves in its body: the expander
           seeds the spliced signature's parameter scope (a macro-introduced `(def (f p…) …)` is past the
           load-time binder index, so the resolver falls back to a live parameter scan) — otherwise the
           body's `p` would spuriously unbind (CDZ0101). The caller's argument is spliced at the CALL site
           (`(helper 5)`), evaluated in the caller's scope, so no capture question arises.")
  (input
    (do
      (def (via-fn (quote arg)) (quasiquote (do (def (helper p) (+ p 100)) (helper (unquote arg)))))
      (def (main) (via-fn 5))
      (export main)))
  (call main)
  (output (: 105 Int64)))

(case
  "a macro may introduce a RECURSIVE helper function and its recursive self-call lowers + its parameter is inferred"
  (doc
    "The recursive twin of the helper case above (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees): a macro introduces a do-local `(def (fact p) (if (> p 1) (* p (fact (- p 1)))
           1))` — a RECURSIVE function — then calls it with the spliced caller argument. `(via-fn 5)`
           expands to `(do (def (fact p) …) (fact 5))` and MUST evaluate to `120 : Int64`, exactly as the
           post-expansion-equivalent hand-written `(do (def (fact p) …) (fact 5))` does. Post-expansion
           equivalence is the core macro guarantee: expanded AST is compiled exactly as if written directly.
           Pins TWO mechanisms a macro-spliced recursive def needs that a load-time def gets for free — the
           spliced def is a FRESH post-load body absent from the load-time indexes:
             (1) the recursive self-call `(fact (- p 1))` must lower to a `Core::Call`, which needs the
                 spliced def registered as a callable (`register_reduced_callables` in `expand_macros`) —
                 else `callee_def_index` misses it and it declines CDZ0900 'needs runtime specialization';
             (2) the parameter `p` must be INFERRED (`solve_recursive_params`, A2), which needs `p`'s
                 signature occurrence to resolve as a `Resolved::Param` — that goes through
                 `resolve::is_param_occurrence`, which walks `parent_of`, so the spliced subtree's PARENT
                 index must be populated (`expand_macros` rebuilds it after the splice) — else `p` resolves
                 as an unbound `Poison`, `type_of`=`Any`, the solve never fires, and the recursive call
                 declines 'a parameter whose type could not be inferred'.
           (Was breaker-fenced `mrf1`; fixed by the two mechanisms above, both in `expand_macros`.)")
  (input
    (do
      (def
        (via-fn (quote arg))
        (quasiquote (do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact (unquote arg)))))
      (def (main) (via-fn 5))
      (export main)))
  (call main)
  (output (: 120 Int64)))

(case
  "a NESTED macro expansion (macro whose output calls another macro) introduces a recursive helper that lowers + infers"
  (doc
    "The multi-ROUND twin of the recursive-helper case above: expansion runs to a FIXPOINT
           (metaprogramming.md §Expansion Runs In Phases To A Fixpoint), so a macro whose output CONTAINS
           ANOTHER macro call is expanded in a LATER round. Here `wrap` expands to `(+ 0 (mkrec x))`, then
           `mkrec` expands (a second round) to a do-local RECURSIVE `(def (fib n) …)` + `(fib x)`. The
           inner recursive helper must STILL lower (its self-call → `Core::Call`) and infer its param,
           exactly as a first-round recursive helper does — `(wrap 10)` must evaluate to `55 : Int64`
           (`fib 10`). Pins that the reduced-callable registration (`register_reduced_callables`) reaches a
           def spliced in a NON-FIRST round: the round-1 splice marks the (then plain) `(mkrec x)` call
           node walked, so the round-2 re-splice into a do-block must UN-MARK that node before the walk,
           else `collect_reduced_callables` returns early and the inner `fib` is never registered →
           CDZ0900 on its self-call. (mrf1 nested-macro follow-up.)")
  (input
    (do
      (def
        (mkrec (quote a))
        (quasiquote
          (do (def (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) (fib (unquote a)))))
      (def (wrap (quote x)) (quasiquote (+ 0 (mkrec (unquote x)))))
      (def (main) (wrap 10))
      (export main)))
  (call main)
  (output (: 55 Int64)))

(case
  "a macro-introduced REFERENCE resolves in the macro's definition scope, not captured by a caller binder (hygiene dir-2)"
  (doc
    "Direction-2 hygiene (metaprogramming.md §Macros Are Hygienic): a name a macro INTRODUCES as a
           REFERENCE MUST NOT be CAPTURED by a same-named binder at the use site — the DUAL of the binder
           cases above (which pin that an introduced BINDER does not capture a caller reference). `(def g
           100)` binds `g` in the macro's DEFINITION scope; `(def (m (quote body)) (quasiquote (+ g (unquote
           body))))` references that `g`. The caller `(do (def g 1) (m 5))` has its OWN `g = 1`. Hygienically
           the macro's introduced `g` denotes the definition-scope `100`, NOT the caller's `1`, so `(m 5)`
           references the definition-site `g` and evaluates to `105 : Int64` (`(+ 100 5)`), not the captured
           `6` (`(+ 1 5)`). SHOULD-WORK, tracked known-fail: the reference-side dual is not yet implemented — a
           macro-introduced free reference is reified as bare syntax and re-resolves at the use site, so it is
           currently CAPTURED (value 6, a silent hygiene miscompile violating §Macros Are Hygienic). Pinned as
           a tracked known-fail (baseline `fail`, non-redding) until the definition-scope-capture mechanism
           lands (owner v-metaprogramming; overlaps the closure-capture work), then re-pinned pass.")
  (input
    (do
      (def g 100)
      (def (m (quote body)) (quasiquote (+ g (unquote body))))
      (def (main) (do (def g 1) (m 5)))
      (export main)))
  (call main)
  (output (: 105 Int64)))

; ── breaker: the post-expansion-equivalence gap. The identical do-local recursive fn compiles and
; computes 120 when written PLAINLY (09/02 pin the plain form), but the macro-introduced expansion of
; the SAME shape declines CDZ0900 "a recursive function needs runtime specialization" — the expander's
; live-parameter-scan fallback (#7529) doesn't feed the recursion machinery the way a load-time def
; does. metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees says the expanded program
; is an ordinary program; this pins the should-work value and tracks the gap (v-metaprogramming).
(case
  "mrf1 a macro-introduced RECURSIVE helper computes like its plainly-written twin (should-work: expansion precedes the core guarantees)"
  (doc
    "`(via-rec 5)` expands to `(do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact 5))` —
           byte-for-byte the plain do-local recursive fn that compiles and computes 120 when written
           directly. Post-expansion equivalence (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees) says the macro-introduced twin MUST behave identically. Today the expansion
           declines CDZ0900 (runtime-specialization not-yet) — the #7529 parameter-scope fallback covers
           the parameter but not the fn's own recursive self-reference. MUST be 120.")
  (input
    (do
      (def
        (via-rec (quote arg))
        (quasiquote (do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact (unquote arg)))))
      (def (main) (via-rec 5))
      (export main)))
  (call main)
  (output (: 120 Int64)))

(case
  "eval sees through macro expansion — a macro producing a (quote …) evals as the plain form"
  (doc
    "`eval` and macro expansion are ONE compile-time tier (metaprogramming.md §Compile-Time Evaluation Is
           One Tier), so `(eval (MACRO …))` whose expansion is a `(quote …)` must behave as the plain
           `(eval (quote …))`. `(def (mk-quoted (quote e)) (quasiquote (quote (unquote e))))` expands
           `(mk-quoted (+ 2 3))` to `(quote (+ 2 3))`; `(eval (mk-quoted (+ 2 3)))` therefore evals `(+ 2 3)`
           to `5 : Int64` — byte-identical to the plain `(eval (quote (+ 2 3)))`. Pins that eval SEES THROUGH
           expansion: the quote/eval reconstruction is re-run over the EXPANDED program (the load-time passes
           precede expansion), so an eval-argument that only became a visible `(quote …)` via a macro is
           reconstructed + folded, not rejected CDZ0101 (a post-expansion-equivalence guarantee).")
  (input
    (do
      (def (mk-quoted (quote e)) (quasiquote (quote (unquote e))))
      (def (main) (eval (mk-quoted (+ 2 3))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a non-terminating self-recursive macro is a hard CDZ0999 error, not a compiler hang"
  (doc
    "A macro whose expansion keeps producing another call to itself — `(def (m (quote e)) (quasiquote (m
           (unquote e))))` with `(m 5)` — has no fixpoint. The expander caps the expansion fuel and reports
           a hard CDZ0999 (the compile-time-reduction-bound band — macro expansion is compile-time
           evaluation) instead of LOOPING FOREVER, which hung the compiler AND `cdz check` (freezing the
           LSP / diagnostics-as-you-type on one accidental self-referential macro edit). Pins that a
           non-terminating macro is a DIAGNOSABLE error, not a hang — a robustness guarantee for the tool.")
  (input (do (def (m (quote e)) (quasiquote (m (unquote e)))) (def (main) (m 5)) (export main)))
  (error CDZ0999 (message "macro expansion did not terminate")))

(case
  "a macro wrapping the spliced body in a HANDLE keeps the caller's names in scope"
  (doc
    "A macro may WRAP the caller's expression in a handler and still see the caller's bindings — the
           with-default / with-handler macro class (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees). `(def (wrap (quote body)) (quasiquote (handle E 0 ((bail () s 99)) (unquote
           body))))` wraps the spliced body in a `handle`; `(wrap (+ n 5))` where the caller has `(def n 3)`
           expands to `(handle E 0 ((bail () s 99)) (+ n 3+…))` and the spliced `(+ n 5)` still resolves the
           caller's `n` → `8 : Int64`. Pins that (a) a macro-produced handle's nullary arm's EMPTY parameter
           list `()` reconstructs as an empty list (not a `(trap \"malformed AST\")`, which mis-read the arm
           CDZ0201), and (b) the spliced body inside the handle keeps caller scope (no spurious unbound).")
  (input
    (do
      (effect E (op bail (-> Int64)))
      (def (wrap (quote body)) (quasiquote (handle E 0 ((bail () s 99)) (unquote body))))
      (def (main) (do (def n 3) (wrap (+ n 5))))
      (export main)))
  (call main)
  (output (: 8 Int64)))

(case
  "a macro-introduced MATCH resolves its arm-pattern binders in the arm body (accessor macros)"
  (doc
    "The bread-and-butter accessor-macro class: a macro whose expansion is a `match` whose arm PATTERN
           binds names its body uses. `(def (fst (quote e)) (quasiquote (match (unquote e) (#tuple(a _b)
           a))))` expands `(fst #tuple(7 9))` to `(match #tuple(7 9) (#tuple(a _b) a))` → `7 : Int64`. Pins
           that a macro-introduced match arm's pattern binder (`a`) resolves in the arm body: the expander
           seeds the spliced subtree's scope AFTER rebuilding the parent index, so the arm — a headless
           `(pattern body)` recognized as a binding scope only via its `match` PARENT — is a candidate and
           its body binder resolves (without the after-parent ordering, the arm-body `a` unbinds CDZ0101).")
  (input
    (do
      (def (fst (quote e)) (quasiquote (match (unquote e) (#tuple(a _b) a))))
      (def (main) (fst #tuple(7 9)))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a macro evaluates a caller argument via the Eval.in-caller op — the compile-time Eval effect (slice 1: literal-valued)"
  (doc
    "The `Eval` effect's `in-caller` operation evaluates an AST in the CALLER's environment AT EXPANSION
           and returns the evaluated value reified back to an `Ast` (DESIGN-macro-system.md §3) — a
           COMPILE-TIME effect the macro expander discharges and ERASES before type-checking. `(m 4)`
           reifies the caller argument to the AST `4`; `(Eval.in-caller x)` evaluates it in the caller env
           → `4`, reified back to the literal `4`; the expansion `(+ 4 1)` computes 5. Pins that the op
           RESOLVES and FOLDS at expansion — the perform is discharged and erased, so it never reaches the
           no-home check (no CDZ0401) and no `{Eval}` row survives to inference. A literal-valued in-caller;
           compound const-evaluation and caller-scope capture are later increments.")
  (input
    (do
      (def (m (quote x)) (quasiquote (+ (unquote (Eval.in-caller x)) 1)))
      (def (main) (m 4))
      (export main)))
  (call main)
  (output (: 5 Int64)))

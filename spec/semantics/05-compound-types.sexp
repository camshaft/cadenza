; Compound types — witnesses core-semantics.md and type-system.md: record/sum/list/map construction,
; field read, structural equality, matching, and the list-index trap. The primary clause is the
; recorded oracle: a well-typed program's value or trap, or — for an ill-typed program — its
; (error <CODE>) rejection, because an ill-typed program has no run and therefore no terminal value.
; For a type rule a generation does not yet cover it DECLINES rather than running (reject-don't-
; miscompile); the gate scores a decline as todo, not disagreement. (needs …) marks later
; capabilities. Diagnostic codes are from options/diagnostics-schema/.

(case "a record is constructed and a field is read"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field. The dotted
           display `p.x` is sugar for the canonical member-access form (. p x); a reader
           expands one to the other (options/code-shape/), so both denote the same tree.")
  (input  (let ((p (record (x 1) (y 2)))) p.x))
  (output (: 1 Int64)))

; --- A record's field names are a SET: each name appears at most once --------------------
; core-semantics.md #A Record Has A Fixed Set Of Named Fields: "A record MUST associate a fixed SET
; of statically-known field names each with a value." A set has each name once, so a record literal
; that repeats a field name is ill-typed and the compiler REJECTS it (CDZ0201) rather than build a
; record with two fields of the same name (which makes `(. r a)` ambiguous: which `a`?).

(case "a record with a duplicate field name is a type error"
  (doc    "`(record (a 1) (a 2))` names the field `a` twice — not a fixed SET of field names, so it is
           ill-typed and the compiler rejects it (CDZ0201, core-semantics.md #A Record Has A Fixed Set
           Of Named Fields), or declines if it does not yet cover the fixed-field-set rule
           (reject-don't-miscompile).")
  (needs      collections)
  (input      (record (a 1) (a 2)))
  (error      CDZ0201))

(case "a record with a non-adjacent duplicate field name is a type error"
  (doc    "The duplicate need not be adjacent: `(record (a 1) (b 2) (a 3))` still names `a` twice, so
           it is ill-typed (CDZ0201). Pins that the fixed-field-set check is over the whole field list,
           not only consecutive names.")
  (needs      collections)
  (input      (record (a 1) (b 2) (a 3)))
  (error      CDZ0201))

; A SUM's variant names are a set too, exactly as a record's field names are — type-system.md #The
; Structural Types makes a sum "of named variants" whose shape is "its variant names with their payload
; types", and #Structural Values Are Comparable Only When Their Shapes Match speaks of a sum's "variant
; SET"; #A Match Is Exhaustive Against The Sum Type's Variant Set checks exhaustiveness against that set.
; For the variant set to be well-defined, the variant names must be distinct, so a sum declaring the same
; variant name twice — `(type T (A Int64 | A Bool))` — is ill-formed and MUST be rejected (CDZ0201), the
; same duplicate-member ill-formedness a record with a duplicate field (`(record (a 1) (a 2))` above), a
; module with a duplicate definition (11-modules.sexp), and an effect declaring an operation twice
; (14-effects-and-handlers.sexp) are rejected for. A compiler that registers each variant without checking
; for a name already declared binds `A` twice with two payload types — `(T.A 5)` and `(T.A true)` both
; construct, an ambiguous variant the closed variant set forbids. This is the fourth closed name-set (after
; record fields, module definitions, and effect operations) whose duplicate-member check the family
; requires. A generation that does not yet detect a duplicate variant name declines rather than binding one.

(case "a sum declaring a variant name twice is a type error"
  (doc    "`(type T (A Int64 | A Bool))` declares the variant `A` twice — but a sum's variant names are a
           SET (type-system.md #The Structural Types Are Record, Tuple, And Sum: a sum's shape is its
           variant names with their payload types; #A Match Is Exhaustive Against The Sum Type's Variant
           Set), so declaring `A` twice makes the variant set ill-defined and MUST be rejected (CDZ0201),
           the same duplicate-member ill-formedness a record with a duplicate field (`(record (a 1) (a
           2))`), a module with a duplicate definition, and an effect declaring an operation twice are
           rejected for. Pins that the duplicate-member check reaches a sum's variant set — the fourth
           closed name-set beside record fields, module definitions, and effect operations. A compiler that
           registers each variant without the check binds `A` twice with two payload types (`(T.A 5)` and
           `(T.A true)` both construct), an ambiguous variant the closed set forbids. A generation that does
           not yet detect a duplicate variant name declines rather than binding one.")
  (needs      sum-type-declaration)
  (input      (do
                (type T (A Int64) (A Bool))
                (def (main) 1) (export main)))
  (error      CDZ0201))

(case "member access is written explicitly as the dot form in the canonical tree"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field: the canonical
           form is (. <record> <key>); `p.y` is its display sugar. This case writes the
           canonical form directly to pin that the tree carries (. …), not a dotted atom.")
  (input  (let ((p (record (x 1) (y 2)))) (. p y)))
  (output (: 2 Int64)))

(case "a boolean record field is projected as the program result"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field at a non-Int field type:
           a record field may hold any type, and projecting a Bool field must carry that Bool across
           the run boundary as the program's result. A field's type is whatever the field holds, not
           uniformly Int64 (an Int64 field already works; this pins a Bool field does too — the
           companion of the boolean tuple-element case).")
  (needs   collections)
  (input   (. (record (flag true)) flag))
  (output  (: true Bool)))

(case "member access on a non-record is a type error"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field (2nd sentence):
           projecting a field of a non-record value has no defined result. Member access requires a
           record operand and 5 is an Int64, so the compiler rejects it (CDZ0201) rather than emit a
           component that traps.")
  (input     (. 5 x))
  (error     CDZ0201))

(case "member access on a boolean is a type error"
  (doc    "The Bool-operand companion: `(. true x)` accesses a field of a non-record, a type error the
           compiler rejects (CDZ0201).")
  (input     (. true x))
  (error     CDZ0201))

(case "member access on a tuple is a type error"
  (doc    "A tuple is not a record — it has positional elements, not named fields (accessed by
           `(. x N)`, not `.field`). So `(. (tuple 1 2) f)` is member access on a non-record: a type
           error the compiler rejects (CDZ0201).")
  (input     (. (tuple 1 2) f))
  (error     CDZ0201))

(case "member access on a string is a type error"
  (doc    "A String is not a record, so `(. \"hi\" x)` is member access on a non-record: a type error
           the compiler rejects (CDZ0201).")
  (input     (. "hi" x))
  (error     CDZ0201))

; --- Accessing a field the record does not have is a static type error, not a runtime trap --------
; The cases above reject member access on a NON-record (an Int, Bool, Tuple, String) at compile time —
; the projection has no defined result, so the compiler rejects rather than emitting a component that
; traps. The SIBLING situation is member access on a record that DOES NOT HAVE the named field: a
; record's TYPE is its field names with their types (type-system.md #The Structural Types Are Record,
; Tuple, And Sum: "a record's field names with their types"), and member access projects "the field
; named by its key FROM the record" (core-semantics.md #Member Access Projects A Record Field) — a
; field the record's type does not carry cannot be projected, so projecting it has no defined result,
; exactly as projecting a field of a non-record does. The row operations already fix this outcome
; UNCONDITIONALLY: projecting-to (type-system.md #A Record Is Restricted To A Named Set Of Its Fields)
; and dropping (§#A Record Is Reduced By Dropping…) "a field the operand record does not contain MUST
; be rejected at compile time with the machine-readable code for a required field that is absent." Bare
; member access is the same projection and MUST reject the same way — NOT emit a component that traps
; at run time. The seed instead lowers `p.z` (z absent) to trapping code: a runtime trap where the spec
; mandates a compile-time rejection, the record-operand companion of the non-record member-access cases.
(case "member access of a field the record does not have is a type error"
  (doc    "`(record (x 1))` has the single field `x`; its type carries no field `z` (type-system.md
           #The Structural Types Are Record… — a record's field names with their types). Projecting the
           absent `z` has no defined result, so it is a compile-time type error (CDZ0201), the
           record-operand companion of `(. 5 x)` (member access on a non-record, rejected above) and the
           bare-access analogue of the row-projection rule that rejects naming a field the operand does
           not contain (type-system.md #A Record Is Restricted To A Named Set Of Its Fields;
           core-semantics.md #Member Access Projects A Record Field). Rejected before lowering, not
           deferred to a runtime trap. A valid field (`(. (record (x 1)) x)` = 1) is unaffected.")
  (input     (. (record (x 1)) z))
  (error     CDZ0201))

(case "member access of an absent field on a function-returned record is a type error"
  (doc    "The field-presence check reaches a record RETURNED by a function, not only a literal: `(mk)`
           returns `(record (x 1))`, whose type carries only `x`, so `(. (mk) z)` names an absent field —
           a compile-time type error (CDZ0201). `resolve` beta-reduces the call to its `(record …)` body,
           so the field set is known at the access site exactly as for a literal (the record-field twin of
           the tuple-arity check reaching a function-returned tuple). A record PARAMETER, whose field set
           is not known in the callee, resolves to no `(record …)` and imposes nothing — it is not
           false-rejected (declining or taking the runtime path instead).")
  (input  (do
            (def (mk) (record (x 1)))
            (def (main) (. (mk) z)) (export main)))
  (error  CDZ0201))

; The positional tuple accessor `(. x N)` requires a TUPLE operand, exactly as member access `.`
; requires a record operand (above). Applying `(. x N)` to a non-tuple — a scalar, a record, a sum —
; has no defined result, so the compiler MUST reject it (CDZ0201) rather than emit a component that
; traps: the same projection-on-the-wrong-kind class as member access, for positional access.

(case "tuple access on a non-tuple is a type error"
  (doc    "`(. 5 0)` projects positional element 0 of the Int64 `5`, which is not a tuple — a type
           error the compiler rejects (CDZ0201), just as `(. 5 x)` (member access on a non-record) is
           rejected above.")
  (input     (. 5 0))
  (error     CDZ0201))

(case "tuple access on a record is a type error"
  (doc    "The record-operand companion: a record has named fields, not positional elements, so
           `(. (record (a 1)) 0)` applies a positional accessor to a non-tuple — a type error
           (CDZ0201), the mirror of `(. (tuple 1 2) f)` (member access on a tuple) above. Pins that
           `(. x N)` requires a tuple, rejecting a record operand.")
  (needs     collections)
  (input     (. (record (a 1)) 0))
  (error     CDZ0201))

; The index of a positional tuple access must be WITHIN the operand tuple's static arity, not only a
; tuple at all. A tuple's arity is part of its type (a fixed-size positional value), so an index outside
; `0..arity` names a position the tuple does not have — a type error the compiler knows statically.
; type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix: "a positional tuple access
; whose index is out of the tuple's static arity [MUST be] rejected" at compile time, "so that a split
; can never name a position the tuple does not have." This is the arity companion of the non-tuple-operand
; cases above: `(. (tuple 10 20 30) 3)` accesses index 3 of a statically-3-element tuple (valid
; indices 0..2), so the compiler MUST reject it (CDZ0201) rather than emit a component that traps at
; run time — a compile-time-knowable ill-typing must not be deferred to a runtime trap, exactly as
; `(. 5 0)` (non-tuple operand) is rejected rather than trapped. (This is DISTINCT from member
; access of a missing record field, which traps: a record's field set can be runtime-dependent, but a
; tuple's arity is static.) A generation that does not yet check the index range declines rather than
; emitting the trapping access.

(case "a positional tuple access out of the tuple's static arity is a type error"
  (doc    "`(. (tuple 10 20 30) 3)` projects position 3 of a three-element tuple, whose valid
           positions are 0..2 — an index outside the tuple's static arity, which names an element the
           tuple does not have. The tuple's arity is part of its type (type-system.md #A Tuple Is Split
           At A Position Into A Prefix And A Suffix), so the compiler knows this statically and MUST reject
           it (CDZ0201) rather than emit a component that traps at run time, exactly as `(. 5 0)`
           (a non-tuple operand) is rejected rather than trapped. A compile-time-knowable ill-typing
           must not be deferred to a runtime trap. A generation that does not yet range-check the index
           declines rather than emitting the trapping access.")
  (input     (. (tuple 10 20 30) 3))
  (error     CDZ0201))

; The static-arity range check must reach a tuple whose arity is known through a FUNCTION RETURN, not only
; a directly-written tuple literal. `mk` returns `(tuple 1 2)`, so `(mk)`'s result is a two-element tuple —
; its arity (2, valid positions 0..1) is statically known at the projection site (the same shape recovery
; that lets `(. (mk) 1)` project element 1). So `(. (mk) 2)` names position 2, outside the arity,
; a compile-time-knowable ill-typing the compiler MUST reject (CDZ0201) — exactly as the literal
; `(. (tuple 10 20 30) 3)` above and the let-bound `(let ((p (tuple 1 2))) (. p 2))` are. A compiler
; that range-checks the index for a literal and a let-bound tuple but NOT for a fn-return tuple emits a
; component that TRAPS at run time on `(. (mk) 2)` — deferring a compile-time-knowable ill-typing to a
; runtime trap, the very thing the case above forbids. (Distinct from a tuple reached through a PARAMETER,
; whose arity is genuinely unknown in the callee's body and which correctly declines "unknown tuple shape";
; here the arity IS known from the callee's return.) A generation that does not yet range-check the index on
; a fn-return tuple declines rather than emitting the trapping access.

(case "a tuple access out of arity on a function-returned tuple is a type error, not a trap"
  (doc    "`mk` returns `(tuple 1 2)`, so `(mk)` is a two-element tuple whose arity is statically known at
           the projection site (as the valid `(. (mk) 1)` shows). `(. (mk) 2)` names position 2,
           outside the arity 0..1 — a compile-time-knowable ill-typing the compiler MUST reject (CDZ0201,
           type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix: a positional access
           out of the static arity is rejected), exactly as the literal `(. (tuple 10 20 30) 3)` and the
           let-bound `(let ((p (tuple 1 2))) (. p 2))` are. Pins that the arity range check reaches a
           tuple whose arity is known through a function return, not only a literal or let-bound tuple. A
           compiler that checks the literal/let cases but emits a runtime trap for the fn-return case defers
           a compile-time-knowable ill-typing to a trap — the very thing the literal case forbids. (A tuple
           reached through a PARAMETER has genuinely unknown arity and correctly declines instead; here the
           arity is known from the callee's return.) A generation that does not yet range-check a fn-return
           tuple's access declines rather than emitting the trapping access.")
  (needs     sum-type-declaration)
  (input     (do
               (def (mk) (tuple 1 2))
               (def (main) (. (mk) 2)) (export main)))
  (error     CDZ0201))

; Projecting a tuple that arrives as a FUNCTION PARAMETER (a runtime tuple whose shape is not the
; inline literal at the projection site) must either compute the projection or DECLINE — never emit an
; invalid component. `(def (fst t) (. t 0))` applied to `(tuple 7 8)` is well-typed and its value is
; 7 (the inline `(let ((t (tuple 7 8))) (. t 0))` and the beta-reducing `((fn (t) (. t 0)) (tuple
; 7 8))` both yield 7). self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not
; Miscompiled: "A generation whose compiler does not yet compile a construct a program uses MUST decline
; to derive a component … rather than emit a component whose observable behavior diverges" — and emitting
; a component that FAILS wasm validation is neither a decline nor a valid component, the worst outcome.
; The record accessor already takes the correct path on the analogous named-parameter case (`(def (geta
; r) (. r a))` DECLINES "runtime member access on a value of unknown record shape"); `(. x N)` on a
; parameter must at least do the same rather than emit invalid bytes. The recorded oracle is the value 7;
; a generation that cannot yet thread the parameter tuple's shape declines (scored todo), and a generation
; that emits an invalid component FAILs this case.

(case "projecting a tuple passed as a function parameter yields the element, never an invalid component"
  (doc    "`(def (fst t) (. t 0))` applied to `(tuple 7 8)` projects element 0 of a tuple that
           arrives as a parameter — a well-typed program whose value is 7 (the inline and lambda forms
           both compute 7). The compiler MUST either compute the projection or DECLINE
           (self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not Miscompiled), never
           emit a component that fails wasm validation — the worst outcome, neither a decline nor a valid
           component. The record accessor already declines the analogous named-parameter case (`(. r a)`
           on a record parameter → 'unknown record shape'); `(. x N)` on a parameter must not emit invalid
           bytes where the record accessor cleanly declines. A generation that cannot yet thread the
           parameter tuple's shape declines rather than emitting an invalid component.")
  (needs     collections)
  (input     (do
               (def (fst t) (. t 0))
               (def (main)  (fst (tuple 7 8))) (export main)))
  (output    (: 7 Int64)))

(case "member access of a missing field is a type error"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field (3rd sentence):
           projecting a field the record does not contain has no defined result. A record's field
           names are part of its type, so `p`'s type carries only `x`; naming the absent `z` is a
           COMPILE-TIME type error (CDZ0201), the bare-access companion of the row-projection rule
           (type-system.md #A Record Is Restricted To A Named Set Of Its Fields) — rejected before
           lowering, not deferred to a runtime trap. The `let`-bound record `p` resolves to a
           compile-time `(record (x 1))`, so the field set is statically known at the access site.")
  (input  (let ((p (record (x 1)))) (. p z)))
  (error  CDZ0201))

(case "member access on a record chosen by a conditional projects the field"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field with a record value that
           is not written inline but SELECTED at run time by a conditional. Both branches yield a
           record with field `a`; `(. <if…> a)` projects `a` from whichever record the condition
           selects. With the condition true (n=0) the first record is chosen, so the field is 1 — the
           access must project the field, not trap. The record is a genuine record however it was
           produced; member access does not require the record to be a compile-time literal.")
  (input  (do
            (def (f n) (. (if (= n 0) (record (a 1)) (record (a 2))) a))
            (def (main) (f 0)) (export main)))
  (output (: 1 Int64)))

(case "member access on a conditionally-chosen record, other branch"
  (doc    "The companion of the case above with the condition false (n=9): the second record is
           selected, so the field `a` projects to 2. Confirms the projection follows the runtime
           branch selection, not a fixed branch.")
  (input  (do
            (def (f n) (. (if (= n 0) (record (a 1)) (record (a 2))) a))
            (def (main) (f 9)) (export main)))
  (output (: 2 Int64)))

(case "tuple access on a tuple chosen by a conditional projects the element"
  (doc    "Witnesses core-semantics.md tuple positional access with a tuple value SELECTED at run time
           by a conditional. Both branches yield a 2-tuple; `(. <if…> 0)` projects element 0 of
           whichever tuple the condition selects. With n=0 the first tuple is chosen, so element 0 is
           1 — the access must project it, not trap. Same requirement as the record case: a positional
           access works on a tuple however it was produced.")
  (input  (do
            (def (f n) (. (if (= n 0) (tuple 1 9) (tuple 2 9)) 0))
            (def (main) (f 0)) (export main)))
  (output (: 1 Int64)))

; --- Positional access on a RUNTIME tuple bound by `let` -------------------------------------------
; A tuple returned from a function and BOUND BY `let` is a genuine runtime value (a value-heap
; positional array), not a compile-time structure. `(. x N)` on such a bound name reads element N from
; the heap array (`arr-get`), unboxing a scalar element to its kind and keeping a compound element as a
; handle. This is the shape a recursive-descent decoder takes — threading a `(node, next-index)` pair
; through `let` — so it must both project a scalar element and yield a compound element a `match`
; consumes. (Without the runtime path a `(. x N)` on a let-bound runtime tuple emitted an unreachable
; that trapped; these pin the arr-get lowering.)

(case "a scalar element is projected from a let-bound runtime tuple"
  (doc    "`mk` returns a runtime tuple `(tuple (NLit 5) 9)` — a genuine value-heap value because its
           first element is a runtime sum. Bound by `let`, positional access `1` reads its second element, the Int64
           `9`. Pins that positional access on a materialized runtime tuple reads and unboxes a scalar
           element (`arr-get` + `get-int`), the index half of a decoder threading `(node, index)`.")
  (needs  sum-type-declaration)
  (input  (do
            (type Node (NLit Int64) (NAdd (Tuple Node Node)))
            (def (mk) (tuple (NLit 5) 9))
            (def (main) (let ((l (mk))) (. l 1))) (export main)))
  (output (: 9 Int64)))

(case "a compound element projected from a let-bound runtime tuple is matched"
  (doc    "The companion where the projected element is itself a runtime compound: positional access `0` of the
           let-bound tuple is the `Node` sum `(NLit 5)`, which a `match` then consumes to its scalar
           payload 5. Pins that a runtime `(. x N)` yields a heap element a `match` can dispatch on —
           the node half of a decoder's `(node, index)` pair (the exact shape a `bytes → AST` reader
           threads through `let`).")
  (needs  sum-type-declaration)
  (input  (do
            (type Node (NLit Int64) (NAdd (Tuple Node Node)))
            (def (mk) (tuple (NLit 5) 9))
            (def (ev e) (match e ((Node.NLit v) v) ((Node.NAdd (tuple a b)) (+ (ev a) (ev b)))))
            (def (main) (let ((l (mk))) (ev (. l 0)))) (export main)))
  (output (: 5 Int64)))

(case "a scalar element is projected directly from a function's runtime tuple result"
  (doc    "The `let`-free companion of the projected-element cases above: `(. x N)` applied DIRECTLY to a
           NAMED-def call that returns a runtime tuple, with no intervening `let`. `(dec 4)` returns
           `(tuple 40 5)`; `(. (dec 4) 0)` projects 40. Pins that positional access on a runtime
           tuple does not depend on the tuple first being `let`-bound — the `let`-bound cases above
           compile, and so must the direct projection (the shape a reader takes to read just one half of
           a returned pair). Distinct from the inline-tuple case `(. (tuple n (+ n 1)) 1)` (a tuple
           built right at the projection) and the lambda case `(. ((fn …) …) 0)` (compile-time
           reduced): here the tuple comes from a NAMED def, which the compiler does not reduce, so the
           projection must recover the operand's shape at the projection site — earlier this emitted an
           invalid component (a decline-don't-miscompile violation), now fixed.")
  (input  (do
            (def (dec i) (tuple (* i 10) (+ i 1)))
            (def (main) (. (dec 4) 0)) (export main)))
  (output (: 40 Int64)))

(case "a scalar element is projected DIRECTLY from a named function's runtime tuple result"
  (doc    "The `let`-free companion: positional access `0` applied DIRECTLY to a named-def function's runtime-tuple
           result — `(. (dec 4) 0)` with no intervening `let` — projects the scalar element. `dec`
           builds a runtime tuple `(tuple (* n 10) 9)`; element 0 is `(* 4 10)` = 40. Pins that the
           projection recovers the operand's shape at the PROJECTION site, not only at a `let`-binding
           site, so a named-def result is projectable like a `let`-bound one. (This directly built a
           runtime tuple constructor into an import-free scalar module — an INVALID component — before
           the runtime constructor learned to decline on the scalar path and defer to the runtime pass:
           decline-don't-miscompile, then compile.)")
  (input  (do
            (def (dec n) (tuple (* n 10) 9))
            (def (main) (. (dec 4) 0)) (export main)))
  (output (: 40 Int64)))

(case "a recursive resolver whose trapping arm builds a compound compiles"
  (doc    "A recursive `Node → Core` resolver — the self-hosting compiler's front rung — applied to a
           runtime-built `Node` and consumed as a scalar. One arm (`unknown-head`) builds a Core whose
           payload is a DEFINITE TRAP: `(Core.KConst (bad))` where `(bad)` = `(Bytes.len (Bytes.of (list
           256)))` (256 is out of byte range, so the payload const-folds to a trap → `Kind::Never`).
           `resolve` of `(NPrim '+' …)` takes the `KAdd` arm (not the trapping one), so `kind-of` of the
           result is 1. Pins that a divergent (Never) compound element does NOT poison the whole
           function: earlier the trapping arm made `resolve`'s entire body emit an INVALID component on
           the runtime-heap path (a Never element boxed as a half-built `sum-new`; a Never-returning
           helper's i64 leaking to a Heap caller), which declined 'cannot box' / failed validation — the
           final blocker on connecting the reader to the pipeline. A Never element now short-circuits to
           `unreachable`, a Never-bodied function stubs to `unreachable` keeping its inferred signature,
           and a Never argument makes its call diverge — so a resolver with a trapping arm compiles, and
           an actually-unknown head traps at run time (the front-end rejection point).")
  (needs  sum-type-declaration)
  (input  (do
            (type Node (NInt Int64) (NPrim (Tuple String Node Node)))
            (type Core (KConst Int64) (KAdd (Tuple Core Core)))
            (def (bad) (Bytes.len (Bytes.of (list 256))))
            (def (resolve node)
              (match node
                ((Node.NInt n) (Core.KConst n))
                ((Node.NPrim (tuple h a b))
                  (if (= h "+") (Core.KAdd (tuple (resolve a) (resolve b)))
                                (Core.KConst (bad))))))
            (def (kind-of c) (match c ((Core.KConst n) 0) (_ 1)))
            (def (main) (kind-of (resolve (Node.NPrim (tuple "+" (Node.NInt 20) (Node.NInt 22)))))) (export main)))
  (output (: 1 Int64)))

(case "two structural values of the same shape are equal"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural (2nd
           sentence) at the value level: two structural records of the same shape and contents are
           well-typed to compare, and the compiler evaluates the comparison to true.")
  (input  (= (record (x 1) (y 2)) (record (x 1) (y 2))))
  (output (: true Bool)))

; --- Record equality is by field-name SET, so it is independent of the order fields are written ---
; core-semantics.md #A Record Has A Fixed Set Of Named Fields (a record's fields are a SET) together
; with deterministic-value-form.md #Ordering Of Aggregate Members Is Fixed ("The canonical encoding of
; an unordered aggregate MUST place its members in a fixed order derived from the members themselves,
; not from the order in which they were inserted or discovered"). A record's fields are an unordered
; set, so two records with the same field-name set and equal values are EQUAL regardless of the order
; the fields are written — exactly as the map order-independence case (above) requires for maps. The
; equality case above compares records in the SAME written order and so cannot witness this; a
; comparison that naively matched field lists positionally would wrongly report these unequal. This
; pins that record `=` normalizes field order (compares as a set), the record companion of "map equality
; is independent of insertion order".

(case "record equality is independent of the order fields are written"
  (doc    "`(record (a 1) (b 2))` and `(record (b 2) (a 1))` have the same field-name set {a, b} with the
           same values, so they are EQUAL (core-semantics.md #A Record Has A Fixed Set Of Named Fields;
           a set is unordered) — true regardless of the written order. Pins that record `=` compares
           field SETS, not positional field lists, mirroring the map order-independence case. MUST be
           true.")
  (input  (= (record (a 1) (b 2)) (record (b 2) (a 1))))
  (output (: true Bool)))

(case "projecting a field is independent of the order fields are written"
  (doc    "Member access finds a field by NAME, not by position, so `(. (record (b 2) (a 1)) a)`
           projects `a` = 1 even though `a` is written second. Pins that projection resolves the field
           name against the record's set, not by the order of construction — the projection companion of
           the order-independent equality case.")
  (input  (. (record (b 2) (a 1)) a))
  (output (: 1 Int64)))

; --- Member access chains: projecting a field of a projected record ----------------------
; core-semantics.md #Member Access Projects A Record Field applies to a record however it was
; obtained — including one that is itself the result of a projection. So `(. (. r outer) inner)`
; projects `outer` from `r` (yielding a record) then `inner` from that. The single-projection and
; conditionally-selected-record cases (above) do not witness a projection whose operand is another
; projection; this pins that a projection's result is an ordinary record that member access consumes.

(case "member access chains through a nested record"
  (doc    "`(. (. (record (outer (record (inner 7)))) outer) inner)` projects the inner record via
           `outer`, then `inner` from it, yielding 7. Pins that member access composes — the operand of
           a `.` may itself be a `.` projection, exactly as it may be a conditional-selected or
           function-returned record (witnessed elsewhere).")
  (input  (. (. (record (outer (record (inner 7)))) outer) inner))
  (output (: 7 Int64)))

(case "a field is projected off a record bound through a match arm"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field where the record reaches
           the `.` projection by being BOUND in a match arm — the payload of a `Some`. `mk` returns
           `(Some (record (a n) (b (+ n 1))))`, a genuine runtime record inside an Option; the match
           binds that record to `r` in the `Some` arm and `(. r b)` projects field `b`. With n=41 the
           record is `(record (a 41) (b 42))`, so `b` is 42. The record is not a compile-time literal
           and does not arrive through a `let` or a conditional — it arrives as a matched constructor
           payload, and the projection must still resolve the field, which requires the match binder to
           carry the payload's record shape to `r`. A binder that dropped the shape would leave `r`
           shapeless and the projection would have no slot to index.")
  (input  (do
            (def (mk n) (Some (record (a n) (b (+ n 1)))))
            (def (main)
              (match (mk 41)
                ((Some r) (. r b))
                ((None u) 0))) (export main)))
  (output (: 42 Int64)))

(case "a field is projected off a record unwrapped from an optional with expect"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field and #Requiring The Value
           Of An Optional Traps On Absence composed: the record reaches the `.` projection by being
           UNWRAPPED from an `Option` with `expect`, where the optional is produced by a FUNCTION CALL
           (not an inline literal). `mk` returns `(Some (record (a n) (b (+ n 1))))`; `(Option.expect
           (mk 41) \"x\")` unwraps the present optional to the record, and `(. … b)` projects field `b`
           = 42. This is the `expect`-unwrap companion of the match-arm binder case above: the value
           arrives through a DIFFERENT binding construct, and the projection must still resolve the
           field — the unwrap must carry the payload's record shape to the projected value. The
           call-produced scrutinee is the demanding form: an inline `(Some (record …))` literal or a
           `let`-bound optional carry the shape structurally, but a call return is a genuine runtime
           value whose shape must be threaded through the `expect` unwrap. A shape-dropping unwrap would
           leave the value slotless — historically this compiled to a VALID component that TRAPPED at
           run (a decline leaked past the emit retry into a runtime trap), which this case pins against.")
  (input  (do
            (def (mk n) (Some (record (a n) (b (+ n 1)))))
            (def (main) (. (Option.expect (mk 41) "x") b)) (export main)))
  (output (: 42 Int64)))

(case "a field is projected off a record unwrapped from a result with expect"
  (doc    "The Result twin of the Option.expect case above: the record reaches the `.` projection by
           being UNWRAPPED from a `Result` with `expect`, the Result produced by a FUNCTION CALL. `mk`
           returns `(Ok (record (a n) (b (+ n 1))))`; `(Result.expect (mk 41) \"x\")` unwraps the Ok to
           the record and `(. … b)` projects field `b` = 42 (the Err case would trap — expect is
           unwrap-or-trap). Same demand as the Option companion: the unwrap must carry the payload's
           record shape to the projected value through the call-produced scrutinee, or the projection
           has no slot to index. Pins that Result.expect — not only Option.expect — threads a compound
           payload's shape to a downstream field access, across the two-variant Result sum.")
  (input  (do
            (def (mk n) (Ok (record (a n) (b (+ n 1)))))
            (def (main) (. (Result.expect (mk 41) "x") b)) (export main)))
  (output (: 42 Int64)))

(case "a sum-type value is constructed through a variant"
  (doc    "Sign is declared where used as (Neg | Zero | Pos) (options/code-shape/); a value is one
           variant. Construction is via application: Sign.Pos is a Constructor (function), and
           (Sign.Pos unit) applies it to unit, producing the Sum value.")
  (input  (let ((s (Sign.Pos unit))) s))
  (output (: (Sign.Pos unit) Sign)))

; A compound value (tuple/record/sum) whose ELEMENT is a RUNTIME value — a function parameter, a
; call result — must be producible as a program RESULT, not only projectable. The value crosses the
; run boundary through the resource-with-display output ABI; a generation that does not yet render a
; runtime-element compound DECLINES rather than miscompiling. The control below is the compile-time-
; known compound, which must reach the same result.

(case "a tuple with a runtime element is returned as a program result"
  (doc    "`f` returns `(tuple n 1)` where `n` is its parameter (a runtime value); `(f 3)` produces
           `(tuple 3 1)`, which must cross the run boundary as the program's result. A well-typed
           program: the compiler evaluates it to the tuple, or declines if it does not yet render a
           compound carrying a runtime element (reject-don't-miscompile). The constant control below is
           the same value known at compile time.")
  (input  (do
            (def (f n) (tuple n 1))
            (def (main) (f 3)) (export main)))
  (output (: (tuple 3 1) (Tuple Int64 Int64))))

(case "a constant tuple is returned as a program result"
  (doc    "The control the case above must match: a compile-time-known `(tuple 3 1)` returns fine
           through the resource-with-display output ABI. The runtime-element tuple must reach the same
           result; the difference is only whether an element is known at compile time.")
  (input  (do
            (def (main) (tuple 3 1)) (export main)))
  (output (: (tuple 3 1) (Tuple Int64 Int64))))

; The `(tuple n 1)` case above carries ONE runtime element beside a constant. A runtime compound built
; on the heap must carry the general shape too: EVERY element computed at run time, MORE than two
; elements, a NON-Int64 element (mixed-type heap layout), a NESTED runtime compound (a heap cell holding
; a reference to another), and it must be PROJECTABLE after construction. These pin the runtime-compound
; construction path (the resource-with-display output ABI over a heap-allocated value) across the shapes
; the single-element case does not reach — each must produce its exact value, not a wrong element,
; truncated tuple, or misread layout.

(case "a tuple with every element computed at run time is returned as a result"
  (doc    "`(tuple (+ a 1) (* b 2))` — BOTH elements are runtime expressions, not one runtime and one
           constant. `(f 3 4)` produces `(tuple 4 8)`, which must cross the run boundary intact. Pins
           that the runtime-tuple construction handles all-computed elements, not only a single runtime
           element beside a literal (the `(tuple n 1)` case above).")
  (input  (do
            (def (f a b) (tuple (+ a 1) (* b 2)))
            (def (main) (f 3 4)) (export main)))
  (output (: (tuple 4 8) (Tuple Int64 Int64))))

(case "a three-element runtime tuple is returned as a result"
  (doc    "`(tuple n (+ n 1) (+ n 2))` with n=10 produces `(tuple 10 11 12)`. Pins that a runtime tuple
           of arity > 2 lays out and returns all its elements — a construction that hard-coded a pair
           would drop the third.")
  (input  (do
            (def (f n) (tuple n (+ n 1) (+ n 2)))
            (def (main) (f 10)) (export main)))
  (output (: (tuple 10 11 12) (Tuple Int64 Int64 Int64))))

(case "a runtime tuple with a boolean element is returned as a result"
  (doc    "`(tuple n (= n 0))` with n=0 produces `(tuple 0 true)` — a mixed Int64/Bool runtime tuple. A
           tuple element's type is whatever that position holds, so the heap layout must carry a Bool
           beside an Int64 and render both across the run boundary (the runtime companion of the
           constant boolean-tuple-element case). Pins that a runtime compound is not uniformly Int64.")
  (input  (do
            (def (f n) (tuple n (= n 0)))
            (def (main) (f 0)) (export main)))
  (output (: (tuple 0 true) (Tuple Int64 Bool))))

(case "a nested runtime tuple is returned as a result"
  (doc    "`(tuple n (tuple n n))` with n=2 produces `(tuple 2 (tuple 2 2))` — a runtime tuple whose
           element is itself a runtime tuple (a heap cell referencing another). Pins that runtime
           compound construction nests: an inner heap value is built and referenced by the outer, and
           the whole structure renders across the run boundary.")
  (input  (do
            (def (f n) (tuple n (tuple n n)))
            (def (main) (f 2)) (export main)))
  (output (: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "an element is projected from a runtime-constructed tuple"
  (doc    "`(. (tuple n (+ n 1)) 1)` with n=5 projects element 1 of a runtime-built tuple, yielding
           6. Pins that positional access reads the correct element of a heap-allocated runtime tuple —
           a layout or offset error would return the wrong element (5) or a garbage value. Companion of
           the constant tuple-access cases, on the runtime construction path.")
  (input  (do
            (def (f n) (. (tuple n (+ n 1)) 1))
            (def (main) (f 5)) (export main)))
  (output (: 6 Int64)))

; A NARROW-width element crosses the heap boundary through an explicit slot conversion: the heap stores
; an integer as one i64 cell (`box-int`/`get-int` are i64), but a narrow width (Int8/16/32, UInt8) lives
; in an i32 machine slot. So a narrow element is EXTENDED i32→i64 on the way into the heap and NARROWED
; i64→i32 on the way out — otherwise the emitted `box-int`/op has a mismatched operand slot and the
; component fails to validate. These pin narrow-width elements built into and projected out of a runtime
; tuple/record, combined by an op (the two-projection shape a single-projection case cannot witness).

(case "two narrow tuple elements are projected and added"
  (doc    "`(let ((t (tuple a b))) (+ (. t 0) (. t 1)))` with `a,b : UInt8` = 100+50 = 150. Both operands
           are narrow elements read back from the heap tuple; each crosses as an i64 cell and is narrowed
           to its UInt8 (i32) slot before the `+`, so the op's operands share the narrow slot — not a
           mismatched i64/i32 the component would reject. A single projection (already witnessed above)
           does not exercise two heap reads feeding one op.")
  (input  (do (def (main (: a UInt8) (: b UInt8)) (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (export main)))
  (call   main (: 100 UInt8) (: 50 UInt8))
  (output (: 150 UInt8)))

(case "two narrow tuple elements are projected and compared"
  (doc    "The comparison face: `(> (. t 0) (. t 1))` with `a,b : UInt8`, 100 > 50 = true. Every binary op
           over two narrow heap projections — not only `+` — needs each element narrowed to its slot.")
  (input  (do (def (main (: a UInt8) (: b UInt8)) (let ((t (tuple a b))) (> (. t 0) (. t 1)))) (export main)))
  (call   main (: 100 UInt8) (: 50 UInt8))
  (output (: true Bool)))

(case "a signed narrow tuple element round-trips through the heap with its sign"
  (doc    "`(+ (. t 0) (. t 1))` with `a,b : Int8`, a = -5, b = 3 → -2. A signed narrow element is
           sign-extended i32→i64 into the heap cell and its low bits narrowed back, so a negative value
           survives the round-trip (a zero-extend would read -5 as 251 and give the wrong sum).")
  (input  (do (def (main (: a Int8) (: b Int8)) (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (export main)))
  (call   main (: -5 Int8) (: 3 Int8))
  (output (: -2 Int8)))

(case "two narrow record fields are projected and added"
  (doc    "The record analogue: `(let ((r (record (x a) (y b)))) (+ (. r x) (. r y)))` with `a,b : UInt8`
           = 150. A record is the same positional heap array as a tuple, so a narrow field crosses the
           heap boundary with the same i32↔i64 slot conversion — the fix is on the heap box/unbox edge,
           not tuple- or record-specific.")
  (input  (do (def (main (: a UInt8) (: b UInt8)) (let ((r (record (x a) (y b)))) (+ (. r x) (. r y)))) (export main)))
  (call   main (: 100 UInt8) (: 50 UInt8))
  (output (: 150 UInt8)))

(case "a runtime tuple built behind a recursive call escapes to the host"
  (doc    "A tuple returned from a RECURSIVE function that threads a runtime value into it —
           `(f 3)` recurses down to `(f 0)`, which builds `(tuple n 7)` with n=0 → `(tuple 0 7)`. The
           element `n` is a genuine runtime value carried through the recursion (it does NOT constant-
           fold, unlike a literal-only tuple), so the compound is built on the value heap and CROSSES
           the host boundary as a resource whose `encode()` walks the live handle
           (component-abi.md §A Compound Result Is Rendered By Compiler-Emitted Code). Pins the genuine
           heap-alloc → escape → walk round-trip: a fold-only path would never touch the runtime.")
  (input  (do
            (def (f n) (if (= n 0) (tuple n 7) (f (- n 1))))
            (def (main) (f 3)) (export main)))
  (output (: (tuple 0 7) (Tuple Int64 Int64))))

(case "a runtime record built behind a recursive call escapes to the host"
  (doc    "The record companion of the recursive-tuple escape: `(f 3)` recurses to `(f 0)`, which builds
           `(record (a n) (b 7))` with n=0 → `(record (a 0) (b 7))`. A record is the SAME positional heap
           array as a tuple (fields in canonical sorted order); the runtime holds a nameless array and the
           compiler-emitted renderer bakes the field names from the static type
           (component-abi.md §The Runtime Does Not Name Or Render Values). Recursive → not folded → a
           genuine heap value that crosses the host boundary as a resource whose `encode()` walks it.")
  (input  (do
            (def (f n) (if (= n 0) (record (a n) (b 7)) (f (- n 1))))
            (def (main) (f 3)) (export main)))
  (output (: (record (a 0) (b 7)) (Record (a Int64) (b Int64)))))

(case "a nested runtime tuple built behind a recursive call escapes to the host"
  (doc    "`(tuple n (tuple n n))` with n=0 (reached via recursion so it does NOT constant-fold) →
           `(tuple 0 (tuple 0 0))`. The INNER tuple is built on the value heap as its own array; the
           OUTER tuple stores the inner HANDLE directly (a nested compound element is already a handle —
           NOT boxed like a scalar). `encode()` walks the nested `arr-get` path and renders both levels
           (component-abi.md §A Compound Result Is Rendered By Compiler-Emitted Code). Pins that a
           runtime compound NESTS: a heap value referencing another heap value crosses the boundary.")
  (input  (do
            (def (f n) (if (= n 0) (tuple n (tuple n n)) (f (- n 1))))
            (def (main) (f 2)) (export main)))
  (output (: (tuple 0 (tuple 0 0)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "a runtime record whose field is a runtime tuple escapes to the host"
  (doc    "`(record (x n) (y (tuple n 1)))` with n=0 (recursion, unfoldable) →
           `(record (x 0) (y (tuple 0 1)))` — a record field that is itself a runtime compound. Pins
           that the type-directed renderer recurses through a HETEROGENEOUS nesting (record → tuple),
           dispatching each sub-shape to its own head; the field holds the inner tuple's handle.")
  (input  (do
            (def (f n) (if (= n 0) (record (x n) (y (tuple n 1))) (f (- n 1))))
            (def (main) (f 2)) (export main)))
  (output (: (record (x 0) (y (tuple 0 1))) (Record (x Int64) (y (Tuple Int64 Int64))))))

; --- Runtime RECORD and LIST results (the same positional heap array as a tuple) ----------
; A record and a list carrying a runtime element are, at run time, the SAME positional heap
; array a tuple is — field names and the tuple/list/record distinction are static type
; information the compiler holds and the (name-free, tag-free) runtime does not. The compiler
; emits a TYPE-DIRECTED renderer that walks the array through the runtime's accessors and bakes
; the right keyword/names, so `(record (a 3) (b 1))`, `(list 1 2 3)` render distinctly from the
; identical underlying array (component-abi.md §A Compound Result Is Rendered By Compiler-Emitted
; Code; §The Runtime Does Not Name Or Render Values).

(case "a record with a runtime field is returned as a program result"
  (doc    "`(record (a n) (b 1))` with n=3 produces `(record (a 3) (b 1))` — a record one of whose
           fields is a runtime value. Pins that a record is constructed on the value heap as a
           positional product and rendered with its field NAMES (which the runtime does not hold —
           the compiler-emitted renderer bakes them from the static type). Companion of the runtime
           tuple result, distinguished only by its static type / rendering.")
  (input  (do
            (def (f n) (record (a n) (b 1)))
            (def (main) (f 3)) (export main)))
  (output (: (record (a 3) (b 1)) (Record (a Int64) (b Int64)))))

(case "record fields render in canonical (key-sorted) order regardless of source order"
  (doc    "`(record (b n) (a 1))` with n=2 renders `(record (a 1) (b 2))` — fields in sorted key
           order, not source order (deterministic-value-form.md). Pins that the runtime array slots
           and the emitted renderer AGREE on the sorted field order, so a field value lands under its
           correct name; a slot/name misalignment would render `(record (a 2) (b 1))`.")
  (input  (do
            (def (f n) (record (b n) (a 1)))
            (def (main) (f 2)) (export main)))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "a list with a runtime element is returned as a program result"
  (doc    "`(list n 2 3)` with n=1 produces `(list 1 2 3)` — a list one of whose elements is a runtime
           value. Pins that a list is constructed on the value heap and rendered `(list …)`,
           distinct from a tuple's `(tuple …)` though the underlying heap array is identical (the
           distinction is the static type the renderer walks).")
  (input  (do
            (def (f n) (list n 2 3))
            (def (main) (f 1)) (export main)))
  (output (: (list 1 2 3) (List Int64))))

(case "two lists are concatenated into one flat list"
  (doc    "`(List.concat (list 1 2) (list 3 4))` produces `(list 1 2 3 4)` — the elements of the first
           list in order followed by those of the second (collections-and-text.md §A List Is Grown By
           Functional Construction, the concatenation clause). Pins that a concatenated list is the
           SAME `List` type as a literal — its representation (an RRB persistent tree join) is
           unobservable (#A List's Representation Is Unspecified And Unobservable), so it renders as one
           flat `(list …)`. The self-hosting compiler assembles output in linear time with this op.")
  (needs      collections)
  (input  (do (def (main) (List.concat (list 1 2) (list 3 4))) (export main)))
  (output (: (list 1 2 3 4) (List Int64))))

(case "the length of a concatenation is the sum of the two lengths"
  (doc    "`(List.len (List.concat (list 1 2 3) (list 4 5)))` = 5 — concatenation appends every element
           of the second list to the first, so the result length is the sum. Reads through the joined
           trie via `vec-len` exactly as a push-built list (collections-and-text.md §A List's
           Representation Is Unspecified And Unobservable).")
  (needs      collections)
  (input  (do (def (main) (List.len (List.concat (list 1 2 3) (list 4 5)))) (export main)))
  (output (: 5 Int64)))

(case "an element read across a concatenation boundary is the right value"
  (doc    "`(Option.expect (List.at (List.concat (list 10 20 30) (list 40 50)) 3) …)` = 40 — index 3 of
           the concatenation is the FIRST element of the second operand, so the join places the second
           list's elements immediately after the first's. Pins that `List.at` reads a concatenated list
           by the same total ordering as a literal.")
  (needs      fallible-access)
  (input  (do
            (def (main)
              (Option.expect (List.at (List.concat (list 10 20 30) (list 40 50)) 3) "in bounds")) (export main)))
  (output (: 40 Int64)))

(case "concatenating with the empty list on the right is identity"
  (doc    "`(List.len (List.concat (list 7 8 9) (list)))` = 3 — concatenating with the empty list yields
           a list equal to the other operand (collections-and-text.md §A List Is Grown By Functional
           Construction: the empty-operand identity). The empty right operand contributes no elements.")
  (needs      collections)
  (input  (do (def (main) (List.len (List.concat (list 7 8 9) (list)))) (export main)))
  (output (: 3 Int64)))

(case "a list-concatenating helper threads lists through a call"
  (doc    "`(def (cat a b) (List.concat a b))` applied to two literals — concatenation works on list
           PARAMETERS, not only inline literals, so both operands are inferred `Heap` and the helper
           emits a runtime `vec-concat`. This is the `code-cat`/emit-assembly idiom a self-hosted
           compiler is written in: joining encoded fragments in linear time rather than pushing one
           element at a time (O(n²)).")
  (needs      collections)
  (input  (do
            (def (cat a b) (List.concat a b))
            (def (main) (List.len (cat (list 1 2 3) (list 4 5 6 7)))) (export main)))
  (output (: 7 Int64)))

(case "concatenating lists of different element types is a type error"
  (doc    "`(List.concat (list 1 2) (list true))` joins an `Int64` list with a `Bool` list — but a list
           has ONE element type (collections-and-text.md §A List Is An Ordered Homogeneous Sequence),
           so a concatenation of two differently-typed lists has no well-typed result and is rejected
           (CDZ0201). A generation that skipped this would render the result at one operand's element
           type, mistyping the other's elements — a wrong value, not merely a missing rejection.")
  (needs      collections)
  (input  (do (def (main) (List.len (List.concat (list 1 2) (list true)))) (export main)))
  (error CDZ0201))

(case "a record whose field is a runtime tuple nests across the boundary"
  (doc    "`(record (x n) (y (tuple n 1)))` with n=5 produces `(record (x 5) (y (tuple 5 1)))` — a
           record field that is itself a runtime compound. Pins that the type-directed renderer
           recurses through a heterogeneous nesting (record → tuple), dispatching each sub-shape to
           its own renderer.")
  (input  (do
            (def (f n) (record (x n) (y (tuple n 1))))
            (def (main) (f 5)) (export main)))
  (output (: (record (x 5) (y (tuple 5 1))) (Record (x Int64) (y (Tuple Int64 Int64))))))

; --- A constructor whose payload is itself a constructor keeps its own variant tag -------
; A Sum value is (variant-tag, payload); its canonical form is `(Variant payload)`
; (deterministic-value-form.md; core-semantics.md #A Constructor Applied To An Argument Is A Sum
; Value). This holds regardless of what the payload IS — including another Sum value. So
; `(Some (Some 5))` is a Some whose payload is a Some, and its canonical form is `(Some (Some 5))`,
; with BOTH variant tags present through construction and serialization.

(case "a constructor whose payload is a constructor keeps the outer variant tag"
  (doc    "`(Some (Some 5))` is an Option whose payload is an Option — a Some carrying a Some. Its
           canonical value form is `(Some (Some 5))`, with BOTH variant tags present. Pins that the
           outer `Some` survives construction and serialization when its payload is itself a Sum.")
  (input  (Some (Some 5)))
  (output (: (Some (Some 5)) (Option (Option Int64)))))

(case "Ok carrying a Some keeps both variant tags"
  (doc    "The Result-of-Option companion: `(Ok (Some 3))` is an Ok whose payload is a Some. Its
           canonical form is `(Ok (Some 3))`; a constructor's own tag is not replaced by `record`
           because its payload is a Sum — both tags survive.")
  (input  (Ok (Some 3)))
  (output (: (Ok (Some 3)) (Result (Option Int64) Int64))))

(case "a nested constructor value dispatches on both tags in a match"
  (doc    "The companion proving the nested Sum value dispatches correctly: matching `(Some (Some 5))`
           on the outer `Some` binds the inner `(Some 5)`, and a nested match on that binds y=5. Both
           variant tags are present for dispatch.")
  (input  (match (Some (Some 5))
            ((Some inner) (match inner ((Some y) y) ((None _) 0)))
            ((None _)     -1)))
  (output (: 5 Int64)))

; --- Runtime SUM results (a constructor applied to a RUNTIME payload, returned as the result) ---
; A Sum value is a (variant, payload); its canonical form is `(Variant payload)`
; (deterministic-value-form.md). The const cases above fold to that text; these build the SAME
; value at RUN TIME (the payload is a function parameter, not a constant), so the value lives on
; the heap as a (discriminant, payload-handle) and the compiler-emitted renderer walks it —
; switching on the runtime discriminant to write the correct variant name, then rendering the
; payload — to reproduce the identical canonical text. Companion of the runtime tuple/record/list
; results above, on the sum shape. The rendering is already pinned (it must match the const form),
; so these record the correct oracle for the runtime-sum construction path.

(case "a unary constructor applied to a runtime value is returned as a program result"
  (doc    "`(Some n)` with n=42 a runtime parameter produces `(Some 42)` — a Sum built at run time
           whose payload is a runtime value. Pins that a runtime sum is constructed on the value heap
           (a discriminant plus the boxed payload) and rendered `(Some 42)`, identical to the constant
           `(Some 42)` form; the renderer switches on the runtime discriminant to recover the variant
           name `Some` (which the tag-free runtime does not hold).")
  (input  (do
            (def (f n) (Some n))
            (def (main) (f 42)) (export main)))
  (output (: (Some 42) (Option Int64))))

(case "a conditionally-selected variant is returned as a runtime sum result"
  (doc    "`(if (= n 0) (None unit) (Some n))` with n=5 produces `(Some 5)` — the branch selects which
           variant is built at run time, so the discriminant is genuinely runtime data. Pins that the
           renderer's discriminant switch recovers the correct variant name for whichever arm ran; the
           n=0 companion below takes the `None` arm.")
  (input  (do
            (def (f n) (if (= n 0) (None unit) (Some n)))
            (def (main) (f 5)) (export main)))
  (output (: (Some 5) (Option Int64))))

(case "a conditionally-selected nullary variant is returned as a runtime sum result"
  (doc    "The `None` companion of the case above: with n=0 the branch selects `(None unit)`, whose
           canonical form is `(None unit)` (a nullary variant carries the unit value). Pins that the
           runtime discriminant switch renders a nullary variant's name and its unit payload correctly,
           distinct from the `Some` arm.")
  (input  (do
            (def (f n) (if (= n 0) (None unit) (Some n)))
            (def (main) (f 0)) (export main)))
  (output (: (None unit) (Option Any))))

(case "a runtime sum whose payload is a runtime tuple nests across the boundary"
  (doc    "`(Some (tuple n 1))` with n=7 produces `(Some (tuple 7 1))` — a runtime sum carrying a
           runtime compound payload. Pins that the type-directed renderer recurses from the sum's
           payload into the tuple renderer, dispatching each sub-shape, and that construction nests a
           heap tuple inside a heap sum.")
  (input  (do
            (def (f n) (Some (tuple n 1)))
            (def (main) (f 7)) (export main)))
  (output (: (Some (tuple 7 1)) (Option (Tuple Int64 Int64)))))

(case "a built-in Option is unwrapped by a helper that binds its payload"
  (doc    "A helper takes an `Option Int64` as a PARAMETER and matches it, binding and returning the
           payload in the `Some` arm and a default in the `None` arm — the idiom for consuming a
           fallible result (`Bytes.at`, `String.from-bytes`, map `get`) that a self-hosted reader is
           written in. `(unwrap (Some 42) 99)` binds x=42 and returns 42. Pins that the BUILT-IN
           polymorphic `Option` supports a payload-binding match across a function boundary, the same
           way a user-declared sum does (see §\"a match arm binds a nested tuple inside a sum payload\"
           and the `Box (Full Int64 | Empty)` shape, which compiles). Distinct from the runtime-Option
           cases above, which only CONSTRUCT and return an Option; this one CONSUMES one whose payload
           kind must be recovered after it crossed a parameter boundary. Matching the same Option at the
           entrypoint directly compiles; only passing it into a helper and binding the payload does not
           yet, so this case guards that the built-in Option's payload kind survives the boundary as a
           user sum's does.")
  (needs  fallible-access)
  (input  (do
            (def (unwrap o d) (match o ((Some x) x) ((None _) d)))
            (def (main) (unwrap (Some 42) 99)) (export main)))
  (output (: 42 Int64)))

(case "a generic unwrap helper consumes a fallible Bytes.at result"
  (doc    "The reader idiom in full: a GENERIC `unwrap` helper (taking an `Option Int64` PARAMETER)
           consumes the fallible `(Bytes.at b i)` result — the shape a byte-walking reader is written
           in, where the producer of the Option and the consumer are different functions. `at` passes
           `(Bytes.at b 1)` (= `(Some 20)`) into `unwrap` with default -1; the payload kind (Int64)
           must be recovered after the Option crossed BOTH the `Bytes.at` producer boundary AND the
           `unwrap` parameter boundary. Pins that a fallible runtime result flows into a generic
           consumer helper — earlier this declined 'arms differ in kind' because the `Some` binder
           bound as an opaque handle across the parameter boundary; now the arm-unification recovers
           it. `(at (Bytes.of (list 10 20 30)) 1)` = 20.")
  (needs  fallible-access)
  (input  (do
            (def (unwrap o d) (match o ((Some x) x) ((None _) d)))
            (def (at b i)     (unwrap (Bytes.at b i) -1))
            (def (main)       (at (Bytes.of (list 10 20 30)) 1)) (export main)))
  (output (: 20 Int64)))

; The payload type of a fallible `List.at` result must flow through `Option.expect` into ARITHMETIC, just
; as it does for `Bytes.at` and just as the match-unwrap form does for `List.at`. `(List.at xs i)` on a
; `List Int64` is `Option Int64`, so `(Option.expect (List.at xs i) msg)` is an Int64 and adding to it is
; well-typed. The seed declines "non-integer operand to arithmetic" for a RUNTIME list (a parameter, or a
; literal with a computed element) — it does not resolve `Option.expect (List.at <runtime-list> i)` to
; Int64 for the arithmetic-operand check — yet the SAME idiom works for `Bytes.at` (`(+ (Option.expect
; (Bytes.at b 1) msg) 10)` = 30), the match-unwrap form works for `List.at` (`(+ (match (List.at xs 1)
; ((Some x) x) ((None _) 0)) 10)` = 30), and `Option.expect (List.at …)` used NON-arithmetically (returned,
; or matched) works (→20). Only `Option.expect` on a `List.at` result, used as an arithmetic operand on a
; runtime list, loses the Int64 element type. It is decline-don't-miscompile-safe (an honest decline, no
; wrong value), so a generation that does not yet propagate a runtime list's element type through
; `Option.expect` into arithmetic declines rather than miscompiling. It is the list-indexing reader idiom a
; self-hosted compiler is written in.

(case "an expect of a list index result is an integer usable in arithmetic"
  (doc    "`(List.at xs i)` on a `List Int64` is `Option Int64`, so `(Option.expect (List.at xs 1) msg)` is
           an Int64 and `(+ … 10)` is well-typed; for `xs` = `(list 10 20 30)`, element 1 is 20, so the sum
           is 30. Pins that a runtime list's element type flows through `Option.expect` into arithmetic —
           the list-indexing reader idiom. The seed declines 'non-integer operand to arithmetic' for a
           runtime list (here a parameter), not resolving the expect result to Int64 for the `+` operand,
           though the same idiom works for `Bytes.at` (§'a generic unwrap helper consumes a fallible
           Bytes.at result' above), the match-unwrap form works for `List.at`, and the expect result works
           when returned or matched (not added). A generation that does not yet propagate a runtime list's
           element type through `Option.expect` into arithmetic declines rather than miscompiling.")
  (needs  fallible-access)
  (input  (do
            (def (f xs) (+ (Option.expect (List.at xs 1) "in bounds") 10))
            (def (main) (f (list 10 20 30))) (export main)))
  (output (: 30 Int64)))

; The arm-unification that recovers a payload kind across branches must reach a NESTED-Option arm against
; a None arm, not only a single-level Some against a None. A function whose branches return `(None unit)`
; and `(Some (Some n))` produces an `Option (Option Int64)` — a valid type — but the two arms differ in
; payload KIND (the `None` arm carries Unit; the `Some` arm carries a nested `Option Int64`), and the
; compound-result-shape inference does not unify them: returning that value as the program result declines
; "cannot infer runtime compound result shape". The single-level analogue works — a function returning
; `(None unit)` vs `(Some n)` yields `(Some 5)` (its arms' kinds DO unify at the result boundary) — and a
; nested producer whose BOTH arms are `(Some (Some …))` works (`(Some (Some 5))`, consistent kind). Only a
; None arm paired with a NESTED-Some arm is not yet unified. The value is unambiguous — consuming the same
; producer with a nested match `((Some (Some x)) …)` reconstructs `(Some (Some 5))` — so the program is
; valid; the compound-result-shape inference just does not yet recover the nested-vs-nullary arm kind. This
; is the nested-payload sibling of the fallible-result arm-unification cases above (which recover a
; single-level Some payload across a boundary). A generation that does not yet unify a None arm with a
; nested-Some arm at the result boundary declines rather than miscompiling.

(case "a function returning None or a nested Some infers its compound result shape"
  (doc    "`cl` returns `(None unit)` for a negative input and `(Some (Some n))` otherwise — an `Option
           (Option Int64)`, whose value for `(cl 5)` is `(Some (Some 5))`. Returning it as the program
           result requires unifying the `None` arm's payload kind (Unit) with the `Some` arm's nested
           payload kind (`Option Int64`) at the compound-result boundary. The single-level analogue (`None`
           vs `(Some n)`) infers fine and yields `(Some 5)`, and a producer whose both arms are `(Some
           (Some …))` infers fine and yields `(Some (Some 5))` — only a None arm paired with a NESTED-Some
           arm is not yet unified, declining \"cannot infer runtime compound result shape\". The value is
           unambiguous (consuming `cl` with a nested `(Some (Some x))` match reconstructs `(Some (Some
           5))`), so the program is valid; this pins that the arm-kind unification recovering a payload
           kind across branches reaches a nested-Option arm against a None arm, the nested-payload sibling
           of the single-level fallible-result unification above. A generation that does not yet unify a
           None arm with a nested-Some arm at the result boundary declines rather than miscompiling.")
  (needs  fallible-access)
  (input  (do
            (def (cl n)   (if (< n 0) (None unit) (Some (Some n))))
            (def (main)   (cl 5)) (export main)))
  (output (: (Some (Some 5)) (Option (Option Int64)))))

(case "a recursive function folds a runtime linked list to a scalar"
  (doc    "The consumption half of the recursive-sum idiom, and the core shape a self-hosted compiler is
           written in: a recursive function CONSUMES a runtime linked list (a user recursive sum) by
           matching its variants and returns a SCALAR. `sm` sums the elements: `sm(Cons(5, Cons(8,
           Cons(2, Nil))))` = 15. The list is built from a runtime value (so it lives on the value heap,
           not folded), and `sm` recurses through `IntList.Cons`/`IntList.Nil` at run time, binding the
           head and tail from each Cons node's payload tuple. Pins that a program whose RESULT is a
           scalar it folded out of a runtime heap value compiles and runs — the `map`/`fold`-over-nodes
           idiom the compiler leans on, distinct from a program whose result IS the heap value.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (sm xs) (match xs
                           ((IntList.Cons (tuple h t)) (+ h (sm t)))
                           ((IntList.Nil _)            0)))
            (def (build n) (IntList.Cons (tuple n (IntList.Cons (tuple 8 (IntList.Cons (tuple 2 (IntList.Nil ()))))))))
            (def (main) (sm (build 5))) (export main)))
  (output (: 15 Int64)))

(case "a recursive function folds a linked list of runtime-determined length"
  (doc    "The genuine self-hosting idiom, distinct from the case above: the list's LENGTH is decided at
           run time, not by a fixed literal spine. `count` builds a descending list `[n, n-1, …, 1]` by
           recursing on `(- n 1)` until `(< n 1)` — so how many `Cons` nodes exist is known only at run
           time. `sm` then folds it, dispatching on the runtime discriminant of each node it did not
           construct at compile time. `sm(count 5)` = 5+4+3+2+1 = 15. This is exactly the shape a
           self-hosted compiler is written in — `(match node ((Ast.App …) …) ((Ast.Lam …) …))` where the
           node came from a reader and its variant is known only at run time — and it CANNOT be resolved
           by compile-time spine unrolling (the case above can). Pins runtime sum-match CONSUMPTION:
           `sum-disc` selects the arm, `sum-payload`/`arr-get` bind the head and tail, and the recursive
           call is a real runtime `call` rather than an unbounded compile-time inline.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (count n) (if (< n 1)
                               (IntList.Nil ())
                               (IntList.Cons (tuple n (count (- n 1))))))
            (def (sm xs) (match xs
                           ((IntList.Cons (tuple h t)) (+ h (sm t)))
                           ((IntList.Nil _)            0)))
            (def (main) (sm (count 5))) (export main)))
  (output (: 15 Int64)))

(case "a sum-match recursion that accumulates a built-in list returns a list"
  (doc    "The ACCUMULATOR companion of the fold above, and the shape a compiler's per-function
           return-kind table takes: `recompute` recurses by `match`-destructuring a user-sum parameter
           (`FL`) and push-accumulates a BUILT-IN `list` in its other parameter `out`, returning `out`
           in the base arm. `(List.len (recompute <2-node FL> (list)))` = 2 — two `List.push`es. Pins
           that the accumulator parameter AND the function's return both converge to the list (heap)
           kind even though the base arm `((FL.FNil _) out)` returns the accumulator bare and the
           recursive arm is a bare self-call — neither match arm independently reports the heap kind on
           the first inference pass. Without unifying the arms to the heap kind, the return locks to
           Int64 while `out` becomes heap, and `List.len` on the result declines 'of a non-list value'
           (the match-form twin of the if-form accumulator case in 13-strings; a compiler that walks a
           function list building a return-kind table is exactly this shape).")
  (needs  sum-type-declaration)
  (input  (do
            (type FL FNil (FCons (Tuple Int64 FL)))
            (def (recompute funcs out)
              (match funcs
                ((FL.FNil _)            out)
                ((FL.FCons (tuple h t)) (recompute t (List.push out h)))))
            (def (main) (List.len (recompute (FL.FCons (tuple 5 (FL.FCons (tuple 6 (FL.FNil ()))))) (list)))) (export main)))
  (output (: 2 Int64)))

(case "a fixpoint loop that threads a growing list accumulator returns that list"
  (doc    "The self-hosting return-kind machinery next needs a monotone FIXPOINT loop — `iterate` a
           table until it stops changing — and the boundary of what compiles is narrow. `loop`
           recurses on a counter and THREADS its list parameter `xs`, growing it by `List.push` each
           round, then the result is consumed as a list: `(List.len (loop (list 1 2 3) 2))` = 5 (three
           seed elements plus two pushes). This pins that a list-typed parameter carried THROUGH a
           recursive fixpoint — derived from the incoming value, even when mutated by `List.push` — and
           returned as a list, converges to the heap kind and compiles. The trigger for the still-open
           blowup (SPEC-BACKLOG) is strictly narrower than a fixpoint loop per se: it needs the list
           parameter to be RE-SEEDED with a fresh `(list …)` each round (NOT derived from the incoming
           value) AND the result consumed as a list; threading the incoming list — this case — is the
           passing side of that boundary. (The fresh-re-seed-plus-list-result conjunction, once the
           open blowup, now also compiles — see the case below.)")
  (input  (do
            (def (loop xs passes)
              (if (< passes 1) xs (loop (List.push xs 9) (- passes 1))))
            (def (main) (List.len (loop (list 1 2 3) 2))) (export main)))
  (output (: 5 Int64)))

(case "a fixpoint that re-seeds a fresh list each round via a helper returns a list"
  (doc    "The full monotone-fixpoint shape a self-hosted compiler's return-kind table takes, and the
           narrow conjunction that used to blow the compiler up (OOM): `iterate` recurses on a counter
           but RE-SEEDS its `ktab` parameter with a FRESHLY-BUILT list each round — `(recompute funcs
           (list))`, NOT derived from the incoming `ktab` — and the result is consumed as a list. Here
           `recompute` rebuilds a one-element list from a one-node `FL` each pass, so `(List.len (iterate
           <1-node FL> (list) 2))` = 1. Pins that a fixpoint whose list accumulator is re-seeded by a
           HELPER CALL (not threaded, not a direct `List.push` of the incoming value) still converges
           BOTH the callee's re-seeded parameter AND the function's return to the heap kind. The trap it
           closes: the argument `(recompute funcs (list))` is heap, but `iterate`'s `ktab` parameter —
           only returned in the base branch and re-passed in the recursive branch, never used in a
           kind-forcing op — defaulted to Int64, so the heap argument mismatched the Int64 parameter and
           the recursive `iterate` INLINED at compile time, re-expanding without bound (the compile-cost
           blowup). Propagating the argument's heap kind ONTO the callee's parameter (the arg → callee-
           param direction of unification) converges `ktab` to heap, so the call emits a real `call`, not
           an inline. This is the fresh-re-seed-plus-list-result form the case above noted as the open
           frontier — now representable.")
  (needs  sum-type-declaration)
  (input  (do
            (type FL FNil (FCons (Tuple Int64 FL)))
            (def (recompute funcs out)
              (match funcs
                ((FL.FNil _)            out)
                ((FL.FCons (tuple h t)) (recompute t (List.push out 7)))))
            (def (iterate funcs ktab passes)
              (if (< passes 1) ktab (iterate funcs (recompute funcs (list)) (- passes 1))))
            (def (main) (List.len (iterate (FL.FCons (tuple 1 (FL.FNil ()))) (list) 2))) (export main)))
  (output (: 1 Int64)))

(case "an element pattern matches a list by its length and elements"
  (doc    "A list is deconstructed by ELEMENT patterns — `(list)` matches exactly the empty list, a
           fixed-arity `(list a b)` matches a list of that exact length binding each position, and an
           element pattern MAY end in a rest binder `(list x .. rest)` that matches any list of at
           least the leading length, binding the leading positions and the rest as a `list`
           (core-semantics.md §A List Is Deconstructed By Element Patterns With An Optional Rest).
           This keeps the list's representation OPAQUE: the matcher observes only length and elements
           in order, never an internal cell/node structure — matching by elements, not by exposing a
           cons cell. Here the scrutinee is an inline list, so the whole match is decided at compile
           time; the recursive-fold-over-a-runtime-parameter form (below) additionally needs a
           materialized list tail for the rest binder.")
  (needs  list-patterns)
  (input  (do (def (main)
            (+ (match (list) ((list) 1) ((list a .. r) 2))
               (+ (match (list 7 8) ((list a b) (+ a b)) (_ 0))
                  (match (list 10 20 30) ((list) 0) ((list x .. rest) x))))) (export main)))
  (output (: 26 Int64)))

(case "the built-in list is folded by an element-with-rest pattern"
  (doc    "The natural fold over the BUILT-IN `list`, without hand-rolling a custom cons-sum. A list is
           deconstructed by ELEMENT patterns with an optional rest binder — `(list)` matches exactly the
           empty list, `(list x .. rest)` binds the first element `x` and the remaining elements as a
           list `rest` — so a total fold needs just those two arms (fixed-arity `(list x y)` are sugar
           for length checks). `sum (list 10 20 30)` = 10 + (20 + (30 + 0)) = 60. This keeps the list's
           representation OPAQUE (the matcher asks length/first/rest, not `Cons`/`Nil` cells — a `list` is
           a persistent tree, not a cons list), matching by elements the way ML/Rust do, not by exposing
           an internal cell structure. Every list-consuming pass a compiler writes — a module's def list,
           a call's argument list, a block's statements — is this fold; without it each must hand-roll a
           `(type FList (FNil | FCons …))` cons-sum that duplicates the sequence type the language already
           has (see the `IntList` fold above, which stands in precisely because the built-in `list` cannot
           yet be matched). The spec addition (`core-semantics.md` §Pattern Matching, list deconstruction)
           and the STATIC/const-fold lowering have landed; this RUNTIME form — `sum` recurses over its
           parameter `xs`, so the rest binder must materialize a list TAIL at run time — additionally
           needs a list-tail primitive, so it is gated behind `list-pattern-runtime-tail` until that
           lands (until then it declines \"runtime list element-pattern (rest binder) needs a list-tail
           primitive\").")
  (needs  list-pattern-runtime-tail)
  (input  (do
            (def (sum xs) (match xs
                            ((list)           0)
                            ((list x .. rest) (+ x (sum rest)))))
            (def (main) (sum (list 10 20 30))) (export main)))
  (output (: 60 Int64)))

(case "an expression tree built at run time is evaluated by matching its node variants"
  (doc    "The compiler's own expression-evaluator shape: a multi-variant recursive sum `Expr` — the
           canonical little AST — is built at run time by `build` (its structure decided by a runtime
           argument), then `ev` evaluates it by dispatching on each node's runtime variant. `Expr.Lit`
           carries an Int64 leaf; `Expr.Add`/`Expr.Mul` each carry a `(Tuple Expr Expr)` of two
           sub-expressions bound from the node's payload and evaluated recursively. `build 4` produces
           `Add(Lit 4, Add(Lit 3, Add(Lit 2, Add(Lit 1, Lit 2))))`, so `ev` yields 4+3+2+1+2 = 12. Pins
           that a THREE-variant recursive sum (not just a two-variant list) built at run time is
           consumed by runtime discriminant dispatch, binding a nested tuple payload of two heap
           sub-nodes and recursing into each — the exact `(match node ((Expr.Add …) …) …)` idiom a
           self-hosted evaluator/compiler is written in.")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64) (Add (Tuple Expr Expr)) (Mul (Tuple Expr Expr)))
            (def (ev e) (match e
                          ((Expr.Lit n)           n)
                          ((Expr.Add (tuple a b)) (+ (ev a) (ev b)))
                          ((Expr.Mul (tuple a b)) (* (ev a) (ev b)))))
            (def (build k) (if (< k 1)
                               (Expr.Lit 2)
                               (Expr.Add (tuple (Expr.Lit k) (build (- k 1))))))
            (def (main) (ev (build 4))) (export main)))
  (output (: 12 Int64)))

(case "a function returns a heap sub-node selected by a match arm"
  (doc    "A helper whose `match` arm yields a heap value bound by the pattern (a payload binder) — the
           tree-walker's hot path: a function that returns a SUB-NODE selected by matching its argument.
           `left` returns the whole `n`-leaf for a `Leaf`, and the first component `a` of the pair for a
           `Pair`; the result (a heap sub-node crossing the function-return boundary) is then matched
           again to fold to a scalar. Pins that constructing a runtime sum, and matching one, are both
           value-heap operations that engage the runtime path — a helper returning a bound sub-node must
           compile to a real function reading the heap, never emit heap-accessor calls into a scalar
           module with no such imports (which produced an invalid component). `left` of `Pair(Leaf 7,
           Leaf 9)` is `Leaf 7`, whose leaf value is 7.")
  (needs  sum-type-declaration)
  (input  (do
            (type T (Leaf Int64) (Pair (Tuple T T)))
            (def (left x) (match x ((T.Leaf n) (T.Leaf n)) ((T.Pair (tuple a b)) a)))
            (def (main) (match (left (T.Pair (tuple (T.Leaf 7) (T.Leaf 9))))
                          ((T.Leaf n) n)
                          ((T.Pair p) 0))) (export main)))
  (output (: 7 Int64)))

(case "a match arm binds a nested tuple inside a sum payload"
  (doc    "A `match` arm destructures a sum payload whose shape is a tuple nested inside a tuple —
           `(Bin (tuple op (tuple a b)))` — binding `op`, `a`, and `b` in one arm. This is the exact
           node a self-hosted compiler's `resolve`/`lower` passes take: a tagged node pairing a head
           opcode with a tuple of its two sub-operands, matched `((Expr.Bin (tuple op (tuple a b))) …)`
           and folded recursively. Pins that the payload binder recurses into a compound slot — a binder
           that is itself a `(tuple …)` reads that slot's heap handle and destructures it by the same
           slot logic — not only a flat payload tuple. The control cases already cover the flat binder
           (`(tuple a b)` in §\"a function returns a heap sub-node\") and a wide flat binder; only the
           NESTING is exercised here. `ev (Bin 0 (Lit 20) (Bin 1 (Lit 22) (Lit 8)))` computes
           `20 + (22 - 8) = 34`.")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64) (Bin (Tuple Int64 (Tuple Expr Expr))))
            (def (ev e) (match e
                          ((Expr.Lit n)                    n)
                          ((Expr.Bin (tuple op (tuple a b)))
                             (if (= op 0) (+ (ev a) (ev b)) (- (ev a) (ev b))))))
            (def (main) (ev (Expr.Bin (tuple 0
                                        (tuple (Expr.Lit 20)
                                               (Expr.Bin (tuple 1
                                                 (tuple (Expr.Lit 22) (Expr.Lit 8))))))))) (export main)))
  (output (: 34 Int64)))

(case "a constructor pattern nested under Some matches a runtime list element"
  (doc    "A `match` arm whose binder is ITSELF a constructor pattern nested under `Some` —
           `((Some (E.Lit n)) n)` — where the scrutinee is a fallible lookup `(List.at xs 0)` on a
           parameter list (a runtime value, not a compile-time constant). The reader's element walk
           takes exactly this shape: index a runtime sequence, and destructure the `Option`'s payload —
           itself a user-sum constructor — in one arm. Pins that a payload binder that is a CONSTRUCTOR
           pattern (not a bare name, not a tuple) recurses correctly: the `Some` payload's heap handle is
           materialized and matched against the inner `(E.Lit n)`, binding `n`. `(first-lit (list (E.Lit
           5)))` = 5. This is the ctor-under-Option companion of the nested-tuple-in-payload case above,
           and the shape a self-hosted compiler uses to read one element of a node list.")
  (needs  sum-type-declaration)
  (input  (do
            (type E (Lit Int64) (Neg Int64))
            (def (first-lit xs) (match (List.at xs 0)
                                  ((Some (E.Lit n)) n)
                                  ((None _)         0)))
            (def (main) (first-lit (list (E.Lit 5)))) (export main)))
  (output (: 5 Int64)))

(case "a recursive resolver transforms one runtime sum tree into another, then consumes it"
  (doc    "The compiler's reader→pipeline JOIN shape: a recursive function that transforms a runtime-built
           value of ONE sum type into a value of a DIFFERENT sum type, whose result is then consumed. A
           `Node` surface tree (head a String, resolved by name) is resolved to a typed `Core` tree —
           `resolve : Node → Core` maps `NInt→KConst` and dispatches an `NPrim`'s String head to the Core
           primitive constructor — and `eval : Core → Int64` folds the Core. Both are recursive walks over
           runtime heap values (the `Node` is built at run time, so `resolve`'s `Core` output is
           materialized at run time, not folded). `resolve (NPrim \"+\" (NInt 20) (NPrim \"*\" (NInt 2)
           (NInt 11)))` builds `KAdd(KConst 20, KMul(KConst 2, KConst 11))`; `eval` yields 20 + (2*11) =
           42. Pins that a Node→Core→scalar transform composes — a runtime sum consumed to build a
           DIFFERENT runtime sum, which is in turn consumed — the exact shape that joins a self-hosted
           reader to its resolved-IR pipeline (distinct from the `Expr` self-evaluator above, which stays
           within one type; here the transform crosses sum types, and the intermediate `Core` is a genuine
           runtime value the producer materializes and the consumer walks).")
  (needs  sum-type-declaration)
  (input  (do
            (type Node (NInt Int64) (NPrim (Tuple String Node Node)))
            (type Core (KConst Int64) (KAdd (Tuple Core Core)) (KSub (Tuple Core Core)) (KMul (Tuple Core Core)))
            (def (resolve n) (match n
                               ((Node.NInt v) (Core.KConst v))
                               ((Node.NPrim (tuple h a b))
                                  (if (= h "+") (Core.KAdd (tuple (resolve a) (resolve b)))
                                  (if (= h "-") (Core.KSub (tuple (resolve a) (resolve b)))
                                                (Core.KMul (tuple (resolve a) (resolve b))))))))
            (def (eval c) (match c
                            ((Core.KConst v) v)
                            ((Core.KAdd (tuple a b)) (+ (eval a) (eval b)))
                            ((Core.KSub (tuple a b)) (- (eval a) (eval b)))
                            ((Core.KMul (tuple a b)) (* (eval a) (eval b)))))
            (def (main) (eval (resolve (Node.NPrim (tuple "+"
                                          (Node.NInt 20)
                                          (Node.NPrim (tuple "*" (Node.NInt 2) (Node.NInt 11)))))))) (export main)))
  (output (: 42 Int64)))

(case "a recursive user sum type is built at run time and renders with qualified variant names"
  (doc    "A QUALIFIED-constructor recursive sum type — the linked-list / AST shape a self-hosted
           compiler manipulates — constructed at run time. `(IntList.Cons (tuple n (IntList.Nil ())))`
           with n=5 a runtime value produces `(IntList.Cons (tuple 5 (IntList.Nil unit)))`: a heap sum
           whose payload is a heap tuple whose second element is a nested heap sum. Pins that a
           QUALIFIED variant (`IntList.Cons`, not a bare `Some`) constructs at run time and the
           type-directed renderer reconstructs its qualified `Type.Variant` name from the sum type's
           declaration (the tag-free runtime holds only the discriminant), recursing through the
           tuple and the nested sum. This is the runtime construction half of the recursive-sum idiom
           the const case §\"a recursive sum type works with pattern matching\" folds; here the payload
           is a genuine runtime value so it lives on the value heap.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (f n) (IntList.Cons (tuple n (IntList.Nil ()))))
            (def (main) (f 5)) (export main)))
  (output (: (IntList.Cons (tuple 5 (IntList.Nil unit))) IntList)))

(case "a recursively-built linked list renders its full runtime spine"
  (doc    "The RENDER counterpart of the runtime-fold cases: a list whose spine is built by a
           self-recursive function (so its length is decided at run time) is returned as the program's
           RESULT and must render its complete structure — `count 3` yields
           `(IntList.Cons (tuple 3 (IntList.Cons (tuple 2 (IntList.Cons (tuple 1 (IntList.Nil unit)))))))`.
           A generation that cannot yet infer the static shape of a value of a RECURSIVE sum type (whose
           shape is unbounded — the tree-shaped renderer would need to walk to a runtime-determined
           depth) MUST decline rather than render a truncated or wrong structure. This is the render dual
           of §\"a recursive function folds a linked list of runtime-determined length\": consuming such a
           list to a scalar works, but rendering it as the boundary result is harder (an infinite static
           shape) and is not on the self-hosting critical path (a compiler returns bytes, not a rendered
           list). Pins decline-don't-miscompile: the wrong answer `(IntList.Cons 0)` — reading the Cons
           payload's tuple handle as a boxed integer — is a FAIL, never an accepted output.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (count n) (if (< n 1)
                               (IntList.Nil ())
                               (IntList.Cons (tuple n (count (- n 1))))))
            (def (main) (count 3)) (export main)))
  (output (: (IntList.Cons (tuple 3 (IntList.Cons (tuple 2 (IntList.Cons (tuple 1 (IntList.Nil unit))))))) IntList)))

(case "a recursively-built binary tree renders its full runtime structure"
  (doc    "The MULTI-WAY recursive counterpart of the linked-list spine: a `Tree` whose `Node` variant
           carries a `(Tuple Tree Tree)` — TWO recursive sub-references, not one — is built by a
           self-recursive `build` and returned as the program RESULT. `build 2` yields a balanced
           depth-2 tree; the renderer must walk BOTH sub-trees of every `Node` to their runtime depth,
           not just a single spine. This pins that a recursive-sum render fn recurses on EACH recursive
           payload position (a `Tree.Node`'s left and right), the render dual of a tree-consuming fold —
           a rendering that walked only one child, or truncated at a fixed depth, would produce a wrong
           structure (decline-don't-miscompile: the wrong shape is a FAIL, never an accepted output).")
  (needs  sum-type-declaration)
  (input  (do
            (type Tree (Leaf Int64) (Node (Tuple Tree Tree)))
            (def (build n) (if (< n 1)
                               (Tree.Leaf n)
                               (Tree.Node (tuple (build (- n 1)) (build (- n 1))))))
            (def (main) (build 2)) (export main)))
  (output (: (Tree.Node (tuple
                (Tree.Node (tuple (Tree.Leaf 0) (Tree.Leaf 0)))
                (Tree.Node (tuple (Tree.Leaf 0) (Tree.Leaf 0))))) Tree)))

; The case above dispatches a nested Sum by matching the outer variant then a SEPARATE inner match on
; the bound payload. A nested pattern deconstructs both tags in ONE arm — `(Ok (Ok n))` matches an Ok
; whose payload is an Ok, binding the innermost payload directly (02-binding "nested patterns
; deconstruct recursively", here across TWO sum types). These pin two-sum nesting the `(Some (Some …))`
; cases do not: Result-of-Result (same sum both levels, both inner arms reachable) and Option-of-Result
; (DIFFERENT sums nested), each a single arm carrying two constructor patterns.

(case "a Result carrying a Result is matched by a nested pattern to its inner value"
  (doc    "`(Ok (Ok 5))` is an Ok whose payload is an Ok; the nested pattern `(Ok (Ok n))` deconstructs
           both tags in one arm, binding n=5. Pins two-level Result nesting matched directly (not via a
           second `match`), the deep-pattern companion of the `(Ok (Some 3))` construction case above.")
  (input  (match (Ok (Ok 5))
            ((Ok (Ok n))  n)
            ((Ok (Err _)) -1)
            ((Err _)      -2)))
  (output (: 5 Int64)))

(case "a nested Result pattern selects the inner Err arm"
  (doc    "The inner-Err companion: `(Ok (Err 9))` matches the `(Ok (Err e))` arm, binding e=9, not the
           `(Ok (Ok n))` arm. Confirms the nested pattern discriminates the INNER variant, not only the
           outer — both inner arms of an Ok-carrying-Result are reachable.")
  (input  (match (Ok (Err 9))
            ((Ok (Ok n))  n)
            ((Ok (Err e)) e)
            ((Err _)      -2)))
  (output (: 9 Int64)))

(case "an Option carrying a Result is matched across two different sum types"
  (doc    "`(Some (Ok 3))` nests a Result inside an Option — DIFFERENT sum types at the two levels. The
           nested pattern `(Some (Ok n))` deconstructs both, binding n=3. Pins that nested matching
           crosses sum-type boundaries, not only same-sum nesting like `(Some (Some …))`.")
  (input  (match (Some (Ok 3))
            ((Some (Ok n))  n)
            ((Some (Err _)) -1)
            ((None _)       0)))
  (output (: 3 Int64)))

(case "a sum-type value is deconstructed by an exhaustive match"
  (doc    "Patterns are uniform: `(Ctor _)` for nullary constructors. The binder `_` matches the
           unit payload. Consistent with unary constructor patterns like `(Some x)`.")
  (input  (match (Sign.Zero unit)
            ((Sign.Neg _)  -1)
            ((Sign.Zero _) 0)
            ((Sign.Pos _)  1)))
  (output (: 0 Int64)))

; The nested-pattern cases above use CONSTANT scrutinees that fold at compile time. These pin the same
; nested constructor-payload pattern on a RUNTIME sum value (built at run time so it lives on the value
; heap), the shape a compiler hits matching an `Option`/`Result` that carries a user AST node, or a
; wrapper sum whose payload is itself a variant. The runtime matcher must dispatch on the OUTER
; discriminant, then — for a nested constructor payload binder — on the INNER discriminant, falling
; through to the outer arm's siblings when the inner variant does not match (exactly as a hand-written
; inner `match` on the bound payload would).

(case "a runtime wrapper sum whose payload is a variant is matched by a nested constructor pattern"
  (doc    "`(W.Wrap (N.P 5))` is built at run time (through the `f` boundary, so it is a heap value, not
           a folded constant) and matched by nested constructor patterns: `(W.Wrap (N.L v))` and
           `(W.Wrap (N.P v))` share the outer `Wrap` discriminant and dispatch on the INNER `N.L`/`N.P`
           tag, while `(W.Empty _)` is the outer sibling. `(f (W.Wrap (N.P 5)))` selects the `N.P` arm
           (5 + 100 = 105). Pins the runtime nested constructor-payload binder — a payload that is itself
           a sum, deconstructed in one arm — with multiple same-outer arms falling through correctly on
           the inner discriminant. Without it the runtime matcher declined `unsupported payload binder`
           (it bound a `(tuple …)` or a bare name, not a nested constructor).")
  (needs  sum-type-declaration)
  (input  (do
            (type N (L Int64) (P Int64))
            (type W (Wrap N) Empty)
            (def (f w) (match w
                         ((W.Wrap (N.L v)) v)
                         ((W.Wrap (N.P v)) (+ v 100))
                         ((W.Empty _)      -1)))
            (def (main) (f (W.Wrap (N.P 5)))) (export main)))
  (output (: 105 Int64)))

(case "a nested constructor payload that misses the inner variant falls through to the outer sibling"
  (doc    "The fall-through companion of the case above: `(f (W.Empty))` matches no `W.Wrap` arm, so it
           reaches the `(W.Empty _)` sibling (-1). And with `(W.Wrap (N.L 5))` the FIRST arm `(W.Wrap
           (N.L v))` matches (5), not the `N.P` arm — the inner discriminant discriminates. Together
           with the case above these pin all three arms of a nested runtime dispatch reachable: inner
           `N.L`, inner `N.P`, and the outer `Empty` fall-through.")
  (needs  sum-type-declaration)
  (input  (do
            (type N (L Int64) (P Int64))
            (type W (Wrap N) Empty)
            (def (f w) (match w
                         ((W.Wrap (N.L v)) v)
                         ((W.Wrap (N.P v)) (+ v 100))
                         ((W.Empty _)      -1)))
            (def (main) (+ (f (W.Wrap (N.L 5))) (f (W.Empty)))) (export main)))
  (output (: 4 Int64)))

(case "a runtime Option carrying a user sum is matched by a nested constructor pattern"
  (doc    "The reader/self-host idiom: a fallible access yields an `Option` whose `Some` payload is a
           user AST node. `(List.at (List.push (list) (N.L 7)) 0)` is a runtime `(Some (N.L 7))`; the
           nested pattern `(Some (N.L v))` deconstructs the built-in Option AND the user variant in one
           arm, binding v=7, while `(None _)` is the empty-access arm. Pins the built-in polymorphic
           `Option` carrying a user sum through the runtime nested matcher — the `List.at`/`Bytes.at`
           result a compiler threads when it decodes a node list.")
  (needs  sum-type-declaration)
  (input  (do
            (type N (L Int64) (P Int64))
            (def (main) (match (List.at (List.push (list) (N.L 7)) 0)
                          ((Some (N.L v)) v)
                          ((Some (N.P v)) (+ v 100))
                          ((None _)       -1))) (export main)))
  (output (: 7 Int64)))

; --- A constructor pattern NESTED INSIDE A TUPLE ELEMENT of a payload --------------------------
; The nested-payload cases above bind a constructor DIRECTLY under a constructor (`(W.Wrap (N.L v))`).
; A distinct shape a compiler / proof kernel hits is a constructor pattern nested inside a TUPLE
; ELEMENT of the payload — `(Outer.Wrap (tuple (Inner.A v) k))`, where the payload is a tuple and one
; of its elements is itself a variant to destructure in the SAME arm. This composes two binder kinds
; the corpus covers separately (a tuple-payload binder, §"a match arm binds a nested tuple inside a sum
; payload"; and a nested constructor binder, §"a runtime wrapper sum whose payload is a variant"), but
; the COMBINATION — a ctor pattern occupying a tuple SLOT — is not yet lowered: the seed declines
; ("runtime sum match: unsupported nested payload binder"). This is exactly the shape of a HOL-style
; `dest_eq`/`TRANS` arm — `(= l r)` destructured as `(Comb (tuple (Comb (tuple _eq l)) r))` where a
; `Comb` binder sits in a tuple slot. A generation that does not yet lower a ctor-in-tuple-slot binder
; declines rather than miscompiling; the recorded oracle is what the composed lowering produces.

(case "a constructor pattern in a tuple payload slot is matched in one arm"
  (doc    "`Outer.Wrap` carries `(Tuple Inner Int64)`; the arm `(Outer.Wrap (tuple (Inner.A v) k))`
           destructures the payload tuple AND the `Inner` variant in its first slot in ONE arm, binding
           `v` and `k`. `(f (Outer.Wrap (tuple (Inner.A 20) 22)))` selects the `Inner.A` arm → 20 + 22 =
           42. Pins a constructor pattern occupying a TUPLE SLOT of a payload — the composition of a
           tuple-payload binder and a nested-constructor binder, the shape a kernel's equation-arm
           `(Comb (tuple (Comb …) r))` takes. The seed declines (\"unsupported nested payload binder\"):
           it lowers a ctor-directly-under-ctor and a flat tuple binder, but not a ctor inside a tuple
           slot. A generation composing the two reproduces 42.")
  (needs  sum-type-declaration)
  (input  (do
            (type Inner (A Int64) (B Int64))
            (type Outer (Wrap (Tuple Inner Int64)))
            (def (f o)
              (match o
                ((Outer.Wrap (tuple (Inner.A v) k)) (+ v k))
                ((Outer.Wrap (tuple (Inner.B v) k)) (- v k))))
            (def (main) (f (Outer.Wrap (tuple (Inner.A 20) 22)))) (export main)))
  (output (: 42 Int64)))

(case "a ctor-in-tuple-slot match is expressible by binding the tuple then re-matching"
  (doc    "The route around the not-yet-lowered ctor-in-tuple-slot binder: bind the tuple element to a
           NAME in the outer arm, then re-match it in a nested `match`. `(Outer.Wrap (tuple i k))` binds
           the whole first slot as `i`, and the inner `(match i ((Inner.A v) …) …)` destructures it —
           the two binder kinds SEPARATED across two matches rather than composed in one pattern. Same
           input, same result (42). Pins that the composed pattern is a surface convenience over
           bind-then-re-match, which IS lowered — a kernel is not blocked, only more verbose (this is
           the one-level-at-a-time peel a HOL `dest_eq` uses to route around the gap above).")
  (needs  sum-type-declaration)
  (input  (do
            (type Inner (A Int64) (B Int64))
            (type Outer (Wrap (Tuple Inner Int64)))
            (def (f o)
              (match o
                ((Outer.Wrap (tuple i k))
                   (match i ((Inner.A v) (+ v k)) ((Inner.B v) (- v k))))))
            (def (main) (f (Outer.Wrap (tuple (Inner.A 20) 22)))) (export main)))
  (output (: 42 Int64)))

; --- A compound bound from a sum payload, extracted ACROSS A FUNCTION BOUNDARY, then projected ---
; A value bound out of a sum payload carries its shape WITHIN THE MATCH ARM (the payload-bound cases
; above project fields, index lists, and re-match variants inside the arm). But when the payload
; compound is RETURNED FROM A HELPER — `(def (unbox b) (match b ((Box.B t) t)))` — and the caller then
; applies a positional accessor `(. x N)` to the returned value, the shape is not recovered at the
; projection site. WORSE THAN A DECLINE: the seed does not refuse to derive a component (which the gate
; would score `todo`); it REJECTS the program with a type-error code (CDZ0201 "tuple access on a
; non-tuple"), asserting a VALID program is ill-typed — a decline-don't-miscompile violation
; (spec/learnings/2026-07-03-decline-do-not-miscompile.md; the projection companion of §"a scalar
; element is projected DIRECTLY from a named function's runtime tuple result", which built an invalid
; component before it learned to decline). Projecting INLINE in the arm, or binding the tuple's slots
; by a tuple-pattern in the arm, recovers the shape and compiles (the control below). This is precisely
; why a HOL-style `concl : Thm -> Term` that returns the payload term for a CALLER to match fails,
; while extracting the conclusion inline in the `Thm` arm compiles — the payload's shape must be
; consumed where it is bound, not threaded through a bare return. The recorded oracle is the projected
; element; a seed that rejects it here FAILS this case, because the program is well-typed — the correct
; not-yet-covered behavior is to DECLINE (todo), never to reject a valid program as a type error.

(case "a tuple payload extracted through a helper return must not be rejected as a type error"
  (doc    "`unbox` returns the `Box.B` payload tuple to its caller, which then applies positional access `1`.
           `(unbox (Box.B (tuple (list) (Term.Var 7))))` is a `(List Int64, Term)` pair, so `(.            … 1)` projects the `Term` and `is-var` of it is 1 — a WELL-TYPED program. The seed does not
           recover the tuple shape at the projection site and REJECTS with CDZ0201 (\"tuple access on a
           non-tuple\"), asserting a valid program is ill-typed — a decline-don't-miscompile violation:
           for a shape it cannot yet thread through a function return it MUST decline (scored todo), not
           reject a valid program (this case FAILs a generation that rejects). This is the shape a HOL
           `concl`/`dest_thm` takes when it returns a payload term for the CALLER to consume — the
           reason such an accessor fails while inline extraction compiles. A generation threading the
           payload's shape through the return reproduces the projected element 1.")
  (needs  sum-type-declaration)
  (input  (do
            (type Term (Var Int64) (Neg Int64))
            (type Box (B (Tuple (List Int64) Term)))
            (def (is-var tm) (match tm ((Term.Var _) 1) ((Term.Neg _) 0)))
            (def (unbox bx) (match bx ((Box.B t) t)))
            (def (main)
              (let ((p (unbox (Box.B (tuple (list) (Term.Var 7))))))
                (is-var (. p 1)))) (export main)))
  (output (: 1 Int64)))

(case "a tuple payload consumed INLINE in the sum arm projects and re-matches"
  (doc    "The control the case above must be distinguished from, and the route a program takes today:
           consume the payload tuple's slots INLINE in the `Box.B` arm — bind them by a tuple-pattern —
           rather than returning the tuple for a caller to project. `((Box.B (tuple _ c)) (is-var c))`
           binds the second slot `c` (a `Term`) in the arm and re-matches it, yielding 1. Pins that a
           payload compound's shape IS available where it is bound — the gap is specifically threading
           it through a bare function RETURN. A HOL kernel routes around `concl`-then-match by
           destructuring the `Thm` inline in the arm that needs the conclusion (the pattern the working
           spike used to verify a minted theorem).")
  (needs  sum-type-declaration)
  (input  (do
            (type Term (Var Int64) (Neg Int64))
            (type Box (B (Tuple (List Int64) Term)))
            (def (is-var tm) (match tm ((Term.Var _) 1) ((Term.Neg _) 0)))
            (def (main)
              (match (Box.B (tuple (list) (Term.Var 7)))
                ((Box.B (tuple _ c)) (is-var c)))) (export main)))
  (output (: 1 Int64)))

; The same payload-through-a-return gap on the BUILT-IN `Option` takes a WORSE form than on a declared
; sum: where `(unbox …)` on a declared `Box` REJECTS the projection (CDZ0201, above — bad, but a refusal),
; a helper that returns a built-in `Some`'s tuple payload emits a VALID component that TRAPS at run time.
; `get` binds `(Some p)`'s payload `p` and returns it; the caller applies positional access `0`. The program is
; well-typed — `(Some (tuple 7 8))`'s payload is a two-tuple, and both inline routes below yield 7 — so
; the recorded outcome is 7. But the seed does not thread the payload's tuple shape through `get`'s return,
; and instead of DECLINING (scored todo) it emits a component whose positional access `0` traps: a decline-don't-
; miscompile violation of the emit-a-broken-component kind (worse than the declared-sum rejection, which
; at least refuses). The correct not-yet-covered behavior is to decline; running to a trap where the
; program has a value is the failure this case pins.

(case "a tuple payload returned through a helper from a built-in Option must not trap"
  (doc    "`get` binds the payload `p` of `(Some p)` and returns it; `(. (get (Some (tuple 7 8))) 0)`
           projects element 0 of that two-tuple payload — a well-typed program whose value is 7 (both
           inline routes below confirm it). The seed does not recover the payload's tuple shape through
           `get`'s bare return and emits a VALID component that TRAPS at positional access `0`, rather than declining
           — a decline-don't-miscompile violation of the emit-a-broken-component kind. This is the
           built-in-`Option` companion of the declared-sum `Box` case above, and WORSE: the declared sum
           rejects the projection (CDZ0201) while the built-in one runs to a trap. A generation that
           cannot yet thread a built-in sum payload's shape through a function return MUST decline (scored
           todo), never emit a component that traps on a valued program.")
  (input  (do
            (def (get o) (match o ((Some p) p) (None (tuple 0 0))))
            (def (main) (. (get (Some (tuple 7 8))) 0)) (export main)))
  (output (: 7 Int64)))

(case "a built-in Option tuple payload consumed INLINE in the Some arm projects"
  (doc    "The control the trap case above must be distinguished from: consume the `Some` payload's tuple
           INLINE in the arm — project `(. p 0)` where `p` is bound in the `Some` arm — rather than
           returning it for a caller. `(match (Some (tuple 7 8)) ((Some p) (. p 0)) (None 0))` yields
           7. Pins that the payload's shape IS available where it is bound; the gap is threading it through
           a bare function RETURN, exactly as for the declared-sum `Box` pair above.")
  (input  (do
            (def (main) (match (Some (tuple 7 8)) ((Some p) (. p 0)) (None 0))) (export main)))
  (output (: 7 Int64)))

; Projecting the RESULT of a call to a function that TAKES and RETURNS a tuple parameter must compute or
; decline, never trap. `(def (go t) t)` is the identity on a tuple; `(. (go (tuple 5 0)) 0)` is a
; well-typed projection of a two-tuple, value 5. When `go`'s body PROJECTS its tuple parameter (`(. ; t 0)`) — which `(. x N)`-on-a-parameter declines as "unknown tuple shape" — that decline shape appears to
; poison `go`'s return type, so the CALLER'S `(. (go …) 0)` emits a VALID component that TRAPS instead
; of computing 5. The trap is not depth-dependent: it happens even at recursion depth 0 (the base arm
; returns the tuple immediately). Contrast: a function returning a FRESH tuple (`(def (mk n) (tuple n (+ n
; 1)))`) has its result projected fine (`(. (mk 5) 0)` = 5), and a SCALAR accumulator threaded through
; the same recursion computes correctly — so the program is well-typed and the value is representable; only
; a tuple-typed parameter threaded and returned, then projected at the call site, traps. self-hosting-and-
; bootstrap.md #An Unsupported Construct Is Declined, Not Miscompiled: a shape the compiler cannot thread
; through the call MUST decline, never emit a component that traps on a valued program.

(case "projecting the result of a function that threads a tuple parameter must not trap"
  (doc    "`(def (go n t) (if (= n 0) t (go (- n 1) (tuple (+ (. t 0) n) (. t 1)))))` threads a
           tuple accumulator `t`, projecting `(. t 0)`/`(. t 1)` in its body and returning a tuple;
           `(. (go 3 (tuple 0 0)) 0)` is well-typed with value 6 (the scalar-accumulator analogue
           computes, and a fresh-tuple-returning helper's result projects fine). The seed emits a VALID
           component that TRAPS at the caller's positional access `0` — the tuple parameter's shape, which `(. x N)` on
           a parameter declines as unknown, is not threaded through `go`'s return, so the call-result
           projection traps rather than computing or declining (a decline-don't-miscompile violation; the
           trap happens even at recursion depth 0). Companion of the built-in-Option payload-return case
           above — here the tuple flows through an ordinary tuple-typed parameter rather than a sum payload.
           A generation that cannot yet thread a tuple parameter's shape through the return declines rather
           than emitting a component that traps.")
  (input  (do
            (def (go n t) (if (= n 0) t (go (- n 1) (tuple (+ (. t 0) n) (. t 1)))))
            (def (main) (. (go 3 (tuple 0 0)) 0)) (export main)))
  (output (: 6 Int64)))

(case "an association list is searched by key with a tuple-carrying Option match"
  (doc    "The compiler's symbol-table / environment idiom: a list of `(key value)` tuples searched by
           key. `lookup` recurses by index, and each `(List.at xs i)` yields an `Option` whose `Some`
           payload is a TUPLE, deconstructed in one arm `((Some (tuple key val)) …)`; it compares the
           bound `key` to the target `k` and returns `val` on a hit, else recurses. `(lookup (list
           (tuple 1 100) (tuple 2 200)) 0 2)` = 200. Pins that the tuple payload's slot binders (`key`,
           `val`) recover their concrete scalar kinds even though the list is a PARAMETER (opaque
           `Heap`, so the element shape — hence the tuple's slot types — is unknown): `key` infers
           Int64 from `(= key k)` by arm-unification, so it unboxes and the `=`/return kinds agree.
           Without that per-slot recovery the binders stay opaque `Heap` and the match declines
           'equality of differing kinds' / 'arms differ in kind'. This is the assoc-list an environment
           or a string→index symbol table is built on.")
  (needs  sum-type-declaration)
  (input  (do
            (def (lookup xs i k)
              (match (List.at xs i)
                ((Some (tuple key val)) (if (= key k) val (lookup xs (+ i 1) k)))
                ((None _)               -1)))
            (def (main) (lookup (list (tuple 1 100) (tuple 2 200)) 0 2)) (export main)))
  (output (: 200 Int64)))

(case "lists are equal by elements in order"
  (doc    "Witnesses collections-and-text.md #A List Is An Ordered Homogeneous Sequence (equality).
           Needs the primitive collections the seed realizes to build an AST (list/map/record).")
  (needs collections)
  (input  (= (list 1 2 3) (list 1 2 3)))
  (output (: true Bool)))

; Two lists of the SAME element type but DIFFERENT LENGTH are the SAME TYPE — a list's length is NOT
; part of its type (a list is a variable-length sequence, grown by `List.push`; collections-and-text.md
; #A List Is An Ordered Homogeneous Sequence types a list by its ELEMENT type, and #A List Is Grown By
; Functional Construction makes length runtime-varying). So comparing `(list 1 2)` with `(list 1 2 3)`
; is a comparison of two `(List Int64)` values, which MUST be TOTAL (core-semantics.md #Equality Is
; Structural — comparable when their types match) and yield `false` (different elements), NOT a type
; error. This is UNLIKE a tuple, whose arity IS part of its type (two tuples of different arity are
; different types and comparing them is rejected). A compiler that treats list length like tuple arity —
; reusing the tuple-shape-incompatibility check on lists — wrongly rejects `(= (list 1 2) (list 1 2 3))`
; as "comparison between values of different shapes", a false rejection of a well-typed total comparison.

(case "two lists of different length are unequal, not a type error"
  (doc    "`(= (list 1 2) (list 1 2 3))` compares two `(List Int64)` values of different length — a list's
           length is not part of its type (a list is variable-length; collections-and-text.md #A List Is
           An Ordered Homogeneous Sequence types it by element type, #A List Is Grown By Functional
           Construction), so both are the same type and the comparison is TOTAL, yielding `false` (they
           differ in their elements), NOT a type error. This is unlike a tuple, whose arity IS part of its
           type (different-arity tuples are rejected as different shapes). Pins that list equality does not
           treat length as a shape mismatch — a compiler reusing the tuple-arity incompatibility check on
           lists wrongly rejects this well-typed total comparison as 'different shapes'.")
  (needs  collections)
  (input  (= (list 1 2) (list 1 2 3)))
  (output (: false Bool)))

; --- A list is homogeneous: its elements share one type ----------------------------------
; collections-and-text.md #A List Is An Ordered Homogeneous Sequence: "A list MUST be an ordered
; sequence whose elements share one type." A list literal whose elements do NOT share one type is
; ill-typed, so the compiler REJECTS it (CDZ0201) rather than build a heterogeneous list value — the
; discipline that prevents projecting a mixed element back out at a different type.

(case "a list mixing integer and boolean elements is a type error"
  (doc    "`(list 1 true)` has an Int64 element and a Bool element — they do not share one type, so
           the list is not homogeneous and the compiler rejects it (CDZ0201, collections-and-text.md
           #A List Is An Ordered Homogeneous Sequence), or declines if it does not yet cover the
           homogeneity rule.")
  (needs     collections)
  (input     (list 1 true))
  (error     CDZ0201))

(case "a list mixing integer and float elements is a type error"
  (doc    "The numeric companion: Int64 and Float64 are distinct types that do not silently unify
           (numeric-model.md #Numeric Types Do Not Silently Promote), so `(list 1 2.5)` mixes two
           element types and is not homogeneous — the compiler rejects it (CDZ0201). Pins that
           homogeneity holds across the numeric types too, not only across obviously unrelated kinds
           like Int64 and Bool.")
  (needs     collections)
  (input     (list 1 2.5))
  (error     CDZ0201))

; Homogeneity is a property of the LIST VALUE, so it must hold under `List.push` too, not only for a
; list LITERAL. collections-and-text.md #A List Is Grown By Functional Construction: `List.push`
; "MUST produce a NEW LIST VALUE" — and a list value's elements share one type (#A List Is An Ordered
; Homogeneous Sequence). So `(List.push (list 1 2) true)` appends a Bool to an Int64 list, making the
; result non-homogeneous — the same violation as the `(list 1 true)` literal above, which MUST be
; rejected (CDZ0201). A `List.push` that skips the element-type check the literal enforces does not just
; build a heterogeneous list: it renders the RESULT using the pushed element's type, so the original
; Int64 elements `1 2` come back as `true true` — `(List.push (list 10 20) false)` yields
; `(list true true false)`, projecting the stored integers 10 and 20 at the Bool type. That is a
; wrong-value miscompile (the "projecting a mixed element back out at a different type" hazard the
; homogeneity rule exists to prevent), strictly worse than a missing rejection. A generation that does
; not yet check the pushed element's type declines rather than building the corrupted list.

(case "pushing an element of a different type onto a list is a type error"
  (doc    "`(List.push (list 1 2) true)` appends a Bool to an Int64 list — the result is not homogeneous
           (collections-and-text.md #A List Is An Ordered Homogeneous Sequence), so it is a type error
           (CDZ0201), exactly as the `(list 1 true)` literal above is. `List.push` produces a new LIST
           value (#A List Is Grown By Functional Construction), which must satisfy the same
           element-share-one-type rule the literal does. A `List.push` that skips the element-type check
           miscompiles: it renders the result at the pushed element's type, so the stored integers come
           back as booleans (`(List.push (list 10 20) false)` → `(list true true false)`) — a wrong value,
           not merely a missing rejection. A generation that does not yet check the pushed element's type
           declines rather than building the mistyped list.")
  (needs     collections)
  (input     (List.push (list 1 2) true))
  (error     CDZ0201))

; `List.update` is the other functional-construction operator (#A List Is Grown By Functional
; Construction pairs "append an element" with "replace the element at an index"), and it has the same
; obligation: replacing a slot with an element of a different type breaks homogeneity. `(List.update
; (list 1 2 3) 1 true)` puts a Bool where an Int64 was — a non-homogeneous result, CDZ0201, exactly as
; the `List.push` and literal cases. And it exhibits the identical render corruption: the whole result
; is walked at the replacement element's type, so `(List.update (list 10 20 30) 0 false)` yields
; `(list false true true)` — the untouched integers 20 and 30 project as `true`. Pins that the
; element-type check covers BOTH functional-construction operators, not only `push`.

(case "updating a list slot with an element of a different type is a type error"
  (doc    "`(List.update (list 1 2 3) 1 true)` replaces an Int64 slot with a Bool — the result is not
           homogeneous (collections-and-text.md #A List Is An Ordered Homogeneous Sequence), a type error
           (CDZ0201), the `List.update` companion of the `List.push` case above. Like push, an unchecked
           update miscompiles by rendering the result at the replacement element's type: `(List.update
           (list 10 20 30) 0 false)` → `(list false true true)`, projecting the untouched integers 20 and
           30 as booleans — a wrong value. Pins that both functional-construction operators enforce the
           element-type rule the literal does. A generation that does not yet check the replacement
           element's type declines rather than building the mistyped list.")
  (needs     collections)
  (input     (List.update (list 1 2 3) 1 true))
  (error     CDZ0201))

; Homogeneity is by element TYPE, and two compound values of the same KIND but different SHAPE are
; different types (type-system.md #Structural Values Are Comparable Only When Their Shapes Match:
; records equal only when their field-name SETS match, tuples only when their lengths match, sums
; only when their variant SETS match). So a list of records with different field sets, or of tuples
; of different arities, is NOT homogeneous and the compiler rejects it (CDZ0201) — the same
; shape-compatibility distinction the equality path applies, applied per element.

(case "a list of records with different field sets is a type error"
  (doc    "`(record (a 1))` and `(record (b 2))` are records with DIFFERENT field-name sets — different
           types (type-system.md #Structural Values Are Comparable Only When Their Shapes Match). A
           list of them is not homogeneous → CDZ0201, exactly as comparing them `(= (record (a 1))
           (record (b 2)))` is rejected.")
  (needs     collections)
  (input     (list (record (a 1)) (record (b 2))))
  (error     CDZ0201))

(case "a list of tuples with different arities is a type error"
  (doc    "`(tuple 1 2)` and `(tuple 1 2 3)` are tuples of different lengths — different types, so a
           list of them is not homogeneous → CDZ0201 (comparing them is likewise rejected as different
           shapes). Pins that the list-element homogeneity check compares tuple ARITY, not just that
           both elements are tuples.")
  (needs     collections)
  (input     (list (tuple 1 2) (tuple 1 2 3)))
  (error     CDZ0201))

; The two cases above reject a list whose RECORD/TUPLE elements differ in shape, because a record's
; field set and a tuple's arity ARE their type. A MAP is the crucial counterpoint: two maps with
; different KEY SETS are the SAME type (Map<KeyType, ValueType>) — a map's keys are runtime data, not
; part of its type (the same distinction the map-comparison cases below turn on). So a list whose
; elements are maps with different keys IS homogeneous, and building it is well-typed — NOT a CDZ0201
; rejection. The seed shares one `shapes_incompatible` check across records and maps (it treats a map's
; key set like a record's field set), so it wrongly rejects this list "elements do not share one shape"
; — the SAME miscompile as the different-keyset map COMPARISON, surfacing here on the list-homogeneity
; path. The recorded true is the correct oracle; the seed's rejection is the bug.

(case "a list of maps with different keys is homogeneous, not a type error"
  (doc    "`(map (a 1))` and `(map (b 2))` are both maps — the SAME type (Map), since a map's key set is
           runtime data, not part of its type (unlike a record's field set). A list of them is therefore
           homogeneous, and comparing two such lists is well-typed and true (identical lists of identical
           maps). The seed wrongly rejects it (CDZ0201 \"list elements do not share one shape\") — the
           list-homogeneity manifestation of the different-keyset map-comparison bug (shapes_incompatible
           shares one arm for Record and Map). MUST be true. Contrast the record/tuple cases above, which
           ARE non-homogeneous because field set / arity IS the type.")
  (needs     collections)
  (input     (= (list (map ("a" 1)) (map ("b" 2))) (list (map ("a" 1)) (map ("b" 2)))))
  (output    (: true Bool)))

(case "indexing a list in bounds yields Some of the element"
  (doc    "Witnesses collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping: a
           well-typed list access whose index is in range yields the element wrapped in Some — an
           Option, not a bare value — so absence and presence share one total return type.")
  (needs fallible-access)
  (input  (List.at (list 1 2 3) 1))
  (output (: (Some 2) (Option Int64))))

(case "indexing a list out of bounds yields None"
  (doc    "Witnesses collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping: a
           well-typed list access whose index is out of range yields None rather than trapping or
           reading an unspecified value. A program that requires the element unwraps this Option with
           `expect` (core-semantics.md #Requiring The Value Of An Optional Traps On Absence).")
  (needs fallible-access)
  (input  (List.at (list 1 2 3) 5))
  (output (: (None unit) (Option Int64))))

; A NEGATIVE index is out of bounds exactly as an over-large one is — no element sits at position -1.
; It is the classic fallible-access miscompile: a lowering that casts the signed index to an unsigned
; width (wasm addresses memory with u32/u64 offsets) turns -1 into a huge in-range-looking offset,
; reading an unspecified value instead of yielding None. #Indexing And Lookup Are Fallible, Not
; Trapping requires None; this pins the negative side of the bounds check the `5` case only exercises
; on the high side.

(case "indexing a list with a negative index yields None"
  (doc    "`(List.at (list 1 2 3) -1)` uses a negative index — no element at position -1 — so it MUST
           yield None (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping), NOT
           wrap the negative index to a large unsigned offset and read an unspecified element. The
           negative-index companion of the out-of-bounds `5` case above; both yield None.")
  (needs fallible-access)
  (input  (List.at (list 1 2 3) -1))
  (output (: (None unit) (Option Int64))))

(case "indexing an empty list yields None"
  (doc    "`(List.at (list) 0)` indexes position 0 of a list with no elements — out of bounds, since
           an empty list has no element at any index — so it MUST yield None. Pins the degenerate
           boundary: index 0 is present only when the list is non-empty.")
  (needs fallible-access)
  (input  (List.at (list) 0))
  (output (: (None unit) (Option Int64))))

(case "indexing a list bound from a sum payload yields the element"
  (doc    "`List.at` on a list that was BOUND OUT OF A SUM PAYLOAD by a `match` arm must read its element
           exactly as `List.at` on a top-level list does — a payload-bound list is the same runtime array
           handle, just reached through a constructor. `f` matches `K.KK`, binding the payload's `(list 10
           20 30)` as `xs`, and reads element 0 → 10. Pins that element access on a payload-bound list is
           wired: `List.len` on such a list already works (a compiler reading `List.len` of a bound arg
           list), so `List.at` on it must too, or a compiler cannot iterate a list stored in a node — the
           natural representation of a multi-argument call `(K fn-index (list args…))` whose lowering reads
           `List.at args i` for each argument. Distinct from the top-level `List.at` cases above (the list
           is a direct parameter there); here the list arrives through a sum payload, the shape a
           self-hosted compiler's IR nodes carry.")
  (needs  sum-type-declaration)
  (input  (do
            (type K (KK (Tuple Int64 (List Int64))))
            (def (f c) (match c ((K.KK (tuple fi xs)) (match (List.at xs 0) ((Some x) x) ((None _) -1)))))
            (def (main) (f (K.KK (tuple 7 (list 10 20 30))))) (export main)))
  (output (: 10 Int64)))

(case "a multi-argument call node is evaluated by iterating its payload arg list"
  (doc    "The multi-argument-call idiom the payload-bound `List.at` unblocks — a self-hosted compiler's
           natural N-ary call representation. A `KCall` node carries `(Tuple Int64 (List Core))`: a
           function index plus an argument LIST of sub-nodes. `ev` matches `KCall`, binds the arg list
           `xs` from the payload, and iterates it — `List.len xs` gives the count, `List.at xs i` reads
           each argument node, and each is evaluated recursively and summed. `KCall` with three
           `KConst` args [10 20 12] evaluates to 10+20+12 = 42. Pins that a list of HEAP sub-nodes stored
           in a sum payload is both measurable (`List.len`) and indexable (`List.at`) and its elements are
           themselves consumable (matched/recursed), so a compiler can lower an N-ary node by iterating
           its payload arg list — distinct from the single-element case above (here the payload list holds
           recursive sum values and is walked in full, the exact shape a `lower`/`ev` pass over a call
           node with a variable argument count takes).")
  (needs  sum-type-declaration)
  (input  (do
            (type Core (KConst Int64) (KCall (Tuple Int64 (List Core))))
            (def (sum-args xs i n) (if (< i n)
                                       (+ (ev (match (List.at xs i) ((Some c) c) ((None _) (Core.KConst 0))))
                                          (sum-args xs (+ i 1) n))
                                       0))
            (def (ev c) (match c
                          ((Core.KConst v) v)
                          ((Core.KCall (tuple fi xs)) (sum-args xs 0 (List.len xs)))))
            (def (main) (ev (Core.KCall (tuple 9 (list (Core.KConst 10) (Core.KConst 20) (Core.KConst 12)))))) (export main)))
  (output (: 42 Int64)))

(case "map equality is independent of insertion order"
  (doc    "Witnesses collections-and-text.md #A Map Associates Keys With Values.")
  (needs collections)
  (input  (= (map ("a" 1) ("b" 2)) (map ("b" 2) ("a" 1))))
  (output (: true Bool)))

; Map equality is STRUCTURAL and must not depend on whether a map's key was a compile-time constant or
; computed at run time (core-semantics.md #Equality Is Structural — two values equal exactly when their
; canonical forms coincide). `(let ((k 5)) (map (k 1)))` and `(let ((j (+ 2 3))) (map (j 1)))` are the
; SAME map `{5:1}` — both render `(map (5 1))`, both `Map.lookup 5` → `(Some 1)`, both size 1 — so they
; MUST compare equal. The seed instead returns FALSE when one side's key was computed at run time (a
; runtime-constructed map) and the other's was a constant (a const-folded map): it compares the two maps'
; DIFFERENT internal representations (const-folded value vs runtime heap handle) rather than their values.
; This is a wrong VALUE, not a decline — worse than the list/tuple case, where a runtime-compound equality
; honestly DECLINES "runtime compound equality (heap walk) not yet emitted"; the map path emits an equality
; that silently answers false. A generation whose map equality cannot yet compare a runtime-constructed
; map against a const one MUST decline (as list/tuple equality does) rather than answering false.

(case "a map with a computed key equals the same map with a constant key"
  (doc    "`(let ((j (+ 2 3))) (map (j 1)))` and `(let ((k 5)) (map (k 1)))` are the SAME map `{5:1}` —
           `(+ 2 3)` is 5, both render `(map (5 1))`, both look up key 5 to `(Some 1)`, both have size 1 —
           so they compare equal under structural equality (core-semantics.md #Equality Is Structural),
           independent of whether the key was a compile-time constant or computed at run time. The seed
           returns false: it compares a runtime-constructed map's heap representation against a const map's
           folded representation rather than their values — a wrong value (a structural-equality miscompile
           for maps). Worse than the list/tuple runtime-compound-equality case, which honestly declines
           rather than answering; the map path emits an equality that answers false. MUST be true. A
           generation whose map equality cannot yet compare a runtime map against a const one declines
           rather than answering false.")
  (needs   collections)
  (input   (let ((j (+ 2 3))) (let ((k 5)) (= (map (j 1)) (map (k 1))))))
  (output  (: true Bool)))

; The positive half of the map-key-is-a-value rule: a name bound in scope, used in a key position, keys
; the map by the name's VALUE — not by the literal name. `(let ((a 5)) (map (a 1)))` is the map with the
; INTEGER key 5, so it equals `(Map.insert Map.empty 5 1)` and is NOT equal to the String-keyed `(Map.insert
; Map.empty "a" 1)`. Decisively, two DISTINCT names bound to the SAME value key the same entry: `(let ((a
; 5)) (let ((b 5)) (map (a 1) (b 2))))` has ONE entry (size 1, the later `b 2` overwriting `a 1` at key 5),
; because keys are compared by value (collections-and-text.md #Keys Are Compared By Value) — impossible if
; the key were the literal name (`a` and `b` differ as strings). This is the companion of the unbound-key
; case below: a bound key resolves to its value; an unbound key is a scope error (never a coerced string).

(case "a bound name in a map key is used by its value, not the literal name"
  (doc    "`a` is bound to 5, so `(map (a 1))` keys the map by the VALUE 5 — it equals `(Map.insert
           Map.empty 5 1)` (an integer key), the value the name holds, NOT the String `\"a\"` of the
           literal name (collections-and-text.md #A Map's Canonical Form: a map's keys are values, resolved
           in scope, not compile-time labels). A reader that treated the key as the literal name would key
           by `\"a\"` and this equality would be false. Pins the positive half of the map-key-is-a-value
           rule (the unbound-key scope-error case below is the negative half). MUST be true.")
  (needs      collections)
  (input      (let ((a 5)) (= (map (a 1)) (Map.insert Map.empty 5 1))))
  (output     (: true Bool)))

(case "two distinct names bound to the same value key the same map entry"
  (doc    "The decisive witness that a map key is the name's VALUE, not the name: `a` and `b` are distinct
           names both bound to 5, so `(map (a 1) (b 2))` has ONE entry at key 5 (the later `(b 2)`
           overwrites `(a 1)`), size 1 — keys are compared by value (collections-and-text.md #Keys Are
           Compared By Value), and 5 = 5. If the key were the literal name, `a` and `b` would be distinct
           string keys and the map would have size 2. MUST be 1.")
  (needs      collections)
  (input      (let ((a 5)) (let ((b 5)) (Map.size (map (a 1) (b 2))))))
  (output     (: 1 Int64)))

; The value-not-literal rule holds when the bound value is itself a STRING, ruling out a type-driven
; coercion (a reader that stringified an ident only when the key type were String would pass the integer
; cases above yet still be wrong here). `a` bound to the String `"x"` keys the map by the VALUE `"x"`, so
; `(map (a 1))` equals `(Map.insert Map.empty "x" 1)` — NOT the map keyed by the literal name `"a"`. And
; two distinct names bound to the SAME string collide to one entry (value equality), while two bound to
; DIFFERENT strings give two — the same value-semantics witness as the integer case, at String key type.
; Together with the integer cases, these show the key is resolved to its VALUE regardless of the value's
; type; the only wrong case is an UNBOUND name (the scope-error case below), which is a resolution failure,
; not a type-driven choice.

(case "a name bound to a string keys a map by its value, not the literal name"
  (doc    "`a` is bound to the String `\"x\"`, so `(map (a 1))` keys the map by the VALUE `\"x\"` — it
           equals `(Map.insert Map.empty \"x\" 1)`, NOT the map keyed by the literal name `\"a\"`. Pins
           that the value-not-literal rule holds at String key type too, ruling out a type-driven ident
           coercion (a reader that stringified an ident only for a String key type would still be wrong
           here — the key is the bound value `\"x\"`, not the name). MUST be true.")
  (needs      collections)
  (input      (let ((a "x")) (= (map (a 1)) (Map.insert Map.empty "x" 1))))
  (output     (: true Bool)))

(case "distinct names bound to the same string key the same map entry"
  (doc    "The String-key companion of the same-value-collision witness: `a` and `b` are distinct names
           both bound to `\"k\"`, so `(map (a 1) (b 2))` has ONE entry at key `\"k\"` (size 1) — keys
           compared by value, and `\"k\"` = `\"k\"`. If the key were the literal name, they would be
           distinct string keys `\"a\"`/`\"b\"` and size would be 2. MUST be 1.")
  (needs      collections)
  (input      (let ((a "k")) (let ((b "k")) (Map.size (map (a 1) (b 2))))))
  (output     (: 1 Int64)))

; A map's KEY is a VALUE, not a compile-time label: collections-and-text.md #A Map's Canonical Form —
; "a map's keys are values of one key type; a record's field names are fixed compile-time labels." So in
; a `(map (k v) …)` literal the key position `k` is an ORDINARY EXPRESSION evaluated and resolved in
; scope (that is how a map has a dynamic key at all) — a bound name resolves to its VALUE (`(let ((k 42))
; (map (k 1)))` is the map `(map (42 1))`, equal to `(Map.insert Map.empty 42 1)`), exactly as the value
; position does. It follows that an UNBOUND name in a key position is the ordinary unbound-name error
; (core-semantics.md #Binding Is Lexical — "a reference to a name with no enclosing binding MUST be a
; compile-time error", unconditional), CDZ0101, exactly as an unbound name in the value position or in any
; ordinary expression is. The seed instead SILENTLY COERCES an unbound key name to a String literal of its
; spelling — `(map (undefined-key 1))` yields `(map ("undefined-key" 1))` — a wrong value that swallows
; the scope error and fabricates a String key the program never wrote. This is the map-key instance of the
; unquote fallback family (an operation that must EVALUATE its operand falling back to quoting/stringifying
; it when evaluation fails, picking the wrong branch for a broken name): a key that fails to resolve is the
; name's scope error, not a cue to reinterpret the name as a String. (There is also no readable literal for
; a String or integer key — `(map ("a" 1))` and `(map (1 10))` both decline "a map entry is not a (key
; value) pair" — so the coercion is the only way the current reader expresses a String key, which is why
; the corpus's `(map (a 1))` cases lean on it; both the coercion and the missing literal-key syntax are the
; same defect: the key position is not read as an ordinary value expression.) A generation that does not
; yet evaluate a map key as a scoped value declines rather than coercing an unbound name to a String.

(case "an unbound name in a map key is a scope error, not a coerced string"
  (doc    "A map's key is a VALUE resolved in scope (collections-and-text.md #A Map's Canonical Form: a
           map's keys are values, not compile-time labels), so a bound name in a key position resolves to
           its value — `(let ((k 42)) (map (k 1)))` is `(map (42 1))` = `(Map.insert Map.empty 42 1)`. An
           UNBOUND name in a key position is therefore the ordinary unbound-name error (CDZ0101,
           core-semantics.md #Binding Is Lexical), exactly as an unbound name in the value position is. The
           seed instead silently coerces the unbound key name to a String of its spelling — `(map
           (undefined-key 1))` → `(map (\"undefined-key\" 1))` — a wrong value that swallows the scope error
           and fabricates a String key the program never wrote (the map-key instance of the unquote-fallback
           family: a position that must EVALUATE its operand must not reinterpret an unresolvable name as a
           String). A generation that does not yet evaluate a map key as a scoped value declines rather than
           coercing.")
  (needs      collections)
  (input      (map (undefined-key 1)))
  (error      CDZ0101))

; --- A map's values share one type (and its keys share one type) -------------------------
; collections-and-text.md #A Map Associates Keys With Values: "A map MUST associate keys of one type
; with values of one type." So a map whose VALUES do not share one type (an Int64 value and a Bool
; value) is ill-typed and the compiler REJECTS it (CDZ0201), exactly as a heterogeneous LIST is
; rejected. (These are written in an equality position because a map is not yet a producible top-level
; value; the equality still forces the map to be built, exercising the homogeneity check.)

(case "a map with values of two different types is a type error"
  (doc    "`(map (a 1) (b true))` associates `a`→Int64 and `b`→Bool: the values do not share one type,
           so the map is not well-typed and the compiler rejects it (CDZ0201, collections-and-text.md
           #A Map Associates Keys With Values — values of ONE type).")
  (needs      collections)
  (input      (= (map ("a" 1) ("b" true)) (map ("a" 1) ("b" true))))
  (error      CDZ0201))

(case "a map mixing integer and float values is a type error"
  (doc    "The numeric companion: Int64 and Float64 are distinct types that do not silently unify
           (numeric-model.md #Numeric Types Do Not Silently Promote), so `(map (a 1) (b 2.5))` has two
           value types and is ill-typed — CDZ0201. Pins that map value-homogeneity holds across the
           numeric types too, mirroring the list case.")
  (needs      collections)
  (input      (= (map ("a" 1) ("b" 2.5)) (map ("a" 1) ("b" 2.5))))
  (error      CDZ0201))

; The KEY-homogeneity half of the same rule, on the `(map …)` LITERAL path. collections-and-text.md #A Map
; Associates Keys With Values: "A map MUST associate keys of one type with values of one type." The seed
; enforces the VALUES half on the literal (the cases above) and enforces the KEYS half on the `Map.insert`
; path (§"inserting a key of a different type into a map is a type error"), but does NOT enforce key
; homogeneity on the `(map …)` literal: a literal with two keys of DIFFERENT types builds a heterogeneous-
; key map rather than being rejected. `(let ((j 5)) (let ((k true)) (map (j 1) (k 2))))` — the keys are the
; VALUES 5 (Int64) and true (Bool), two types — produces `(map (5 1) (true 2))`, an ill-typed map with two
; key types, instead of CDZ0201. (The keys are bound names so this is independent of the unbound-key→string
; coercion bug; the same holds for Int+String keys, `(map (j 1) (k 2))` with `j`=5, `k`="s".) The
; value-homogeneity check and the `Map.insert` key-homogeneity check both exist; the literal's key-
; homogeneity check is the missing sibling. A generation that does not yet check a map literal's key
; homogeneity declines rather than building a heterogeneous-key map.

(case "a map literal with keys of two different types is a type error"
  (doc    "`(let ((j 5)) (let ((k true)) (map (j 1) (k 2))))` has keys that are the VALUES 5 (Int64) and
           true (Bool) — two types — so the map associates keys of MORE than one type, ill-typed and
           rejected (CDZ0201, collections-and-text.md #A Map Associates Keys With Values — keys of ONE
           type). The key-homogeneity check must fire on the map's CONSTRUCTION — when the heterogeneous
           map is built and RETURNED as the result — exactly as the VALUE-homogeneity check does (a bare
           `(map (a 1) (b true))` returned as the result is rejected 'map values do not share one type').
           The keys are BOUND names, so this is independent of the unbound-name→string coercion. A seed
           that checks key homogeneity only when the map flows into an OPERATION (`Map.size`, `=`) but not
           when it is merely constructed/returned still builds a heterogeneous-key map `(map (5 1) (true
           2))` here — the construction-path half the value check already covers. A generation that does
           not yet check a map literal's key homogeneity on construction declines rather than building a
           heterogeneous-key map.")
  (needs      collections)
  (input      (let ((j 5)) (let ((k true)) (map (j 1) (k 2)))))
  (error      CDZ0201))

; As with a list (the compound-shape homogeneity cases above), map value-homogeneity is by value TYPE:
; two compound values of the same KIND but different SHAPE are different types (type-system.md
; #Structural Values Are Comparable Only When Their Shapes Match). So a map whose values are records
; with different field sets, or tuples of different arities, is not value-homogeneous → CDZ0201,
; applying shape compatibility per value.

(case "a map with record values of different field sets is a type error"
  (doc    "`(record (x 1))` and `(record (y 2))` are different record types (different field-name sets),
           so a map associating them as values is not value-homogeneous — CDZ0201 (collections-and-text.md
           #A Map Associates Keys With Values: values of ONE type). Mirrors the list-of-diff-field-records
           case.")
  (needs      collections)
  (input      (= (map ("a" (record (x 1))) ("b" (record (y 2)))) (map ("a" (record (x 1))) ("b" (record (y 2))))))
  (error      CDZ0201))

(case "a map with tuple values of different arities is a type error"
  (doc    "`(tuple 1 2)` and `(tuple 1 2 3)` are different tuple types (different lengths), so a map
           with them as values is not value-homogeneous — CDZ0201. Pins that the map-value homogeneity
           check compares tuple ARITY, mirroring the list case.")
  (needs      collections)
  (input      (= (map ("a" (tuple 1 2)) ("b" (tuple 1 2 3))) (map ("a" (tuple 1 2)) ("b" (tuple 1 2 3)))))
  (error      CDZ0201))

(case "a map with a duplicate key is a type error"
  (doc    "collections-and-text.md #A Map Associates Keys With Values: \"A map MUST contain each key at
           most once.\" `(map (a 1) (a 2))` repeats the key `a`, so it is ill-typed and the compiler
           rejects it (CDZ0201) rather than build it — a repeated key makes the association ambiguous
           (which value does `a` hold?).")
  (needs      collections)
  (input      (= (map ("a" 1) ("a" 2)) (map ("a" 1) ("a" 2))))
  (error      CDZ0201))

(case "comparing a map to a record is a type error"
  (doc    "Witnesses type-system.md #Structural Values Are Comparable Only When Their Shapes Match: a
           record and a map are DISTINCT types (a record's field set is fixed by its form; a map's key
           set is a collection — core-semantics.md #A Record Has A Fixed Set Of Named Fields,
           collections-and-text.md #A Map Associates Keys With Values). Comparing values of two
           different types is a type error the compiler rejects (CDZ0201), even though they carry the
           same keys mapped to the same values.")
  (needs      collections)
  (input      (= (map ("a" 1) ("b" 2)) (record (a 1) (b 2))))
  (error      CDZ0201))

(case "comparing an empty map to an empty record is a type error"
  (doc    "The degenerate companion of the case above: emptiness does not erase the type distinction.
           An empty map and an empty record are different types (type-system.md #Structural Values Are
           Comparable Only When Their Shapes Match), so the comparison is a type error the compiler
           rejects (CDZ0201).")
  (needs      collections)
  (input      (= (map) (record)))
  (error      CDZ0201))

; --- Two maps with different KEY SETS are comparable: the result is false, not a type error ---
; type-system.md #Structural Values Are Comparable Only When Their Shapes Match restricts comparison
; for RECORDS (by field-name set), TUPLES (by arity), and SUMS (by variant set) — shapes fixed
; statically by a value's FORM. A map's key set is NOT such a shape: a map's type is Map<KeyType,
; ValueType>, and its keys are a runtime COLLECTION, not a fixed part of its type (the existing "map vs
; record" case above turns on exactly this — "a map's key set is a collection"). So two maps with the
; SAME key and value TYPES but DIFFERENT keys are the SAME TYPE, and comparing them is well-typed. Its
; result is fixed by collections-and-text.md #A Map Associates Keys With Values ("Two maps MUST be equal
; exactly when they associate the same keys with equal values"): different keys ⇒ they do not associate
; the same keys ⇒ the comparison is FALSE, not a compile-time rejection. The seed wrongly applies the
; record/tuple/sum shape-match rule to a map's key set and REJECTS the comparison (CDZ0201, "comparison
; between values of different shapes") — a miscompile: it refuses a valid program that must run and
; yield false. The recorded false is the correct oracle; the seed's rejection is the bug.

(case "two maps with different keys are unequal, not a type error"
  (doc    "`(map (a 1) (b 2))` and `(map (a 1) (c 2))` have the same key and value types but different
           key sets. A map's key set is runtime data, not part of its type (unlike a record's fixed
           field set), so the two maps are the SAME type and the comparison is well-typed. They do not
           associate the same keys, so `=` is FALSE (collections-and-text.md #A Map Associates Keys With
           Values), NOT a type error. The seed wrongly treats the key set as a shape and rejects the
           comparison (CDZ0201) — a miscompile that refuses a valid program. MUST be false.")
  (needs      collections)
  (input      (= (map ("a" 1) ("b" 2)) (map ("a" 1) ("c" 2))))
  (output     (: false Bool)))

(case "two maps of different sizes are unequal, not a type error"
  (doc    "`(map (a 1))` and `(map (a 1) (b 2))` differ in their number of entries. A map's entry count
           is runtime data, not part of its type, so the comparison is well-typed and FALSE — they do
           not associate the same keys (collections-and-text.md #A Map Associates Keys With Values). The
           size-difference companion of the case above; the seed rejects it (CDZ0201) rather than
           yielding false — the same miscompile. Contrast records `(= (record (a 1)) (record (a 1) (b
           2)))`, which IS a type error, because a record's field set IS its shape.")
  (needs      collections)
  (input      (= (map ("a" 1)) (map ("a" 1) ("b" 2))))
  (output     (: false Bool)))

(case "an empty map is unequal to a non-empty map, not a type error"
  (doc    "The degenerate companion: an empty map and a one-entry map are the same map type (both
           Map<…>), so comparing them is well-typed and FALSE — they associate different keys. Pins that
           emptiness on one side of a map comparison yields false, not a shape-mismatch rejection
           (contrast the empty-map-vs-empty-record case above, which IS a type error because map and
           record are different types). MUST be false.")
  (needs      collections)
  (input      (= (map) (map ("a" 1))))
  (output     (: false Bool)))

(case "member access on a map is a type error"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field: member access projects
           a field from the RECORD it is applied to; applied to a value that is not a record it is a
           type error. A map is not a record (its keys are a collection, not a fixed field set), so
           `(. m a)` on a map `m` is rejected (CDZ0201) rather than projecting the entry for `a`.")
  (needs      collections)
  (input      (let ((m (map ("a" 1) ("b" 2)))) (. m a)))
  (error      CDZ0201))

; --- Shape-mismatch comparisons within one kind are type errors -------------------------
; type-system.md #Structural Values Are Comparable Only When Their Shapes Match is stronger
; than the cross-KIND cases above (map vs record, list vs tuple): even two values of the SAME
; kind are comparable only when their SHAPES match — two tuples only when their lengths are
; identical, two records only when their field-name sets are identical, two sums only when
; their variant sets are identical. A comparison whose shapes differ is a type error the compiler
; REJECTS (CDZ0201) rather than reporting as unequal.

(case "comparing tuples of different lengths is a type error"
  (doc    "type-system.md #Structural Values Are Comparable Only When Their Shapes Match: two tuples
           are comparable only when their lengths are identical. A 2-tuple and a 3-tuple have
           different shapes, so the comparison is a type error the compiler rejects (CDZ0201).")
  (input      (= (tuple 1 2) (tuple 1 2 3)))
  (error      CDZ0201))

; The shape-match requirement is RECURSIVE: it applies to every corresponding pair of sub-values, not
; only the top-level shape. So comparing two same-shape outer tuples whose corresponding ELEMENTS have
; mismatched shapes (an inner 2-tuple vs an inner 3-tuple, or an inner tuple vs a scalar) is a type
; error (CDZ0201) — the sub-comparison is between incompatible shapes.

(case "a nested tuple-arity mismatch in equality is a type error"
  (doc    "The outer tuples are both 2-tuples (matching shape), but their first elements are a 2-tuple
           and a 3-tuple — mismatched shapes. Shape compatibility is recursive, so this comparison is
           a type error (CDZ0201), the same as the top-level `(= (tuple 1 2) (tuple 1 2 3))` above.")
  (input      (= (tuple (tuple 1 2) 9) (tuple (tuple 1 2 3) 9)))
  (error      CDZ0201))

(case "a nested element kind mismatch in equality is a type error"
  (doc    "The outer tuples match shape, but one's first element is a tuple and the other's is a scalar
           — different types at that position. Recursive shape matching makes this a type error
           (CDZ0201), like the top-level `(= (tuple 1 2) 5)`.")
  (input      (= (tuple (tuple 1 2) 9) (tuple 5 9)))
  (error      CDZ0201))

(case "comparing records with different field-name sets is a type error"
  (doc    "type-system.md #Structural Values Are Comparable Only When Their Shapes Match: two records
           are comparable only when their sets of field names are identical. `(record (a 1))` and
           `(record (b 1))` have disjoint field names — different shapes — so the comparison is a type
           error (CDZ0201).")
  (needs      collections)
  (input      (= (record (a 1)) (record (b 1))))
  (error      CDZ0201))

(case "comparing records whose field sets differ in size is a type error"
  (doc    "type-system.md #Structural Values Are Comparable Only When Their Shapes Match: a record
           with fields {a} and one with fields {a, b} have different field-name sets, hence different
           shapes. The comparison is a type error the compiler rejects (CDZ0201). (Contrast `(record
           (a 1))` vs `(record (a 2))` — same shape, different value — which is an ordinary false, not
           a type error.)")
  (needs      collections)
  (input      (= (record (a 1)) (record (a 1) (b 2))))
  (error      CDZ0201))

(case "comparing sums with disjoint variant sets is a type error"
  (doc    "type-system.md #Structural Values Are Comparable Only When Their Shapes Match: two sums are
           comparable only when their variant sets are identical. An Option value (Some) and a Result
           value (Ok) belong to different sum types, so comparing them is a type error the compiler
           rejects (CDZ0201). (Two values of the SAME sum but different variants — Some vs None — are
           an ordinary false, witnessed elsewhere.)")
  (input      (= (Some 1) (Ok 1)))
  (error      CDZ0201))

(case "two different variants of the SAME sum compare unequal"
  (doc    "The complement of the case above. `Some` and `None` are two variants of ONE sum type
           (Option) — the same variant set — so they are COMPARABLE (type-system.md #Structural
           Values Are Comparable Only When Their Shapes Match: sums are comparable when their variant
           sets are identical). The comparison is well-typed and yields false because the variants
           differ; it is NOT a type error and MUST NOT be rejected. This is the boundary the
           disjoint-variant case above must not over-reach into: differing variant TAG (same sum) is an
           ordinary false; differing variant SET (different sums) is the type error.")
  (input  (= (Some 1) (None unit)))
  (output (: false Bool)))

(case "two variants of the same sum compare unequal regardless of operand order"
  (doc    "The order-flipped companion: `(= (None unit) (Some 1))` is the same well-typed comparison
           yielding false. Pins that neither operand order is mistaken for a shape/type mismatch.")
  (input  (= (None unit) (Some 1)))
  (output (: false Bool)))

(case "the two variants of Result compare unequal"
  (doc    "Same rule for the Result sum: `Ok` and `Err` are two variants of one sum type, so `(= (Ok 1)
           (Err 2))` is a well-typed comparison yielding false — not a type error. Same variant set →
           comparable; the differing tag makes them unequal.")
  (input  (= (Ok 1) (Err 2)))
  (output (: false Bool)))

(case "a match not covering the scrutinee is a compile-time rejection"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected. When the scrutinee's
           variant set is known statically and the arms leave a variant uncovered — only Neg and Pos
           patterns are present for a Sign value — the compiler rejects the non-exhaustive match
           (CDZ0210) before it runs, rather than emit a component that traps on an unmatched value.")
  (input    (match (Sign.Zero unit)
              ((Sign.Neg _) -1)
              ((Sign.Pos _)  1)))
  (error    CDZ0210))

(case "a nullary constructor is a single-arity function taking unit"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (2nd sentence): a 'nullary' variant is a constructor whose argument type
           is Unit. Construction is uniform: (Sign.Zero unit) applies the constructor to unit and
           produces the Sum value. No pre-applied Sums in the prelude — the constructor is the value
           bound to Sign.Zero, not the already-constructed variant.")
  (input  (Sign.Zero unit))
  (output (: (Sign.Zero unit) Sign)))

; --- A nullary variant's argument type is Unit -------------------------------------------
; core-semantics.md #A Sum Type Constructor Is A Single-Arity Function (2nd sentence): "A nullary
; variant MUST be a constructor whose argument type is Unit, not a pre-constructed Sum value." So a
; nullary variant is applied to `unit` — `(None unit)`, `(Sign.Pos unit)` — and applying it to any
; NON-unit value is a type error (the argument type is Unit, and Int64/Bool/etc. do not match), which
; the compiler REJECTS (CDZ0201) — the same class as heterogeneous-list and constructor-over-application.

(case "a nullary variant applied to a non-unit payload is a type error"
  (doc    "`None`'s argument type is Unit (it is Option's nullary variant), so `(None 5)` applies it to
           an Int64 — a type mismatch the compiler rejects (CDZ0201), or declines if it does not yet
           check the nullary variant's Unit argument type. A malformed `(None 5)` would be observable
           (matching `(None n)` binding n=5), a payload a nullary variant must never carry — the
           reason to reject at compile time.")
  (input     (None 5))
  (error     CDZ0201))

(case "a nullary Sign variant applied to a non-unit payload is a type error"
  (doc    "The companion for a user-facing sum: `Sign.Pos` is a nullary variant of Sign (Neg | Zero |
           Pos), so its argument type is Unit and `(Sign.Pos 5)` is a type error (CDZ0201). Pins that
           the Unit argument-type rule for nullary variants holds for every sum, not only Option.")
  (input     (Sign.Pos 5))
  (error     CDZ0201))

; The dual of the nullary-variant rule: a UNARY variant with a DECLARED payload type checks its argument
; against that type, exactly as the nullary variant checks its argument is Unit. core-semantics.md #A Sum
; Type Constructor Is A Single-Arity Function makes a constructor "a single-arity function that, when
; applied to exactly one argument, produces a Sum value" — and a function application type-checks its
; argument (#Applying A Function Binds Its Parameter To Its Argument), so `T.Mk` declared `(Mk Int64)` has
; argument type Int64 and `(T.Mk "x")` applies it to a String — a type mismatch the compiler MUST reject
; (CDZ0201). type-system.md #The Structural Types makes a sum's shape "its variant names with their payload
; types", so a payload of the wrong type is ill-typed. A compiler that constructs the variant without
; checking the payload against the declared type produces an observably ill-typed value — `(T.Mk "x")`
; renders as `(T.Mk "x")`, and matching it binds the String where an Int64 is declared (a downstream
; `(String.byte-len n)` reads it as a String and succeeds, running the ill-typed program). This is the
; typed-payload companion of the nullary-variant cases above (which check the argument is Unit); the
; argument-type check must hold for a declared non-Unit payload too. A generation that does not yet check
; a unary variant's payload type declines rather than constructing the mistyped value.

(case "a unary variant applied to a wrong-type payload is a type error"
  (doc    "`(type T (Mk Int64))` declares `T.Mk` with payload type Int64, so `(T.Mk \"x\")` applies it to a
           String — a type mismatch the compiler MUST reject (CDZ0201, core-semantics.md #A Sum Type
           Constructor Is A Single-Arity Function with #Applying A Function Binds Its Parameter To Its
           Argument: a constructor is a single-arity function whose argument is type-checked, exactly as
           `(f \"x\")` on an Int64-parameter `f` is). This is the typed-payload companion of the
           nullary-variant cases above (`(None 5)`, `(Sign.Pos 5)`, which check the argument is Unit): a
           unary variant's DECLARED payload type is checked just as the nullary variant's Unit type is. A
           compiler that constructs the variant without checking the payload produces an observably
           ill-typed value `(T.Mk \"x\")`, and matching it binds the String where an Int64 is declared. A
           generation that does not yet check a unary variant's payload type declines rather than
           constructing the mistyped value.")
  (needs     sum-type-declaration)
  (input     (do
               (type T (Mk Int64))
               (def (main) (T.Mk "x")) (export main)))
  (error     CDZ0201))

; The unary-variant payload-type check must cover a COMPOUND payload type too — including a TUPLE. A
; variant declared `(Pair (Tuple Int64 Int64))` has payload type `(Tuple Int64 Int64)`, so `(T.Pair (tuple
; 1 2 3))` applies it to a THREE-element tuple where a two-element one is declared — a type mismatch
; (CDZ0201), since a tuple's length is part of its type (type-system.md #A Tuple Is Reshaped Positionally,
; whose length is part of its type). A compiler that checks a scalar/String/List/Record payload (those
; landed) but not a Tuple payload constructs `(T.Pair (tuple 1 2 3))` and lets a downstream `(. p 2)`
; project position 2 — a position the DECLARED two-element payload type does not have — yielding 3, a
; wrong value the declared arity forbids. `(T.Pair 5)` (a scalar where the tuple payload is declared) slips
; through the same way. This is the Tuple-payload sibling of the scalar unary-variant case above: the
; payload-type check must cover every payload type shape, tuple included. A generation that does not yet
; check a tuple-typed payload declines rather than constructing the mistyped value.

(case "a unary variant applied to a wrong-arity tuple payload is a type error"
  (doc    "`(type T (Pair (Tuple Int64 Int64)))` declares `T.Pair` with payload type `(Tuple Int64 Int64)`,
           so `(T.Pair (tuple 1 2 3))` applies it to a three-element tuple where a two-element one is
           declared — a type mismatch the compiler MUST reject (CDZ0201): a tuple's length is part of its
           type (type-system.md #A Tuple Is Reshaped Positionally, #The Structural Types Are Record, Tuple,
           And Sum), so the arities do not unify, exactly as the scalar unary-variant case (`(T.Mk \"x\")`)
           above. Pins that the unary-variant payload-type check covers a COMPOUND (tuple) payload, not
           only scalars/String/List/Record: a compiler that skips the tuple payload constructs `(T.Pair
           (tuple 1 2 3))` and lets a downstream `(. p 2)` project a position the declared two-element
           payload lacks, yielding 3 — a wrong value the declared arity forbids. A generation that does not
           yet check a tuple-typed payload declines rather than constructing the mistyped value.")
  (needs     sum-type-declaration)
  (input     (do
               (type T (Pair (Tuple Int64 Int64)))
               (def (main) (T.Pair (tuple 1 2 3))) (export main)))
  (error     CDZ0201))

(case "a program's unary variant reusing a prelude nullary variant name is unary"
  (doc    "A program declares `(type Expr (Lit Int64 | Neg Expr))` whose `Neg` variant carries a
           payload — reusing the NAME of the prelude `(type Sign (Neg | Zero | Pos))`'s NULLARY `Neg`.
           The program's declaration governs its own `Expr.Neg`: it is UNARY, so `(Expr.Neg (Expr.Lit
           5))` is well-typed (not a nullary-variant-carries-a-payload error), and a recursive fold over
           it computes — `depth` of a singly-negated literal is 1. Pins that a program `(type …)`
           overrides a prelude variant's ARITY by name, so a self-hosted compiler declaring an AST whose
           variant names (`Neg`, `Lit`, `App`, …) happen to collide with prelude ones is not misjudged
           by the collided prelude arity. Without the override the prelude's nullary `Neg` misfires
           CDZ0201 on the program's `(Expr.Neg …)`.")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64) (Neg Expr))
            (def (depth e) (match e ((Expr.Lit n) 0) ((Expr.Neg x) (+ 1 (depth x)))))
            (def (main)    (depth (Expr.Neg (Expr.Lit 5)))) (export main)))
  (output (: 1 Int64)))

(case "a bare nullary constructor is the nullary sum value"
  (doc    "A nullary variant used as a VALUE may be written BARE — `NNil`, not `(Node.NNil unit)`. A
           bare nullary constructor IS the nullary sum value (core-semantics.md #A Sum Type Constructor
           Is A Single-Arity Function: its argument type is Unit), equivalent to `(Ctor unit)`. Matched,
           built at run time, and returned, the bare and applied forms denote one value. `(classify 0)`
           builds `NNil` (bare) and matches its `((Node.NNil _) …)` arm → 1; `(classify 7)` builds
           `(Node.NLit 7)` → 7. Pins that the bare nullary form both constructs and matches — the shape
           a reader takes writing `NNil` for an empty node rather than the verbose `(Node.NNil unit)`.")
  (needs  sum-type-declaration)
  (input  (do
            (type Node (NLit Int64) NNil)
            (def (classify n) (if (= n 0) NNil (Node.NLit n)))
            (def (val x) (match x ((Node.NLit v) v) ((Node.NNil _) 1)))
            (def (main) (+ (val (classify 0)) (val (classify 7)))) (export main)))
  (output (: 8 Int64)))

(case "a match arm building a fresh runtime compound infers its shape"
  (doc    "A non-recursive `emit` dispatches on a sum variant and each arm BUILDS a fresh runtime
           compound (a Bytes value) — `((Expr.Lit n) (Bytes.of (list 66))) / ((Expr.Neg n) (Bytes.of
           (list 124)))`. The match's result shape is the unified shape of its arm bodies (both Bytes),
           inferred exactly as an `if`'s two branches are — so returning it across the run boundary
           renders `b\"B\"`. Pins that a match-arm-returns-fresh-compound needs no `if`-on-discriminant
           workaround: this is the compiler's own emit/lower DISPATCH shape (dispatch on a node's variant,
           build its output bytes).")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64) (Neg Int64))
            (def (emit e) (match e ((Expr.Lit n) (Bytes.of (list 66))) ((Expr.Neg n) (Bytes.of (list 124)))))
            (def (main)   (emit (Expr.Lit 5))) (export main)))
  (output (: b"B" Bytes)))

(case "a recursive lower from a sum to Bytes assembles its output in match arms"
  (doc    "The compiler's emit spine: a recursive `lower : Expr → Bytes` dispatches on each node's
           variant and BUILDS the output bytes in the arm — a `Lit` emits its opcode byte, a `Neg`
           concatenates the lowered child with a suffix byte. `(lower (Neg (Lit 5)))` = `b\"B|\"` (0x42
           'B' for the Lit, 0x7C '|' appended for the Neg). Pins that a match arm may both build a fresh
           compound AND recurse — the shape is inferred and the runtime Bytes assemble correctly, the
           exact shape a self-hosted backend's `lower`/`serialize` takes.")
  (needs  sum-type-declaration)
  (input  (do
            (type Expr (Lit Int64) (Neg Expr))
            (def (lower e) (match e
                             ((Expr.Lit n) (Bytes.of (list 66)))
                             ((Expr.Neg x) (Bytes.concat (lower x) (Bytes.of (list 124))))))
            (def (main)    (lower (Expr.Neg (Expr.Lit 5)))) (export main)))
  (output (: b"B|" Bytes)))

(case "a unary constructor is a single-arity function"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (1st sentence): Some is a single-arity constructor. Applied to an
           argument, it produces a Sum tagged 'Some' carrying that argument as payload.")
  (input  (Some 42))
  (output (: (Some 42) (Option Int64))))

(case "nullary constructor patterns are uniform with unary"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant (4th sentence): patterns are uniform `(Ctor binder)`. A nullary variant
           pattern is `(Sign.Zero _)` (the binder matches unit), not a bare `Sign.Zero`. Uniformity:
           no arity-based special case — all constructor patterns have the same syntactic form. The
           match arms every variant of Sign (Neg | Zero | Pos) so it is exhaustive (#Matching Is
           Exhaustive Or Rejected) — the point here is the uniform pattern SHAPE, not coverage, so it
           covers all three; a match omitting a variant is the separate non-exhaustive case above.")
  (input  (match (Sign.Zero unit)
            ((Sign.Neg _)  0)
            ((Sign.Zero _) 1)
            ((Sign.Pos _)  2)))
  (output (: 1 Int64)))

(case "constructor pattern binders capture the payload uniformly"
  (doc    "Witnesses core-semantics.md #A Sum Type Constructor Is A Single-Arity Function Producing
           The Tagged Variant: the pattern `(Some x)` binds `x` to the payload (42); the pattern
           `(None u)` binds `u` to the payload (unit). Both are single-arity patterns — uniform
           handling, no nullary special case.")
  (input  (match (Some 42)
            ((Some x) x)
            ((None _) 0)))
  (output (: 42 Int64)))

(case "a match over multiple nullary constructors with uniform patterns"
  (doc    "Witnesses core-semantics.md #The Pattern Matcher MUST Handle All Constructor Patterns
           Uniformly: Sign has three nullary variants (Neg, Zero, Pos); each pattern is `(Ctor _)`.
           The pattern matcher does not special-case them vs unary constructors — all constructor
           patterns have identical structure.")
  (input  (match (Sign.Neg unit)
            ((Sign.Neg _)  -1)
            ((Sign.Zero _) 0)
            ((Sign.Pos _)  1)))
  (output (: -1 Int64)))

(case "mixing nullary and unary constructors in one match"
  (doc    "Witnesses core-semantics.md #The Pattern Matcher MUST NOT Special-Case Nullary Vs Unary
           Constructors: a match over Option (which has Some taking a payload and None taking unit)
           uses uniform `(Ctor binder)` syntax for both. No branch distinguishes nullary from unary.")
  (input  (match (Some 5)
            ((Some n)  n)
            ((None _)  0)))
  (output (: 5 Int64)))

(case "the prelude binds Constructor values not pre-applied Sums"
  (doc    "Witnesses core-semantics.md #The Prelude MUST Bind Constructor Values Only: the name
           `None` resolves to a Constructor (a function value), not to a pre-constructed Sum. You
           cannot use bare `None` as a value; you must apply it: `(None unit)`. The bound value is
           the constructor, not the variant.")
  (input  (let ((ctor None)) (ctor unit)))
  (output (: (None unit) (Option Any))))

(case "a sum type is declared with named variants"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable Constructed And Deconstructed (1st
           sentence): a program declares a sum type as a set of named variants. Syntax TBD
           (options/sum-type-declaration/); this case uses (type Color (Red | Green | Blue)) to
           declare Color with three nullary constructors. Each constructor is single-arity taking
           Unit per the uniform constructor requirement. The constructors bind in a Color record:
           Color.Red, Color.Green, Color.Blue. Applying the nullary constructor `(Color.Red unit)`
           yields the Sum value that renders `(Color.Red unit)` — the same `(Variant unit)` form
           every nullary variant takes ((None unit), (Sign.Pos unit)); a nullary variant carries
           unit, so its one canonical value form is the constructor applied to unit, never a bare
           tag (deterministic-value-form.md #A Value Has One Canonical Byte Form).")
  (needs  sum-type-declaration)
  (input  (do
            (type Color Red Green Blue)
            (Color.Red unit)))
  (output (: (Color.Red unit) Color)))

(case "a sum type variant can carry data"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable Constructed And Deconstructed (1st
           sentence: 'each optionally carrying data'). Syntax (type Result (Ok Int64 | Err))
           declares Result where Ok carries an Int64 and Err carries Unit (nullary). Both are
           single-arity: Ok takes Int64, Err takes Unit. Constructors: Result.Ok, Result.Err.")
  (needs  sum-type-declaration)
  (input  (do
            (type Result (Ok Int64) Err)
            (Result.Ok 42)))
  (output (: (Result.Ok 42) Result)))

(case "sum type constructors are in scope after declaration"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable: declaring a sum type binds its
           constructors in the enclosing scope as members of a record named after the type.
           After (type Status (Ready | Waiting)), both Status.Ready and Status.Waiting are
           Constructor values accessible via member access.")
  (needs  sum-type-declaration)
  (input  (do
            (type Status Ready Waiting)
            (match (Status.Ready unit)
              ((Status.Ready _)   1)
              ((Status.Waiting _) 0))))
  (output (: 1 Int64)))

(case "a sum type can mix nullary and payload-carrying variants"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable and the uniform constructor
           requirement: a sum can have both nullary (take Unit) and payload-carrying (take data)
           constructors. (type Maybe (Just Int64 | Nothing)) declares Maybe where Just takes
           Int64 and Nothing takes Unit — both single-arity, uniformly handled.")
  (needs  sum-type-declaration)
  (input  (do
            (type Maybe (Just Int64) Nothing)
            (match (Maybe.Just 7)
              ((Maybe.Just n)    n)
              ((Maybe.Nothing _) 0))))
  (output (: 7 Int64)))

(case "Result type for fallible operations"
  (doc    "The compiler returns Result values for operations that can fail (parse errors, validation).
           Result is a sum type with Ok carrying success and Err carrying failure. Both are single-arity
           constructors. This replaces trapping for recoverable errors.")
  (input  (match (Ok 42)
            ((Ok n)  (+ n 1))
            ((Err _) 0)))
  (output (: 43 Int64)))

(case "Result propagates errors without trapping"
  (doc    "With Result, a function that encounters an error returns (Err ...) instead of trapping.
           The caller matches and decides how to handle it. This is essential for a compiler that
           needs to report diagnostics rather than crash on the first error.")
  (input  (match (Err "parse error")
            ((Ok _)  "success")
            ((Err e) e)))
  (output (: "parse error" String)))

(case "match on a sum value returned by a function selects the runtime variant"
  (doc    "Witnesses core-semantics.md #Sum Types Are Declarable Constructed And Deconstructed and
           #Matching Is Exhaustive Or Rejected for a sum whose VARIANT is determined at RUN TIME: the
           Result/Option idiom a compiler leans on. `classify` returns (Some n) or (None unit) by a
           condition, so its result's variant is not known at compile time; the match must dispatch on
           the runtime variant. classify(5) is (Some 5), so the Some arm binds x=5 and yields 5. The
           existing Result cases above match a directly-written constructor; this pins the far more
           common case where the constructor comes from a call.")
  (input  (do
            (def (classify n) (if (> n 0) (Some n) (None unit)))
            (def (main) (match (classify 5)
                          ((Some x) x)
                          ((None _) 0))) (export main)))
  (output (: 5 Int64)))

(case "match on a function-returned sum takes the other variant at runtime"
  (doc    "The companion of the case above on the other branch: classify(-3) is (None unit), so the
           None arm is selected, yielding 0. Confirms the match follows the runtime variant, not a
           fixed one.")
  (input  (do
            (def (classify n) (if (> n 0) (Some n) (None unit)))
            (def (main) (match (classify -3)
                          ((Some x) x)
                          ((None _) 0))) (export main)))
  (output (: 0 Int64)))

(case "match on a Result returned by a fallible function"
  (doc    "The canonical compiler idiom: a fallible `parse` returns (Ok v) or (Err e) by a condition,
           and the caller matches the runtime Result. parse(5) is (Ok 42), so the Ok arm binds v=42 and
           yields 43. A compiler that reports diagnostics rather than trapping depends on matching a
           Result whose variant is decided at run time.")
  (input  (do
            (def (parse n) (if (= n 0) (Err 1) (Ok 42)))
            (def (main) (match (parse 5)
                          ((Ok v)  (+ v 1))
                          ((Err e) e))) (export main)))
  (output (: 43 Int64)))

; The runtime-sum cases above return one of TWO variants (Some/None, Ok/Err). A classifier that returns
; one of THREE variants selected at run time — the `Sign` (Neg | Zero | Pos) shape — must dispatch to
; the correct arm of a three-way match, not just a two-way one. This pins that runtime variant selection
; and match dispatch scale past two variants: `classify` picks Neg/Zero/Pos by nested `if`, and the
; match distinguishes all three at run time (core-semantics.md #Sum Types Are Declarable Constructed And
; Deconstructed + #Matching Is Exhaustive Or Rejected over a three-variant sum whose variant is runtime).

(case "a runtime three-variant classifier dispatches to the negative arm"
  (doc    "`classify` returns one of three Sign variants by nested `if`; classify(-5) is `(Sign.Neg
           unit)`, so the three-way match selects the Neg arm and yields -1. Pins that a runtime variant
           chosen among THREE (not two) reaches the right arm — the three-variant companion of the
           Some/None and Ok/Err runtime classifiers above.")
  (input  (do
            (def (classify n)
              (if (< n 0) (Sign.Neg unit)
                  (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))))
            (def (main) (match (classify -5)
                          ((Sign.Neg _)  -1)
                          ((Sign.Zero _) 0)
                          ((Sign.Pos _)  1))) (export main)))
  (output (: -1 Int64)))

(case "a runtime three-variant classifier dispatches to the middle arm"
  (doc    "The middle-variant companion: classify(0) is `(Sign.Zero unit)`, selecting the Zero arm for 0.
           Confirms the three-way runtime dispatch reaches the MIDDLE variant, not only the first or
           last — the arm most likely to be mis-ordered in a cascade of comparisons.")
  (input  (do
            (def (classify n)
              (if (< n 0) (Sign.Neg unit)
                  (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))))
            (def (main) (match (classify 0)
                          ((Sign.Neg _)  -1)
                          ((Sign.Zero _) 0)
                          ((Sign.Pos _)  1))) (export main)))
  (output (: 0 Int64)))

(case "unit is the empty tuple"
  (doc    "Witnesses core-semantics.md: unit is the 0-element tuple. There is no separate Unit concept
           — unit and () are the same value. This unifies the type system: nullary constructors take the
           empty tuple, functions that 'return nothing' return the empty tuple.")
  (input  (= unit ()))
  (output (: true Bool)))

(case "a tuple is constructed with positional elements"
  (doc    "Witnesses core-semantics.md: tuples are the product type — fixed-size, heterogeneous,
           positionally accessed. A 2-tuple (pair): (tuple 1 true) produces a value of type
           (Tuple Int64 Bool). Tuples are how multi-field constructors pass 'multiple arguments'
           to a single-arity function/constructor.")
  (input  (tuple 1 true))
  (output (: (tuple 1 true) (Tuple Int64 Bool))))

(case "tuple elements are accessed by index"
  (doc    "Witnesses core-semantics.md: tuple elements are accessed positionally. (. t 0) gets
           the first element, (. t 1) the second, etc. Access is bounds-checked against the
           tuple's statically-known arity — an out-of-bounds index is a type error the compiler
           rejects.")
  (input  (let ((t (tuple 1 "hello" true)))
            (. t 1)))
  (output (: "hello" String)))

(case "a boolean tuple element is projected as the program result"
  (doc    "Witnesses core-semantics.md tuple positional access at a non-Int element type: element 1 of
           `(tuple 42 true)` is the Bool true. Projecting it as the program's result must carry the
           Bool across the run boundary — a tuple element's type is whatever that position holds, not
           uniformly Int64. (Element 0, an Int64, already works; this pins that a Bool element does
           too.)")
  (input  (. (tuple 42 true) 1))
  (output (: true Bool)))

(case "tuples are deconstructed by pattern matching"
  (doc    "Witnesses core-semantics.md: tuple patterns bind positional elements. The pattern
           (tuple a b) binds a and b to the first and second elements respectively. This is how
           you 'multi-argument pattern match' with single-arity constructors — the payload is a tuple.")
  (input  (let ((pair (tuple 3 7)))
            (match pair
              ((tuple a b) (+ a b)))))
  (output (: 10 Int64)))

; --- A pattern is LINEAR: it binds each name at most once ------------------------------------
; core-semantics.md #Bindings Introduced By A Pattern Are Scoped To Its Branch: "A pattern MUST bind
; each name at most once; a pattern that binds the same name more than once MUST be a compile-time
; error (CDZ0102)." So `(tuple x x)` — binding `x` twice — is rejected, rather than silently letting
; the second binder shadow the first (which would make `(tuple x x)` bind `x` to the second element)
; or imposing a hidden equality constraint (matching only a tuple whose two elements are equal). The
; case carries `(needs linear-patterns)`: the seed does not yet enforce pattern linearity — it accepts
; the pattern and lets the second binder shadow — so it SKIPS this case until a generation realizes the
; check; a later generation runs it and produces the CDZ0102 rejection.

(case "a pattern that binds the same name twice is rejected"
  (doc    "`(match (tuple 1 2) ((tuple x x) x) (_ 0))` binds `x` twice in one pattern — not a linear
           pattern — so the compiler rejects it (CDZ0102, core-semantics.md #Bindings Introduced By A
           Pattern Are Scoped To Its Branch), rather than shadowing (which would yield 2) or imposing an
           equality constraint (which would fall through to 0). Pins linearity: a repeated binder is an
           error, not a silent shadow or a hidden equality test.")
  (needs  linear-patterns)
  (input  (match (tuple 1 2) ((tuple x x) x) (_ 0)))
  (error  CDZ0102))

; Linearity holds ACROSS sub-patterns, not only within one flat pattern. core-semantics.md #Patterns
; Compose: "A composed pattern MUST bind the union of its sub-patterns' bindings … and MUST remain linear
; across the whole pattern, so that a name appearing in more than one sub-pattern is the same CDZ0102
; error as one appearing twice in a flat pattern." So `(tuple x (tuple x y))` — `x` bound once at the
; outer position and again inside the nested tuple pattern — is a repeated binder across the composition,
; CDZ0102, exactly as the flat `(tuple x x)` is. This is the nested companion of the flat case above; it
; carries the same `(needs linear-patterns)` gate (the seed does not yet enforce linearity, so it SKIPS —
; and without an explicit nested case a linearity check written only for a flat pattern's immediate binders
; would still accept the cross-sub-pattern repeat, silently shadowing the outer `x`).

(case "a pattern that binds the same name across nested sub-patterns is rejected"
  (doc    "`(match (tuple 1 (tuple 2 3)) ((tuple x (tuple x y)) x) (_ 0))` binds `x` at the outer tuple's
           first position AND inside the nested tuple pattern — a repeated binder across the composition,
           which MUST be rejected (CDZ0102, core-semantics.md #Patterns Compose: linearity holds across the
           whole pattern, a name in more than one sub-pattern is the same error as one twice in a flat
           pattern). Pins that the linearity check descends into sub-patterns, not only the immediate
           binders of one pattern node — a check that scans only a flat pattern's binders would accept this
           and silently shadow the outer `x` (yielding 2). Same `(needs linear-patterns)` gate as the flat
           case; a generation realizing linearity must catch the nested repeat too.")
  (needs  linear-patterns)
  (input  (match (tuple 1 (tuple 2 3)) ((tuple x (tuple x y)) x) (_ 0)))
  (error  CDZ0102))

(case "a recursive sum type works with pattern matching"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable: sum types can be recursive — a variant
           can carry the type itself. (type IntList (Cons (Tuple Int64 IntList) | Nil)) is a linked list.
           Pattern matching deconstructs recursively. This is critical: the AST is a recursive sum type.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (let ((xs (IntList.Cons (tuple 1 (IntList.Cons (tuple 2 (IntList.Nil ())))))))
              (match xs
                ((IntList.Cons (tuple head _)) head)
                ((IntList.Nil _)               0)))))
  (output (: 1 Int64)))

(case "comparing same-shape nominal types is a type error"
  (doc    "Witnesses type-system.md #User Types Are Declarable As Nominal Or Structural. Point and
           Vector share a shape but are distinct nominal types; the compiler tracks nominal identity
           and rejects comparing them (CDZ0202), or declines if it does not yet track nominal tags in
           comparison (reject-don't-miscompile).")
  (input    (= (Point (x 0) (y 0)) (Vector (x 0) (y 0))))
  (error    CDZ0202))

; --- The other half of the nominal boundary: nominal vs the untagged shape ----------------
; type-system.md #Nominal Types Are Not Comparable Across Their Boundary, 2nd sentence: "A
; comparison between a NOMINAL value and the UNDERLYING STRUCTURAL value of the same shape MUST be
; rejected by a type-tracking generation, so that a nominal value never silently compares equal to
; the untagged shape it was declared distinct from." So `(= (Point …) (record …))` — a nominal
; record vs a plain record of the same shape — is a type error the compiler rejects (CDZ0202), just
; as the nominal-vs-nominal case above is.

(case "a nominal record compared to a plain record of the same shape is a type error"
  (doc    "`(Point (x 0) (y 0))` is a nominal record; `(record (x 0) (y 0))` is the untagged structural
           value of the same shape. Comparing them is a type error the compiler rejects (CDZ0202,
           type-system.md #Nominal Types Are Not Comparable Across Their Boundary, 2nd sentence) — a
           nominal value never silently compares equal to the untagged shape it was declared distinct
           from.")
  (input    (= (Point (x 0) (y 0)) (record (x 0) (y 0))))
  (error    CDZ0202))

(case "a plain record compared to a nominal record of the same shape is a type error"
  (doc    "The order-flipped companion: `(= (record …) (Point …))` is the same nominal-boundary
           violation regardless of which operand carries the tag — CDZ0202. Pins that the nominal tag
           is checked on either side of the comparison, not only the left.")
  (input    (= (record (x 0) (y 0)) (Point (x 0) (y 0))))
  (error    CDZ0202))

; --- The nominal boundary holds for user-declared SUM types too, not only nominal records -----------
; type-system.md #Nominal Is An Orthogonal Modifier Over Any Structural Type declares nominal available
; over "record, tuple, or SUM"; #Nominal Types Are Not Comparable Across Their Boundary then makes a
; comparison of two DIFFERENT nominal types a type error (CDZ0202), and #Nominal Is An Orthogonal Modifier
; Over Any Structural Type makes two nominal types "distinct whenever their fully-qualified names differ,
; EVEN WHEN their underlying structures and their declared local names are identical". A user `(type …)` sum is
; nominal (its value renders with its type tag, `(A.Mk 1)`), so two distinct sum types `A` and `B` that
; happen to share a variant name `Mk` are still distinct nominal types — comparing `(A.Mk 1)` with
; `(B.Mk 1)` is the same nominal-boundary violation the Point/Vector record case above pins, and MUST be
; rejected (CDZ0202). A compiler that compares two same-shape sums STRUCTURALLY — matching only on the
; shared variant set `{Mk}` and the payload — silently answers the comparison `false` (the untagged
; structural comparison the nominal boundary forbids), the sum sibling of the nominal-record gap. It is a
; wrong VALUE, not merely a missing rejection: the spec says the comparison must be caught, and `false`
; answers it. A generation that does not yet track nominal tags on a sum in comparison DECLINES rather
; than answering (reject-don't-miscompile) — answering `false` is the failure.

(case "comparing two same-shape nominal sum types is a type error, not false"
  (doc    "`A` and `B` are distinct user-declared sum types that share the variant name `Mk` and the
           payload type `Int64` — but a nominal type's identity is its fully-qualified name, so they are
           distinct nominal types even with identical structure and local variant name (type-system.md
           #Nominal Is An Orthogonal Modifier Over Any Structural Type). Comparing `(A.Mk 1)` with `(B.Mk 1)`
           is therefore a comparison across the nominal boundary — a type error (CDZ0202), exactly as the
           Point/Vector nominal-record case above. This is the SUM sibling of that record case: the
           nominal boundary is checked for a user sum type, not only for a nominal record. A compiler that
           compares the two sums structurally (matching only the shared variant set and payload) answers
           `false` — the untagged structural comparison the nominal boundary forbids, a wrong value the
           spec says must be caught rather than answered. A generation that does not yet track nominal
           tags on a sum declines rather than answering false (reject-don't-miscompile).")
  (needs    sum-type-declaration)
  (input    (do
              (type A (Mk Int64))
              (type B (Mk Int64))
              (= (A.Mk 1) (B.Mk 1))))
  (error    CDZ0202))

; --- A list grows by functional construction (collections-and-text.md #A List Is Grown By Functional
;     Construction) --------------------------------------------------------------------------------
; A list is grown with `List.push` (append an element) and `List.update` (replace the element at an
; index), each producing a NEW list value that leaves its operand unchanged — a list is immutable under
; growth exactly as it is under reading. This is the accumulator a self-hosting compiler builds its
; output in. Crucially, growth does NOT introduce a second sequence type: collections-and-text.md #A
; List's Representation Is Unspecified And Unobservable lets the runtime back a list with any structure
; (a contiguous array for a small or literal one, a structurally-shared persistent tree for a grown one)
; and keep the choice invisible, so a `(list …)` literal and a pushed-onto list are the SAME type and
; render `(list …)` alike. The elements are runtime values (a parameter, a computed value), so the list
; lives on the value heap, not folded. Tagged (needs list-growth).

(case "pushing an element onto a list appends it"
  (doc    "`List.push` is a functional constructor: it produces a NEW list with the element appended.
           Pushing a runtime value `n` onto the empty list `(list)`, then two more elements, yields
           `(list 7 8 9)` for `n=7`. The elements are runtime values (the first is a parameter), so the
           list lives on the value heap, and the whole grown value renders `(list …)` — the same form a
           list literal renders, because growth changes only the representation, not the type.")
  (needs  list-growth)
  (input  (do
            (def (mk n) (List.push (List.push (List.push (list) n) 8) 9))
            (def (main)  (mk 7)) (export main)))
  (output (: (list 7 8 9) (List Int64))))

(case "the length of a grown list is its element count"
  (doc    "`List.len` reads the element count as a scalar — the fold-to-scalar half of the idiom, like
           `Bytes.len`. `(List.len (List.push (List.push (list) n) 8))` = 2 for any `n`. Pins that the
           length operation reads a list however it was built (grown here, not a literal).")
  (needs  list-growth)
  (input  (do
            (def (sz n) (List.len (List.push (List.push (list) n) 8)))
            (def (main)  (sz 7)) (export main)))
  (output (: 2 Int64)))

(case "updating a list index replaces that element, leaving others"
  (doc    "`List.update` is a functional constructor producing a NEW list with one index replaced,
           leaving the operand list unchanged. Updating index 0 of a two-element list to a runtime `n`
           yields `(list 99 2)` for `n=99`. The replace-at-index is defined for the in-bounds index 0.")
  (needs  list-growth)
  (input  (do
            (def (put n) (List.update (List.push (List.push (list) 1) 2) 0 n))
            (def (main)  (put 99)) (export main)))
  (output (: (list 99 2) (List Int64))))

(case "a list built by a runtime-length loop has that many elements"
  (doc    "The genuine self-hosting idiom: a list whose LENGTH is decided at run time, built by a
           recursion that pushes an element per step, then measured. `build` pushes `0,1,…,n-1` onto
           the empty list — how many pushes happen is known only at run time — and `List.len` folds
           the result to a scalar. `(List.len (build (list) 0 5))` = 5. This is exactly how a
           self-hosted compiler accumulates and then measures an output buffer: a runtime-length
           functional sequence consumed to a scalar. Pins that a recursive builder's list return
           value flows through the recursion (its kind converges to a heap value) and is consumable —
           the representation carrying it (a persistent tree) is an unobservable implementation detail.")
  (needs  list-growth)
  (input  (do
            (def (build v i n) (if (< i n) (build (List.push v i) (+ i 1) n) v))
            (def (main)         (List.len (build (list) 0 5))) (export main)))
  (output (: 5 Int64)))

(case "a recursive list accumulator grown by push and returned in the base arm stays a list"
  (doc    "The variant of the recursive-builder above that a compiler's arg-list reader takes, and that
           tests the accumulator's return-kind inference specifically. Here the `list` accumulator `acc`
           is RETURNED UNCHANGED in the base arm (`(if (< n 1) acc …)`) and grown by `List.push` in the
           NON-first argument position of the recursive call (`(build (- n 1) (List.push acc n))`). The
           accumulator's kind must converge to `list`/heap — the base arm returns it, and `List.push`
           yields a list, so the return kind is a list — and `List.len` on the result must read the
           built list's length. `build 3 (list)` pushes 3, 2, 1, so the length is 3. Distinct from the
           case above, where the pushed list is the recursive call's FIRST argument (its kind forced
           positionally); here `acc` is returned bare in the base arm, so a naive inference seeds it
           scalar and `List.push acc n` must UPGRADE that seed to a list rather than the bare return
           collapsing it — the same threaded-accumulator convergence the recursive-sum-consumer case
           below pins for a heap sum, here for a `list` return. This is exactly the reader's
           argument-accumulation loop: `(read-args … i out) = (read-args … (+ i 1) (List.push out
           (read-node …)))`, which builds the `(list Node)` of a call's operands — so a self-hosted
           compiler cannot construct a multi-argument call's arg list until this infers a list return.")
  (needs  list-growth)
  (input  (do
            (def (build n acc) (if (< n 1) acc (build (- n 1) (List.push acc n))))
            (def (main)        (List.len (build 3 (list)))) (export main)))
  (output (: 3 Int64)))

(case "a list built by a recursive push-loop is then iterated by index"
  (doc    "The full arg-list round-trip a self-hosted reader performs, composing the push-BUILD and the
           index-READ: `build` accumulates `[0 1 2]` into a list by a recursive push-loop (the reader's
           `read-args` shape — grow the accumulator per operand), then `sum-at` iterates the built list
           by index (`List.at` + `List.len`, the lowering's arg-walk shape) and sums the elements. The
           `let`-bound built list is both measurable and indexable, and its elements are consumed. `build
           0 3 (list)` = `[0 1 2]`; summed = 0+1+2 = 3. Pins that a list is BUILT by push-recursion AND
           READ by indexed iteration in one program — the two halves of a multi-argument call's argument
           handling (construct the `(list Node)` of operands, then walk it to lower each) working
           together, over a `let`-bound runtime list. Distinct from the build-only case above (which only
           measures the built list's length) and the read-only payload-`List.at` cases (which index a
           pre-built list): this composes build and read, the complete arg-list idiom.")
  (needs  list-growth)
  (input  (do
            (def (build i n out) (if (< i n) (build (+ i 1) n (List.push out i)) out))
            (def (sum-at xs i n) (if (< i n)
                                     (+ (match (List.at xs i) ((Some x) x) ((None _) 0)) (sum-at xs (+ i 1) n))
                                     0))
            (def (main) (let ((xs (build 0 3 (list)))) (sum-at xs 0 (List.len xs)))) (export main)))
  (output (: 3 Int64)))

(case "a list value consumed by two operations in one function is not freed early"
  (doc    "A list is an immutable value: passing the SAME list to two operations must let BOTH observe it
           unchanged — reference counting must not free the shared backing after the first consumer. Here
           `both` receives one list `e` and consumes it TWICE: `(use e 1)` pushes `1` and sums the result,
           `(use e 2)` pushes `2` and sums, and the two are combined under arithmetic. Each `use` sees the
           empty base independently, so `(* 10 (use e 1)) + (use e 2)` = `10*1 + 2` = 12. Pins that a value
           consumed by multiple operations in one scope is dup'd, not freed by the first drop — the Perceus
           reference-counting discipline for a shared immutable heap value (memory-and-resource-model.md).
           Distinct from the single-thread push cases above, which never share a list across two consumers.")
  (needs  list-growth)
  (input  (do
            (def (scan xs k) (match (List.at xs k) ((None _) 0) ((Some h) (+ h (scan xs (+ k 1))))))
            (def (use e n) (scan (List.push e n) 0))
            (def (both e) (+ (* 10 (use e 1)) (use e 2)))
            (def (main) (both (list))) (export main)))
  (output (: 12 Int64)))

(case "a recursive sum consumer whose arguments are recursive sum producers compiles"
  (doc    "The self-hosting compiler's spine: a recursive tree-walk (`lower`) whose arms combine the
           results of TWO recursive self-calls through a second recursive consumer (`code-cat`) that
           threads an accumulator. `code-cat`'s second parameter is a compound value returned unchanged
           in its base arm and passed along in its recursive arm — so it MUST be typed as a heap value,
           the same as its first parameter, or the heap argument at the call site forces the compiler
           to inline an unbounded recursion (a compile-time blowup, not a runtime one). Pins that a
           threaded compound accumulator's kind converges to a heap value regardless of the order its
           constraints are discovered, so every recursive call lowers to a real function call. Folds
           the result to a scalar so the case is representation-independent: `lower` of the tree for
           `(20 + 22)` yields three instructions, whose count is 3.")
  (needs  sum-type-declaration)
  (input  (do
            (type Instr (IConst Int64) IAdd)
            (type Code CNil (CCons (Tuple Instr Code)))
            (type Core (KConst Int64) (KAdd (Tuple Core Core)))
            (def (one i)        (Code.CCons (tuple i (Code.CNil ()))))
            (def (code-cat xs ys)
              (match xs
                ((Code.CNil _)              ys)
                ((Code.CCons (tuple h t))   (Code.CCons (tuple h (code-cat t ys))))))
            (def (len c)
              (match c ((Code.CNil _) 0) ((Code.CCons (tuple h t)) (+ 1 (len t)))))
            (def (lower node)
              (match node
                ((Core.KConst n)         (one (Instr.IConst n)))
                ((Core.KAdd (tuple a b)) (code-cat (lower a) (code-cat (lower b) (one (Instr.IAdd ())))))))
            (def (main) (len (lower (Core.KAdd (tuple (Core.KConst 20) (Core.KConst 22)))))) (export main)))
  (output (: 3 Int64)))

; --- The Map OPERATION surface (collections-and-text.md §A Map Is Built By Functional Construction,
; §Keys Are Compared By Value, §A Map Renders As Its Entries In Canonical Key Order). The map value
; cases above pin equality/homogeneity/key-set on the `(map (k v)…)` LITERAL; these pin the OPERATIONS
; that build and query a map — empty, insert/swap, lookup (fallible → Option), remove/take, size — and
; the canonical render. Keys here are VALUES (integers), compared by value (§Keys Are Compared By
; Value). Gated `(needs maps)`: the compiler does not yet lower `Map.*` to the runtime's persistent map
; ops, so they SKIP until that lands (then each moves to a real value).

(case "inserting into the empty map then looking a key up yields the value"
  (doc    "`Map.empty` is the empty map; `Map.insert` adds an association and produces a NEW map
           (functional construction — collections-and-text.md §A Map Is Built By Functional
           Construction); `Map.lookup` is total, yielding `(Some v)` for a present key (§Indexing And
           Lookup Are Fallible, Not Trapping — the map clause). Here the key `1` maps to `10`.")
  (needs  maps)
  (input  (do (def (main) (Map.lookup (Map.insert Map.empty 1 10) 1)) (export main)))
  (output (: (Some 10) (Option Int64))))

(case "looking up an absent key yields None"
  (doc    "`Map.lookup` on a key the map does not contain yields `(None unit)` — the total-lookup rule
           (collections-and-text.md, the map clause of §Indexing And Lookup Are Fallible, Not
           Trapping): absence is data, not a trap. `2` is not a key of a map that holds only `1`.")
  (needs  maps)
  (input  (do (def (main) (Map.lookup (Map.insert Map.empty 1 10) 2)) (export main)))
  (output (: (None unit) (Option Int64))))

; A `Map.lookup` result MUST match its true variant regardless of how the map was constructed. A map built
; by a `(map …)` LITERAL with a RUN-TIME-computed key is mis-represented: `Map.lookup` on it renders the
; correct `(Some v)` when returned directly, but MATCHING that result dispatches to the `None` arm — a
; wrong-arm miscompile. `(let ((j (+ 2 3))) (match (Map.lookup (map (j 1)) 5) ((Some v) v) ((None _) -1)))`
; yields -1 though key 5 (= `(+ 2 3)`) is present with value 1, so the `(Some v)` arm binds v=1 and the
; result MUST be 1. The same map built with a CONST key (`(let ((j 5)) (map (j 1)))`) matches correctly
; (→1), and a `Map.insert`-built map (even with a computed key) matches correctly (→1) — only the
; computed-key `(map …)` LITERAL produces a map whose lookup Option mis-dispatches. This is the same
; under-realized computed-key-map-literal defect as the const/runtime map-equality miscompile (§"a map with
; a computed key equals the same map with a constant key"): the literal path with a runtime key builds a
; map that does not behave as a proper runtime map (its lookup result carries a variant tag the match
; misreads). A generation whose computed-key map literal is not yet a proper runtime map declines rather
; than mis-dispatching its lookup result.

(case "matching a lookup from a computed-key map literal selects the present-value arm"
  (doc    "A map built by a `(map …)` literal with a run-time-computed key `(+ 2 3)` (= 5) holds key 5 ↦ 1;
           `(Map.lookup … 5)` is `(Some 1)`, so matching it MUST bind the `(Some v)` arm's v=1 and yield 1
           (core-semantics.md #Matching Is Exhaustive Or Rejected — the first matching arm; the scrutinee
           is a Some). The seed yields -1: the computed-key map LITERAL is mis-represented, so the lookup's
           Option mis-dispatches to the `None` arm even though the value is present (the lookup renders
           `(Some 1)` when returned directly, so the value is right — only the match reads the wrong
           variant). The same map with a CONST key matches correctly (→1), and a `Map.insert`-built map
           matches correctly (→1) — only the computed-key `(map …)` literal is broken, the same under-
           realized computed-key-literal defect behind the const/runtime map-equality miscompile. A
           generation whose computed-key map literal is not yet a proper runtime map declines rather than
           mis-dispatching.")
  (needs  maps)
  (input  (do
            (def (main)
              (let ((j (+ 2 3)))
                (match (Map.lookup (map (j 1)) 5)
                  ((Some v) v)
                  ((None _) -1)))) (export main)))
  (output (: 1 Int64)))

(case "inserting a key already present replaces its value, not the size"
  (doc    "Adding a key that is already present REPLACES its value rather than adding a second entry
           (collections-and-text.md §A Map Is Built By Functional Construction, preserving each key at
           most once). After inserting `1↦10` then `1↦99`, `Map.size` is 1 and the key holds 99.")
  (needs  maps)
  (input  (do
            (def (main) (Map.size (Map.insert (Map.insert Map.empty 1 10) 1 99))) (export main)))
  (output (: 1 Int64)))

(case "removing a key drops its association and the size"
  (doc    "`Map.remove` produces a NEW map without the key (functional construction), and removing a
           key the map holds lowers the size by one. Two keys inserted, one removed → size 1.")
  (needs  maps)
  (input  (do
            (def (main)
              (Map.size (Map.remove (Map.insert (Map.insert Map.empty 1 10) 2 20) 1))) (export main)))
  (output (: 1 Int64)))

(case "removing an absent key leaves the map unchanged"
  (doc    "Removing a key the map does not contain yields a map equal to the operand rather than
           trapping (collections-and-text.md §A Map Is Built By Functional Construction — removal is
           total). Size is unchanged at 1.")
  (needs  maps)
  (input  (do (def (main) (Map.size (Map.remove (Map.insert Map.empty 1 10) 2))) (export main)))
  (output (: 1 Int64)))

(case "the value-yielding insert reports the value it replaced"
  (doc    "`Map.swap` is the value-yielding add: it produces `(tuple <prior-value-optional> <new-map>)`
           (collections-and-text.md §A Map Is Built By Functional Construction — the two-form rule).
           Replacing key `1`'s value `10` with `99` reports the prior `(Some 10)`; here we project that
           optional. Adding a NEW key would report `(None unit)`.")
  (needs  maps)
  (input  (do
            (def (main) (. (Map.swap (Map.insert Map.empty 1 10) 1 99) 0)) (export main)))
  (output (: (Some 10) (Option Int64))))

(case "the value-yielding remove reports the value it dropped"
  (doc    "`Map.take` is the value-yielding remove: it produces `(tuple <removed-value-optional>
           <new-map>)`. Removing the present key `1` (value `10`) reports `(Some 10)`; taking an ABSENT
           key reports `(None unit)` and leaves the map unchanged (removal is total). Here we project
           the reported optional.")
  (needs  maps)
  (input  (do
            (def (main) (. (Map.take (Map.insert Map.empty 1 10) 1) 0)) (export main)))
  (output (: (Some 10) (Option Int64))))

; A map operation applies to a map that arrives through a FUNCTION PARAMETER, not only a map constructed
; inline in the same expression. Every OTHER heap collection already supports this — `(def (f xs) (List.len
; xs))`, `(def (f b) (Bytes.len b))`, and `(def (f s) (String.byte-len s))` all compile and run when the
; collection is a parameter — so a map, an ordinary heap value (collections-and-text.md #A Map Is Built By
; Functional Construction; memory-and-resource-model.md — a map is a heap value like a list), must too:
; `(def (f mp) (Map.size mp))` applied to a map is a well-typed program. The seed lowers `Map.*` for a map
; built inline (`(Map.size (Map.insert Map.empty 1 10))` works) but declines a `Map.*` operation whose map
; operand is a parameter ("unsupported dotted-application") — the map is the ONLY heap collection whose
; operations do not yet accept a parameter operand, blocking the ordinary idiom of threading a map (an
; environment, a symbol table) through a function or a recursive accumulator, which a self-hosted compiler
; is written in (its `List` accumulator equivalent already works). It is decline-don't-miscompile-safe (an
; honest decline, no wrong value), so a generation that does not yet lower a `Map.*` operation on a
; parameter operand declines rather than miscompiling.

(case "a map operation applies to a map passed as a function parameter"
  (doc    "`(def (count mp) (Map.size mp))` takes a map as a PARAMETER and reads its size; applied to a
           two-entry map it yields 2. Pins that a `Map.*` operation accepts a map operand that arrives
           through a function boundary, not only one constructed inline — every other heap collection
           already does (`List.len`/`Bytes.len`/`String.byte-len` on a parameter all compile), and a map is
           an ordinary heap value, so threading a map through a function (an environment or symbol table, a
           self-hosted compiler's core idiom — its List-accumulator equivalent works) must compile. The
           seed lowers `Map.*` on an inline-built map but declines it on a parameter map ('unsupported
           dotted-application') — the map is the only heap collection whose operations do not yet take a
           parameter operand. A generation that does not yet lower a `Map.*` on a parameter operand declines
           rather than miscompiling.")
  (needs  maps)
  (input  (do
            (def (count mp) (Map.size mp))
            (def (main) (count (Map.insert (Map.insert Map.empty 1 10) 2 20))) (export main)))
  (output (: 2 Int64)))

(case "a built map renders its entries in canonical key order"
  (doc    "A map returned as the program RESULT renders its entries as key-value pairs in the
           deterministic canonical key order (collections-and-text.md §A Map Renders As Its Entries In
           Canonical Key Order, §Map Iteration Is Deterministic) — independent of insertion order, and
           distinguishable from a record. Inserting 2↦20 then 1↦10 renders in key order.")
  (needs  maps)
  (input  (do (def (main) (Map.insert (Map.insert Map.empty 2 20) 1 10)) (export main)))
  (output (: (map (1 10) (2 20)) (Map Int64 Int64))))

; Map homogeneity (collections-and-text.md #A Map Associates Keys With Values: "A map MUST associate keys
; of one type with values of one type") holds under `Map.insert` too, not only for a map literal. The
; literal already rejects a map whose values disagree in type (§"a map with values of two different types
; is a type error", above); `Map.insert` produces a NEW MAP VALUE (#A Map Is Built By Functional
; Construction), which must satisfy the same rule. So inserting an association whose value type differs
; from the map's value type — `(Map.insert (Map.insert Map.empty 1 10) 2 true)` puts a Bool value into an
; Int64-valued map — is a type error (CDZ0201), the map analogue of the `List.push` homogeneity case. An
; insert that skips the check builds a map with values of two types, the value-side of the association the
; homogeneity rule forbids. A generation that does not yet check the inserted value's type declines rather
; than building the mixed-value map.

(case "inserting a value of a different type into a map is a type error"
  (doc    "`(Map.insert (Map.insert Map.empty 1 10) 2 true)` inserts a Bool value into a map whose values
           are Int64 — the result associates values of two types, which a map may not (collections-and-text.md
           #A Map Associates Keys With Values), a type error (CDZ0201), exactly as the `(map (a 1) (b true))`
           literal is rejected (§\"a map with values of two different types is a type error\"). `Map.insert`
           produces a new map value and must satisfy the same one-value-type rule the literal does — the
           functional-construction companion, mirroring the `List.push`/`List.update` homogeneity cases. A
           generation that does not yet check the inserted value's type declines rather than building it.")
  (needs  maps)
  (input  (do (def (main) (Map.insert (Map.insert Map.empty 1 10) 2 true)) (export main)))
  (error  CDZ0201))

(case "inserting a key of a different type into a map is a type error"
  (doc    "The key-side companion: `(Map.insert (Map.insert Map.empty 1 10) true 20)` inserts a Bool KEY
           into a map whose keys are Int64 — a map associates keys of ONE type (collections-and-text.md #A
           Map Associates Keys With Values), so mixing an Int64 key with a Bool key is a type error
           (CDZ0201). `Map.insert` must enforce the key's type against the map's key type as the value case
           does. A generation that does not yet check the inserted key's type declines rather than building
           the mixed-key map.")
  (needs  maps)
  (input  (do (def (main) (Map.insert (Map.insert Map.empty 1 10) true 20)) (export main)))
  (error  CDZ0201))

; --- Map PATTERN matching (ask-61) — a SEPARATE phase, gated `(needs map-patterns)`. A map's key set is
; a RUNTIME collection, not a static shape, so a map pattern is a KEY-DIRECTED LOOKUP: `(map (k p) .. rest)`
; matches when the map HAS key `k` bound to a value matching `p`, binding `rest` to the remaining map. This
; is a QUERY (lowering to `Map.lookup` per key + `Map.remove` for the rest), distinct from the structural
; tuple/sum/list patterns. `Map.lookup`→Option already expresses "match on a key's presence"; these pin the
; PATTERN sugar. Skip until the clause + lowering land (a later phase after the `Map.*` ops).

(case "a map pattern matches a present key and binds its value"
  (doc    "`(map (k p) …)` is a key-directed lookup pattern (core-semantics.md §A Map Is Matched By
           Key-Directed Patterns, ask-61): the arm matches because key `1` is present, binding `v` to
           its value `10`. The catch-all covers the non-match — exhaustiveness needs it, since a map's
           key set is unbounded (no shape to cover).")
  (needs  map-patterns)
  (input  (do
            (def (main)
              (match (Map.insert Map.empty 1 10)
                ((map (1 v)) v)
                (_           0))) (export main)))
  (output (: 10 Int64)))

(case "a map pattern falls through when the key is absent"
  (doc    "The companion: the `(map (2 v))` arm does NOT match a map lacking key `2`, so the match falls
           through to the catch-all — the key-directed pattern is a genuine presence test, not a blanket
           match. Pins the absent-key non-match.")
  (needs  map-patterns)
  (input  (do
            (def (main)
              (match (Map.insert Map.empty 1 10)
                ((map (2 v)) v)
                (_           99))) (export main)))
  (output (: 99 Int64)))

(case "a map pattern binds the rest of the map after the named key"
  (doc    "`(map (1 v) .. rest)` binds `v` to key 1's value AND `rest` to the map with key 1 removed
           (the operand minus the named keys — the map analogue of the list `.. rest`, ask-61). Here
           `rest` still holds key 2, so its size is 1.")
  (needs  map-patterns)
  (input  (do
            (def (main)
              (match (Map.insert (Map.insert Map.empty 1 10) 2 20)
                ((map (1 v) .. rest) (Map.size rest))
                (_                   0))) (export main)))
  (output (: 1 Int64)))

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
           `tuple.N`, not `.field`). So `(. (tuple 1 2) f)` is member access on a non-record: a type
           error the compiler rejects (CDZ0201).")
  (input     (. (tuple 1 2) f))
  (error     CDZ0201))

(case "member access on a string is a type error"
  (doc    "A String is not a record, so `(. \"hi\" x)` is member access on a non-record: a type error
           the compiler rejects (CDZ0201).")
  (input     (. "hi" x))
  (error     CDZ0201))

; The positional tuple accessor `tuple.N` requires a TUPLE operand, exactly as member access `.`
; requires a record operand (above). Applying `tuple.N` to a non-tuple — a scalar, a record, a sum —
; has no defined result, so the compiler MUST reject it (CDZ0201) rather than emit a component that
; traps: the same projection-on-the-wrong-kind class as member access, for positional access.

(case "tuple access on a non-tuple is a type error"
  (doc    "`(tuple.0 5)` projects positional element 0 of the Int64 `5`, which is not a tuple — a type
           error the compiler rejects (CDZ0201), just as `(. 5 x)` (member access on a non-record) is
           rejected above.")
  (input     (tuple.0 5))
  (error     CDZ0201))

(case "tuple access on a record is a type error"
  (doc    "The record-operand companion: a record has named fields, not positional elements, so
           `(tuple.0 (record (a 1)))` applies a positional accessor to a non-tuple — a type error
           (CDZ0201), the mirror of `(. (tuple 1 2) f)` (member access on a tuple) above. Pins that
           `tuple.N` requires a tuple, rejecting a record operand.")
  (needs     collections)
  (input     (tuple.0 (record (a 1))))
  (error     CDZ0201))

(case "member access of a missing field traps"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field (3rd sentence):
           projecting a field the record does not contain traps rather than producing an
           unspecified value.")
  (input  (let ((p (record (x 1)))) (. p z)))
  (trap   "no such field"))

(case "member access on a record chosen by a conditional projects the field"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field with a record value that
           is not written inline but SELECTED at run time by a conditional. Both branches yield a
           record with field `a`; `(. <if…> a)` projects `a` from whichever record the condition
           selects. With the condition true (n=0) the first record is chosen, so the field is 1 — the
           access must project the field, not trap. The record is a genuine record however it was
           produced; member access does not require the record to be a compile-time literal.")
  (input  (module m
            (def (f n) (. (if (= n 0) (record (a 1)) (record (a 2))) a))
            (def (main) (f 0))))
  (output (: 1 Int64)))

(case "member access on a conditionally-chosen record, other branch"
  (doc    "The companion of the case above with the condition false (n=9): the second record is
           selected, so the field `a` projects to 2. Confirms the projection follows the runtime
           branch selection, not a fixed branch.")
  (input  (module m
            (def (f n) (. (if (= n 0) (record (a 1)) (record (a 2))) a))
            (def (main) (f 9))))
  (output (: 2 Int64)))

(case "tuple access on a tuple chosen by a conditional projects the element"
  (doc    "Witnesses core-semantics.md tuple positional access with a tuple value SELECTED at run time
           by a conditional. Both branches yield a 2-tuple; `(tuple.0 <if…>)` projects element 0 of
           whichever tuple the condition selects. With n=0 the first tuple is chosen, so element 0 is
           1 — the access must project it, not trap. Same requirement as the record case: a positional
           access works on a tuple however it was produced.")
  (input  (module m
            (def (f n) (tuple.0 (if (= n 0) (tuple 1 9) (tuple 2 9))))
            (def (main) (f 0))))
  (output (: 1 Int64)))

; --- Positional access on a RUNTIME tuple bound by `let` -------------------------------------------
; A tuple returned from a function and BOUND BY `let` is a genuine runtime value (a value-heap
; positional array), not a compile-time structure. `tuple.N` on such a bound name reads element N from
; the heap array (`arr-get`), unboxing a scalar element to its kind and keeping a compound element as a
; handle. This is the shape a recursive-descent decoder takes — threading a `(node, next-index)` pair
; through `let` — so it must both project a scalar element and yield a compound element a `match`
; consumes. (Without the runtime path a `tuple.N` on a let-bound runtime tuple emitted an unreachable
; that trapped; these pin the arr-get lowering.)

(case "a scalar element is projected from a let-bound runtime tuple"
  (doc    "`mk` returns a runtime tuple `(tuple (NLit 5) 9)` — a genuine value-heap value because its
           first element is a runtime sum. Bound by `let`, `tuple.1` reads its second element, the Int64
           `9`. Pins that positional access on a materialized runtime tuple reads and unboxes a scalar
           element (`arr-get` + `get-int`), the index half of a decoder threading `(node, index)`.")
  (needs  sum-type-declaration)
  (input  (module m
            (type Node (NLit Int64 | NAdd (Tuple Node Node)))
            (def (mk) (tuple (NLit 5) 9))
            (def (main) (let ((l (mk))) (tuple.1 l)))))
  (output (: 9 Int64)))

(case "a compound element projected from a let-bound runtime tuple is matched"
  (doc    "The companion where the projected element is itself a runtime compound: `tuple.0` of the
           let-bound tuple is the `Node` sum `(NLit 5)`, which a `match` then consumes to its scalar
           payload 5. Pins that a runtime `tuple.N` yields a heap element a `match` can dispatch on —
           the node half of a decoder's `(node, index)` pair (the exact shape a `bytes → AST` reader
           threads through `let`).")
  (needs  sum-type-declaration)
  (input  (module m
            (type Node (NLit Int64 | NAdd (Tuple Node Node)))
            (def (mk) (tuple (NLit 5) 9))
            (def (ev e) (match e ((Node.NLit v) v) ((Node.NAdd (tuple a b)) (+ (ev a) (ev b)))))
            (def (main) (let ((l (mk))) (ev (tuple.0 l))))))
  (output (: 5 Int64)))

(case "a scalar element is projected directly from a function's runtime tuple result"
  (doc    "The `let`-free companion of the projected-element cases above: `tuple.N` applied DIRECTLY to a
           NAMED-def call that returns a runtime tuple, with no intervening `let`. `(dec 4)` returns
           `(tuple 40 5)`; `(tuple.0 (dec 4))` projects 40. Pins that positional access on a runtime
           tuple does not depend on the tuple first being `let`-bound — the `let`-bound cases above
           compile, and so must the direct projection (the shape a reader takes to read just one half of
           a returned pair). Distinct from the inline-tuple case `(tuple.1 (tuple n (+ n 1)))` (a tuple
           built right at the projection) and the lambda case `(tuple.0 ((fn …) …))` (compile-time
           reduced): here the tuple comes from a NAMED def, which the compiler does not reduce, so the
           projection must recover the operand's shape at the projection site — earlier this emitted an
           invalid component (a decline-don't-miscompile violation), now fixed.")
  (input  (module m
            (def (dec i) (tuple (* i 10) (+ i 1)))
            (def (main) (tuple.0 (dec 4)))))
  (output (: 40 Int64)))

(case "a scalar element is projected DIRECTLY from a named function's runtime tuple result"
  (doc    "The `let`-free companion: `tuple.0` applied DIRECTLY to a named-def function's runtime-tuple
           result — `(tuple.0 (dec 4))` with no intervening `let` — projects the scalar element. `dec`
           builds a runtime tuple `(tuple (* n 10) 9)`; element 0 is `(* 4 10)` = 40. Pins that the
           projection recovers the operand's shape at the PROJECTION site, not only at a `let`-binding
           site, so a named-def result is projectable like a `let`-bound one. (This directly built a
           runtime tuple constructor into an import-free scalar module — an INVALID component — before
           the runtime constructor learned to decline on the scalar path and defer to the runtime pass:
           decline-don't-miscompile, then compile.)")
  (input  (module m
            (def (dec n) (tuple (* n 10) 9))
            (def (main) (tuple.0 (dec 4)))))
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
  (input  (module m
            (type Node (NInt Int64 | NPrim (Tuple String Node Node)))
            (type Core (KConst Int64 | KAdd (Tuple Core Core)))
            (def (bad) (Bytes.len (Bytes.of (list 256))))
            (def (resolve node)
              (match node
                ((Node.NInt n) (Core.KConst n))
                ((Node.NPrim (tuple h a b))
                  (if (= h "+") (Core.KAdd (tuple (resolve a) (resolve b)))
                                (Core.KConst (bad))))))
            (def (kind-of c) (match c ((Core.KConst n) 0) (_ 1)))
            (def (main) (kind-of (resolve (Node.NPrim (tuple "+" (Node.NInt 20) (Node.NInt 22))))))))
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
  (input  (module m
            (def (mk n) (Some (record (a n) (b (+ n 1)))))
            (def (main)
              (match (mk 41)
                ((Some r) (. r b))
                ((None u) 0)))))
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
  (input  (module m
            (def (mk n) (Some (record (a n) (b (+ n 1)))))
            (def (main) (. (Option.expect (mk 41) "x") b))))
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
  (input  (module m
            (def (mk n) (Ok (record (a n) (b (+ n 1)))))
            (def (main) (. (Result.expect (mk 41) "x") b))))
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
  (input  (module m
            (def (f n) (tuple n 1))
            (def (main) (f 3))))
  (output (: (tuple 3 1) (Tuple Int64 Int64))))

(case "a constant tuple is returned as a program result"
  (doc    "The control the case above must match: a compile-time-known `(tuple 3 1)` returns fine
           through the resource-with-display output ABI. The runtime-element tuple must reach the same
           result; the difference is only whether an element is known at compile time.")
  (input  (module m
            (def (main) (tuple 3 1))))
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
  (input  (module m
            (def (f a b) (tuple (+ a 1) (* b 2)))
            (def (main) (f 3 4))))
  (output (: (tuple 4 8) (Tuple Int64 Int64))))

(case "a three-element runtime tuple is returned as a result"
  (doc    "`(tuple n (+ n 1) (+ n 2))` with n=10 produces `(tuple 10 11 12)`. Pins that a runtime tuple
           of arity > 2 lays out and returns all its elements — a construction that hard-coded a pair
           would drop the third.")
  (input  (module m
            (def (f n) (tuple n (+ n 1) (+ n 2)))
            (def (main) (f 10))))
  (output (: (tuple 10 11 12) (Tuple Int64 Int64 Int64))))

(case "a runtime tuple with a boolean element is returned as a result"
  (doc    "`(tuple n (= n 0))` with n=0 produces `(tuple 0 true)` — a mixed Int64/Bool runtime tuple. A
           tuple element's type is whatever that position holds, so the heap layout must carry a Bool
           beside an Int64 and render both across the run boundary (the runtime companion of the
           constant boolean-tuple-element case). Pins that a runtime compound is not uniformly Int64.")
  (input  (module m
            (def (f n) (tuple n (= n 0)))
            (def (main) (f 0))))
  (output (: (tuple 0 true) (Tuple Int64 Bool))))

(case "a nested runtime tuple is returned as a result"
  (doc    "`(tuple n (tuple n n))` with n=2 produces `(tuple 2 (tuple 2 2))` — a runtime tuple whose
           element is itself a runtime tuple (a heap cell referencing another). Pins that runtime
           compound construction nests: an inner heap value is built and referenced by the outer, and
           the whole structure renders across the run boundary.")
  (input  (module m
            (def (f n) (tuple n (tuple n n)))
            (def (main) (f 2))))
  (output (: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "an element is projected from a runtime-constructed tuple"
  (doc    "`(tuple.1 (tuple n (+ n 1)))` with n=5 projects element 1 of a runtime-built tuple, yielding
           6. Pins that positional access reads the correct element of a heap-allocated runtime tuple —
           a layout or offset error would return the wrong element (5) or a garbage value. Companion of
           the constant tuple-access cases, on the runtime construction path.")
  (input  (module m
            (def (f n) (tuple.1 (tuple n (+ n 1))))
            (def (main) (f 5))))
  (output (: 6 Int64)))

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
  (input  (module m
            (def (f n) (record (a n) (b 1)))
            (def (main) (f 3))))
  (output (: (record (a 3) (b 1)) (Record (a Int64) (b Int64)))))

(case "record fields render in canonical (key-sorted) order regardless of source order"
  (doc    "`(record (b n) (a 1))` with n=2 renders `(record (a 1) (b 2))` — fields in sorted key
           order, not source order (deterministic-value-form.md). Pins that the runtime array slots
           and the emitted renderer AGREE on the sorted field order, so a field value lands under its
           correct name; a slot/name misalignment would render `(record (a 2) (b 1))`.")
  (input  (module m
            (def (f n) (record (b n) (a 1)))
            (def (main) (f 2))))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "a list with a runtime element is returned as a program result"
  (doc    "`(list n 2 3)` with n=1 produces `(list 1 2 3)` — a list one of whose elements is a runtime
           value. Pins that a list is constructed on the value heap and rendered `(list …)`,
           distinct from a tuple's `(tuple …)` though the underlying heap array is identical (the
           distinction is the static type the renderer walks).")
  (input  (module m
            (def (f n) (list n 2 3))
            (def (main) (f 1))))
  (output (: (list 1 2 3) (List Int64))))

(case "a record whose field is a runtime tuple nests across the boundary"
  (doc    "`(record (x n) (y (tuple n 1)))` with n=5 produces `(record (x 5) (y (tuple 5 1)))` — a
           record field that is itself a runtime compound. Pins that the type-directed renderer
           recurses through a heterogeneous nesting (record → tuple), dispatching each sub-shape to
           its own renderer.")
  (input  (module m
            (def (f n) (record (x n) (y (tuple n 1))))
            (def (main) (f 5))))
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
  (input  (module m
            (def (f n) (Some n))
            (def (main) (f 42))))
  (output (: (Some 42) (Option Int64))))

(case "a conditionally-selected variant is returned as a runtime sum result"
  (doc    "`(if (= n 0) (None unit) (Some n))` with n=5 produces `(Some 5)` — the branch selects which
           variant is built at run time, so the discriminant is genuinely runtime data. Pins that the
           renderer's discriminant switch recovers the correct variant name for whichever arm ran; the
           n=0 companion below takes the `None` arm.")
  (input  (module m
            (def (f n) (if (= n 0) (None unit) (Some n)))
            (def (main) (f 5))))
  (output (: (Some 5) (Option Int64))))

(case "a conditionally-selected nullary variant is returned as a runtime sum result"
  (doc    "The `None` companion of the case above: with n=0 the branch selects `(None unit)`, whose
           canonical form is `(None unit)` (a nullary variant carries the unit value). Pins that the
           runtime discriminant switch renders a nullary variant's name and its unit payload correctly,
           distinct from the `Some` arm.")
  (input  (module m
            (def (f n) (if (= n 0) (None unit) (Some n)))
            (def (main) (f 0))))
  (output (: (None unit) (Option Any))))

(case "a runtime sum whose payload is a runtime tuple nests across the boundary"
  (doc    "`(Some (tuple n 1))` with n=7 produces `(Some (tuple 7 1))` — a runtime sum carrying a
           runtime compound payload. Pins that the type-directed renderer recurses from the sum's
           payload into the tuple renderer, dispatching each sub-shape, and that construction nests a
           heap tuple inside a heap sum.")
  (input  (module m
            (def (f n) (Some (tuple n 1)))
            (def (main) (f 7))))
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
  (input  (module m
            (def (unwrap o d) (match o ((Some x) x) ((None _) d)))
            (def (main) (unwrap (Some 42) 99))))
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
  (input  (module m
            (def (unwrap o d) (match o ((Some x) x) ((None _) d)))
            (def (at b i)     (unwrap (Bytes.at b i) -1))
            (def (main)       (at (Bytes.of (list 10 20 30)) 1))))
  (output (: 20 Int64)))

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
  (input  (module m
            (type IntList (Cons (Tuple Int64 IntList) | Nil))
            (def (sm xs) (match xs
                           ((IntList.Cons (tuple h t)) (+ h (sm t)))
                           ((IntList.Nil _)            0)))
            (def (build n) (IntList.Cons (tuple n (IntList.Cons (tuple 8 (IntList.Cons (tuple 2 (IntList.Nil ()))))))))
            (def (main) (sm (build 5)))))
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
  (input  (module m
            (type IntList (Cons (Tuple Int64 IntList) | Nil))
            (def (count n) (if (< n 1)
                               (IntList.Nil ())
                               (IntList.Cons (tuple n (count (- n 1))))))
            (def (sm xs) (match xs
                           ((IntList.Cons (tuple h t)) (+ h (sm t)))
                           ((IntList.Nil _)            0)))
            (def (main) (sm (count 5)))))
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
  (input  (module m
            (type FL (FNil | FCons (Tuple Int64 FL)))
            (def (recompute funcs out)
              (match funcs
                ((FL.FNil _)            out)
                ((FL.FCons (tuple h t)) (recompute t (List.push out h)))))
            (def (main) (List.len (recompute (FL.FCons (tuple 5 (FL.FCons (tuple 6 (FL.FNil ()))))) (list))))))
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
  (input  (module m
            (def (loop xs passes)
              (if (< passes 1) xs (loop (List.push xs 9) (- passes 1))))
            (def (main) (List.len (loop (list 1 2 3) 2)))))
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
  (input  (module m
            (type FL (FNil | FCons (Tuple Int64 FL)))
            (def (recompute funcs out)
              (match funcs
                ((FL.FNil _)            out)
                ((FL.FCons (tuple h t)) (recompute t (List.push out 7)))))
            (def (iterate funcs ktab passes)
              (if (< passes 1) ktab (iterate funcs (recompute funcs (list)) (- passes 1))))
            (def (main) (List.len (iterate (FL.FCons (tuple 1 (FL.FNil ()))) (list) 2)))))
  (output (: 1 Int64)))

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
           yet be matched). This is a spec addition — `core-semantics.md` §Pattern Matching gains list
           deconstruction — plus seed lowering; until then it declines \"unsupported list pattern\".")
  (needs  list-patterns)
  (input  (module m
            (def (sum xs) (match xs
                            ((list)           0)
                            ((list x .. rest) (+ x (sum rest)))))
            (def (main) (sum (list 10 20 30)))))
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
  (input  (module m
            (type Expr (Lit Int64 | Add (Tuple Expr Expr) | Mul (Tuple Expr Expr)))
            (def (ev e) (match e
                          ((Expr.Lit n)           n)
                          ((Expr.Add (tuple a b)) (+ (ev a) (ev b)))
                          ((Expr.Mul (tuple a b)) (* (ev a) (ev b)))))
            (def (build k) (if (< k 1)
                               (Expr.Lit 2)
                               (Expr.Add (tuple (Expr.Lit k) (build (- k 1))))))
            (def (main) (ev (build 4)))))
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
  (input  (module m
            (type T (Leaf Int64 | Pair (Tuple T T)))
            (def (left x) (match x ((T.Leaf n) (T.Leaf n)) ((T.Pair (tuple a b)) a)))
            (def (main) (match (left (T.Pair (tuple (T.Leaf 7) (T.Leaf 9))))
                          ((T.Leaf n) n)
                          ((T.Pair p) 0)))))
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
  (input  (module m
            (type Expr (Lit Int64 | Bin (Tuple Int64 (Tuple Expr Expr))))
            (def (ev e) (match e
                          ((Expr.Lit n)                    n)
                          ((Expr.Bin (tuple op (tuple a b)))
                             (if (= op 0) (+ (ev a) (ev b)) (- (ev a) (ev b))))))
            (def (main) (ev (Expr.Bin (tuple 0
                                        (tuple (Expr.Lit 20)
                                               (Expr.Bin (tuple 1
                                                 (tuple (Expr.Lit 22) (Expr.Lit 8)))))))))))
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
  (input  (module m
            (type E (Lit Int64 | Neg Int64))
            (def (first-lit xs) (match (List.at xs 0)
                                  ((Some (E.Lit n)) n)
                                  ((None _)         0)))
            (def (main) (first-lit (list (E.Lit 5))))))
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
  (input  (module m
            (type Node (NInt Int64 | NPrim (Tuple String Node Node)))
            (type Core (KConst Int64 | KAdd (Tuple Core Core) | KSub (Tuple Core Core) | KMul (Tuple Core Core)))
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
                                          (Node.NPrim (tuple "*" (Node.NInt 2) (Node.NInt 11))))))))))
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
  (input  (module m
            (type IntList (Cons (Tuple Int64 IntList) | Nil))
            (def (f n) (IntList.Cons (tuple n (IntList.Nil ()))))
            (def (main) (f 5))))
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
  (input  (module m
            (type IntList (Cons (Tuple Int64 IntList) | Nil))
            (def (count n) (if (< n 1)
                               (IntList.Nil ())
                               (IntList.Cons (tuple n (count (- n 1))))))
            (def (main) (count 3))))
  (output (: (IntList.Cons (tuple 3 (IntList.Cons (tuple 2 (IntList.Cons (tuple 1 (IntList.Nil unit))))))) IntList)))

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
  (input  (module m
            (type N (L Int64 | P Int64))
            (type W (Wrap N | Empty))
            (def (f w) (match w
                         ((W.Wrap (N.L v)) v)
                         ((W.Wrap (N.P v)) (+ v 100))
                         ((W.Empty _)      -1)))
            (def (main) (f (W.Wrap (N.P 5))))))
  (output (: 105 Int64)))

(case "a nested constructor payload that misses the inner variant falls through to the outer sibling"
  (doc    "The fall-through companion of the case above: `(f (W.Empty))` matches no `W.Wrap` arm, so it
           reaches the `(W.Empty _)` sibling (-1). And with `(W.Wrap (N.L 5))` the FIRST arm `(W.Wrap
           (N.L v))` matches (5), not the `N.P` arm — the inner discriminant discriminates. Together
           with the case above these pin all three arms of a nested runtime dispatch reachable: inner
           `N.L`, inner `N.P`, and the outer `Empty` fall-through.")
  (needs  sum-type-declaration)
  (input  (module m
            (type N (L Int64 | P Int64))
            (type W (Wrap N | Empty))
            (def (f w) (match w
                         ((W.Wrap (N.L v)) v)
                         ((W.Wrap (N.P v)) (+ v 100))
                         ((W.Empty _)      -1)))
            (def (main) (+ (f (W.Wrap (N.L 5))) (f (W.Empty))))))
  (output (: 4 Int64)))

(case "a runtime Option carrying a user sum is matched by a nested constructor pattern"
  (doc    "The reader/self-host idiom: a fallible access yields an `Option` whose `Some` payload is a
           user AST node. `(List.at (List.push (list) (N.L 7)) 0)` is a runtime `(Some (N.L 7))`; the
           nested pattern `(Some (N.L v))` deconstructs the built-in Option AND the user variant in one
           arm, binding v=7, while `(None _)` is the empty-access arm. Pins the built-in polymorphic
           `Option` carrying a user sum through the runtime nested matcher — the `List.at`/`Bytes.at`
           result a compiler threads when it decodes a node list.")
  (needs  sum-type-declaration)
  (input  (module m
            (type N (L Int64 | P Int64))
            (def (main) (match (List.at (List.push (list) (N.L 7)) 0)
                          ((Some (N.L v)) v)
                          ((Some (N.P v)) (+ v 100))
                          ((None _)       -1)))))
  (output (: 7 Int64)))

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
  (input  (module m
            (def (lookup xs i k)
              (match (List.at xs i)
                ((Some (tuple key val)) (if (= key k) val (lookup xs (+ i 1) k)))
                ((None _)               -1)))
            (def (main) (lookup (list (tuple 1 100) (tuple 2 200)) 0 2))))
  (output (: 200 Int64)))

(case "lists are equal by elements in order"
  (doc    "Witnesses collections-and-text.md #A List Is An Ordered Homogeneous Sequence (equality).
           Needs the primitive collections the seed realizes to build an AST (list/map/record).")
  (needs collections)
  (input  (= (list 1 2 3) (list 1 2 3)))
  (output (: true Bool)))

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
  (input     (= (list (map (a 1)) (map (b 2))) (list (map (a 1)) (map (b 2)))))
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
  (input  (module m
            (type K (KK (Tuple Int64 (List Int64))))
            (def (f c) (match c ((K.KK (tuple fi xs)) (match (List.at xs 0) ((Some x) x) ((None _) -1)))))
            (def (main) (f (K.KK (tuple 7 (list 10 20 30)))))))
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
  (input  (module m
            (type Core (KConst Int64 | KCall (Tuple Int64 (List Core))))
            (def (sum-args xs i n) (if (< i n)
                                       (+ (ev (match (List.at xs i) ((Some c) c) ((None _) (Core.KConst 0))))
                                          (sum-args xs (+ i 1) n))
                                       0))
            (def (ev c) (match c
                          ((Core.KConst v) v)
                          ((Core.KCall (tuple fi xs)) (sum-args xs 0 (List.len xs)))))
            (def (main) (ev (Core.KCall (tuple 9 (list (Core.KConst 10) (Core.KConst 20) (Core.KConst 12))))))))
  (output (: 42 Int64)))

(case "map equality is independent of insertion order"
  (doc    "Witnesses collections-and-text.md #A Map Associates Keys With Values.")
  (needs collections)
  (input  (= (map (a 1) (b 2)) (map (b 2) (a 1))))
  (output (: true Bool)))

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
  (input      (= (map (a 1) (b true)) (map (a 1) (b true))))
  (error      CDZ0201))

(case "a map mixing integer and float values is a type error"
  (doc    "The numeric companion: Int64 and Float64 are distinct types that do not silently unify
           (numeric-model.md #Numeric Types Do Not Silently Promote), so `(map (a 1) (b 2.5))` has two
           value types and is ill-typed — CDZ0201. Pins that map value-homogeneity holds across the
           numeric types too, mirroring the list case.")
  (needs      collections)
  (input      (= (map (a 1) (b 2.5)) (map (a 1) (b 2.5))))
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
  (input      (= (map (a (record (x 1))) (b (record (y 2)))) (map (a (record (x 1))) (b (record (y 2))))))
  (error      CDZ0201))

(case "a map with tuple values of different arities is a type error"
  (doc    "`(tuple 1 2)` and `(tuple 1 2 3)` are different tuple types (different lengths), so a map
           with them as values is not value-homogeneous — CDZ0201. Pins that the map-value homogeneity
           check compares tuple ARITY, mirroring the list case.")
  (needs      collections)
  (input      (= (map (a (tuple 1 2)) (b (tuple 1 2 3))) (map (a (tuple 1 2)) (b (tuple 1 2 3)))))
  (error      CDZ0201))

(case "a map with a duplicate key is a type error"
  (doc    "collections-and-text.md #A Map Associates Keys With Values: \"A map MUST contain each key at
           most once.\" `(map (a 1) (a 2))` repeats the key `a`, so it is ill-typed and the compiler
           rejects it (CDZ0201) rather than build it — a repeated key makes the association ambiguous
           (which value does `a` hold?).")
  (needs      collections)
  (input      (= (map (a 1) (a 2)) (map (a 1) (a 2))))
  (error      CDZ0201))

(case "comparing a map to a record is a type error"
  (doc    "Witnesses type-system.md #Structural Values Are Comparable Only When Their Shapes Match: a
           record and a map are DISTINCT types (a record's field set is fixed by its form; a map's key
           set is a collection — core-semantics.md #A Record Has A Fixed Set Of Named Fields,
           collections-and-text.md #A Map Associates Keys With Values). Comparing values of two
           different types is a type error the compiler rejects (CDZ0201), even though they carry the
           same keys mapped to the same values.")
  (needs      collections)
  (input      (= (map (a 1) (b 2)) (record (a 1) (b 2))))
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
  (input      (= (map (a 1) (b 2)) (map (a 1) (c 2))))
  (output     (: false Bool)))

(case "two maps of different sizes are unequal, not a type error"
  (doc    "`(map (a 1))` and `(map (a 1) (b 2))` differ in their number of entries. A map's entry count
           is runtime data, not part of its type, so the comparison is well-typed and FALSE — they do
           not associate the same keys (collections-and-text.md #A Map Associates Keys With Values). The
           size-difference companion of the case above; the seed rejects it (CDZ0201) rather than
           yielding false — the same miscompile. Contrast records `(= (record (a 1)) (record (a 1) (b
           2)))`, which IS a type error, because a record's field set IS its shape.")
  (needs      collections)
  (input      (= (map (a 1)) (map (a 1) (b 2))))
  (output     (: false Bool)))

(case "an empty map is unequal to a non-empty map, not a type error"
  (doc    "The degenerate companion: an empty map and a one-entry map are the same map type (both
           Map<…>), so comparing them is well-typed and FALSE — they associate different keys. Pins that
           emptiness on one side of a map comparison yields false, not a shape-mismatch rejection
           (contrast the empty-map-vs-empty-record case above, which IS a type error because map and
           record are different types). MUST be false.")
  (needs      collections)
  (input      (= (map) (map (a 1))))
  (output     (: false Bool)))

(case "member access on a map is a type error"
  (doc    "Witnesses core-semantics.md #Member Access Projects A Record Field: member access projects
           a field from the RECORD it is applied to; applied to a value that is not a record it is a
           type error. A map is not a record (its keys are a collection, not a fixed field set), so
           `(. m a)` on a map `m` is rejected (CDZ0201) rather than projecting the entry for `a`.")
  (needs      collections)
  (input      (let ((m (map (a 1) (b 2)))) (. m a)))
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
  (input  (module m
            (type Expr (Lit Int64 | Neg Expr))
            (def (depth e) (match e ((Expr.Lit n) 0) ((Expr.Neg x) (+ 1 (depth x)))))
            (def (main)    (depth (Expr.Neg (Expr.Lit 5))))))
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
  (input  (module m
            (type Node (NLit Int64 | NNil))
            (def (classify n) (if (= n 0) NNil (Node.NLit n)))
            (def (val x) (match x ((Node.NLit v) v) ((Node.NNil _) 1)))
            (def (main) (+ (val (classify 0)) (val (classify 7))))))
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
  (input  (module m
            (type Expr (Lit Int64 | Neg Int64))
            (def (emit e) (match e ((Expr.Lit n) (Bytes.of (list 66))) ((Expr.Neg n) (Bytes.of (list 124)))))
            (def (main)   (emit (Expr.Lit 5)))))
  (output (: b"B" Bytes)))

(case "a recursive lower from a sum to Bytes assembles its output in match arms"
  (doc    "The compiler's emit spine: a recursive `lower : Expr → Bytes` dispatches on each node's
           variant and BUILDS the output bytes in the arm — a `Lit` emits its opcode byte, a `Neg`
           concatenates the lowered child with a suffix byte. `(lower (Neg (Lit 5)))` = `b\"B|\"` (0x42
           'B' for the Lit, 0x7C '|' appended for the Neg). Pins that a match arm may both build a fresh
           compound AND recurse — the shape is inferred and the runtime Bytes assemble correctly, the
           exact shape a self-hosted backend's `lower`/`serialize` takes.")
  (needs  sum-type-declaration)
  (input  (module m
            (type Expr (Lit Int64 | Neg Expr))
            (def (lower e) (match e
                             ((Expr.Lit n) (Bytes.of (list 66)))
                             ((Expr.Neg x) (Bytes.concat (lower x) (Bytes.of (list 124))))))
            (def (main)    (lower (Expr.Neg (Expr.Lit 5))))))
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
            (type Color (Red | Green | Blue))
            (Color.Red unit)))
  (output (: (Color.Red unit) Color)))

(case "a sum type variant can carry data"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable Constructed And Deconstructed (1st
           sentence: 'each optionally carrying data'). Syntax (type Result (Ok Int64 | Err))
           declares Result where Ok carries an Int64 and Err carries Unit (nullary). Both are
           single-arity: Ok takes Int64, Err takes Unit. Constructors: Result.Ok, Result.Err.")
  (needs  sum-type-declaration)
  (input  (do
            (type Result (Ok Int64 | Err))
            (Result.Ok 42)))
  (output (: (Result.Ok 42) Result)))

(case "sum type constructors are in scope after declaration"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable: declaring a sum type binds its
           constructors in the enclosing scope as members of a record named after the type.
           After (type Status (Ready | Waiting)), both Status.Ready and Status.Waiting are
           Constructor values accessible via member access.")
  (needs  sum-type-declaration)
  (input  (do
            (type Status (Ready | Waiting))
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
            (type Maybe (Just Int64 | Nothing))
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
  (input  (module m
            (def (classify n) (if (> n 0) (Some n) (None unit)))
            (def (main) (match (classify 5)
                          ((Some x) x)
                          ((None _) 0)))))
  (output (: 5 Int64)))

(case "match on a function-returned sum takes the other variant at runtime"
  (doc    "The companion of the case above on the other branch: classify(-3) is (None unit), so the
           None arm is selected, yielding 0. Confirms the match follows the runtime variant, not a
           fixed one.")
  (input  (module m
            (def (classify n) (if (> n 0) (Some n) (None unit)))
            (def (main) (match (classify -3)
                          ((Some x) x)
                          ((None _) 0)))))
  (output (: 0 Int64)))

(case "match on a Result returned by a fallible function"
  (doc    "The canonical compiler idiom: a fallible `parse` returns (Ok v) or (Err e) by a condition,
           and the caller matches the runtime Result. parse(5) is (Ok 42), so the Ok arm binds v=42 and
           yields 43. A compiler that reports diagnostics rather than trapping depends on matching a
           Result whose variant is decided at run time.")
  (input  (module m
            (def (parse n) (if (= n 0) (Err 1) (Ok 42)))
            (def (main) (match (parse 5)
                          ((Ok v)  (+ v 1))
                          ((Err e) e)))))
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
  (input  (module m
            (def (classify n)
              (if (< n 0) (Sign.Neg unit)
                  (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))))
            (def (main) (match (classify -5)
                          ((Sign.Neg _)  -1)
                          ((Sign.Zero _) 0)
                          ((Sign.Pos _)  1)))))
  (output (: -1 Int64)))

(case "a runtime three-variant classifier dispatches to the middle arm"
  (doc    "The middle-variant companion: classify(0) is `(Sign.Zero unit)`, selecting the Zero arm for 0.
           Confirms the three-way runtime dispatch reaches the MIDDLE variant, not only the first or
           last — the arm most likely to be mis-ordered in a cascade of comparisons.")
  (input  (module m
            (def (classify n)
              (if (< n 0) (Sign.Neg unit)
                  (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))))
            (def (main) (match (classify 0)
                          ((Sign.Neg _)  -1)
                          ((Sign.Zero _) 0)
                          ((Sign.Pos _)  1)))))
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
  (doc    "Witnesses core-semantics.md: tuple elements are accessed positionally. (tuple.0 t) gets
           the first element, (tuple.1 t) the second, etc. Access is bounds-checked against the
           tuple's statically-known arity — an out-of-bounds index is a type error the compiler
           rejects.")
  (input  (let ((t (tuple 1 "hello" true)))
            (tuple.1 t)))
  (output (: "hello" String)))

(case "a boolean tuple element is projected as the program result"
  (doc    "Witnesses core-semantics.md tuple positional access at a non-Int element type: element 1 of
           `(tuple 42 true)` is the Bool true. Projecting it as the program's result must carry the
           Bool across the run boundary — a tuple element's type is whatever that position holds, not
           uniformly Int64. (Element 0, an Int64, already works; this pins that a Bool element does
           too.)")
  (input  (tuple.1 (tuple 42 true)))
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

(case "a recursive sum type works with pattern matching"
  (doc    "Witnesses type-system.md #Sum Types Are Declarable: sum types can be recursive — a variant
           can carry the type itself. (type IntList (Cons (Tuple Int64 IntList) | Nil)) is a linked list.
           Pattern matching deconstructs recursively. This is critical: the AST is a recursive sum type.")
  (needs  sum-type-declaration)
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList) | Nil))
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
  (input  (module m
            (def (mk n) (List.push (List.push (List.push (list) n) 8) 9))
            (def (main)  (mk 7))))
  (output (: (list 7 8 9) (List Int64))))

(case "the length of a grown list is its element count"
  (doc    "`List.len` reads the element count as a scalar — the fold-to-scalar half of the idiom, like
           `Bytes.len`. `(List.len (List.push (List.push (list) n) 8))` = 2 for any `n`. Pins that the
           length operation reads a list however it was built (grown here, not a literal).")
  (needs  list-growth)
  (input  (module m
            (def (sz n) (List.len (List.push (List.push (list) n) 8)))
            (def (main)  (sz 7))))
  (output (: 2 Int64)))

(case "updating a list index replaces that element, leaving others"
  (doc    "`List.update` is a functional constructor producing a NEW list with one index replaced,
           leaving the operand list unchanged. Updating index 0 of a two-element list to a runtime `n`
           yields `(list 99 2)` for `n=99`. The replace-at-index is defined for the in-bounds index 0.")
  (needs  list-growth)
  (input  (module m
            (def (put n) (List.update (List.push (List.push (list) 1) 2) 0 n))
            (def (main)  (put 99))))
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
  (input  (module m
            (def (build v i n) (if (< i n) (build (List.push v i) (+ i 1) n) v))
            (def (main)         (List.len (build (list) 0 5)))))
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
  (input  (module m
            (def (build n acc) (if (< n 1) acc (build (- n 1) (List.push acc n))))
            (def (main)        (List.len (build 3 (list))))))
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
  (input  (module m
            (def (build i n out) (if (< i n) (build (+ i 1) n (List.push out i)) out))
            (def (sum-at xs i n) (if (< i n)
                                     (+ (match (List.at xs i) ((Some x) x) ((None _) 0)) (sum-at xs (+ i 1) n))
                                     0))
            (def (main) (let ((xs (build 0 3 (list)))) (sum-at xs 0 (List.len xs))))))
  (output (: 3 Int64)))

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
  (input  (module m
            (type Instr (IConst Int64 | IAdd))
            (type Code  (CNil | CCons (Tuple Instr Code)))
            (type Core  (KConst Int64 | KAdd (Tuple Core Core)))
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
            (def (main) (len (lower (Core.KAdd (tuple (Core.KConst 20) (Core.KConst 22))))))))
  (output (: 3 Int64)))

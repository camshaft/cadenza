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

(case "a sum-type value is deconstructed by an exhaustive match"
  (doc    "Patterns are uniform: `(Ctor _)` for nullary constructors. The binder `_` matches the
           unit payload. Consistent with unary constructor patterns like `(Some x)`.")
  (input  (match (Sign.Zero unit)
            ((Sign.Neg _)  -1)
            ((Sign.Zero _) 0)
            ((Sign.Pos _)  1)))
  (output (: 0 Int64)))

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

(case "indexing a list out of bounds traps"
  (doc    "Witnesses collections-and-text.md #List Operations Are Total Or Trap: a well-typed list
           access whose index is out of range halts at run time with a trap — a total-or-trap
           operation, not a static rejection.")
  (needs collections)
  (input  (List.at (list 1 2 3) 5))
  (trap   "list index out of bounds"))

; A NEGATIVE index is out of bounds exactly as an over-large one is — no element sits at position -1.
; It is the classic total-or-trap miscompile: a lowering that casts the signed index to an unsigned
; width (wasm addresses memory with u32/u64 offsets) turns -1 into a huge in-range-looking offset,
; reading an unspecified value instead of trapping. #List Operations Are Total Or Trap requires the
; trap; this pins the negative side of the bounds check the `5` case only exercises on the high side.

(case "indexing a list with a negative index traps"
  (doc    "`(List.at (list 1 2 3) -1)` uses a negative index — no element at position -1 — so it MUST
           trap (collections-and-text.md #List Operations Are Total Or Trap), NOT wrap the negative
           index to a large unsigned offset and read an unspecified element. The negative-index
           companion of the out-of-bounds `5` case above; both must trap.")
  (needs collections)
  (input  (List.at (list 1 2 3) -1))
  (trap   "list index out of bounds"))

(case "indexing an empty list traps"
  (doc    "`(List.at (list) 0)` indexes position 0 of a list with no elements — out of bounds, since
           an empty list has no element at any index — so it MUST trap. Pins the degenerate boundary:
           index 0 is valid only when the list is non-empty.")
  (needs collections)
  (input  (List.at (list) 0))
  (trap   "list index out of bounds"))

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

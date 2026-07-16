; Rows and open sums — witnesses type-system.md #Records Are Rows, Open By Default Under Inference,
; #A Sum Type May Be Open, With A Mandatory Open-Tail Arm, and #An Open Sum's Payload May Be
; Schema-Typed. These exercise rows / open-sums, which a later generation realizes; the seed
; realizes closed records and closed sums (05-compound-types) but not row polymorphism or open sums.
; The primary clause is the recorded oracle: a well-typed program's value, or — for an ill-typed one —
; its (error <CODE>) rejection (a rule a generation does not yet cover is declined, not run).

(case "a function open over a record's extra fields accepts any record with the used field"
  (doc    "Witnesses type-system.md #Records Are Rows, Open By Default Under Inference: `get-x` uses only
           field `x`, so it is typed open over the other fields and accepts a record that also has `y`.
           Row polymorphism, not a fixed shape, is what inference assigns.")
  (input  (do
            (def (get-x r) (. r x))
            (def (main) (get-x (record (x 1) (y 2)))) (export main)))
  (output (: 1 Int64)))

(case "subset record comparison is explicit projection, not an overloaded equality"
  (doc    "Witnesses type-system.md #Records Are Rows (subset comparison is explicit projection-then-=):
           comparing a two-field record against a one-field record by first projecting the shared field
           yields true; `=` is never silently widened to ignore the extra field.")
  (input  (do
            (def (main)
              (= (. (record (x 1) (y 2)) x)
                 (. (record (x 1)) x))) (export main)))
  (output (: true Bool)))

; --- Record reshaping: explicit row operations yield a new closed record -------------------
; type-system.md #A Record Row Is Reshaped Only Through An Explicit Operation Yielding A New Value and
; its four companions pin the operation surface the rows learning promised
; (spec/learnings/2026-07-04-records-are-rows-open-by-default.md: an explicit `project`/narrowing is
; "the only thing that changes the shape"). options/record-tuple-operations/ pins the concrete forms:
; three primitives — `Record.project` (restrict to named fields), `Record.without` (drop named fields),
; `Record.merge` (disjoint union) — from which `extend`/`with`/`pop` reduce by a meaning-preserving
; rewrite. Each yields a NEW closed record (the value heap is immutable); each result shape is fixed
; statically from the operands' shapes. A field-name list `(a b …)` is written literally, as a `record`
; literal writes names — not a runtime value. These are rows cases (like the open-record
; cases above): the seed does not realize row inference, and `Record.*` is an unbound name to it, so
; it DECLINES them rather than rejecting the unbound prelude name (which would be a gate FAIL).

(case "projecting a record restricts it to the named fields"
  (doc    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields: `Record.project`
           narrows a record to exactly the stated field names, each bound to the value the operand holds.
           `(Record.project (record (a 1) (b 2) (c 3)) (a c))` keeps `a` and `c`, dropping `b`, yielding
           the closed record `(record (a 1) (c 3))`. The result renders in canonical key-sorted order.")
  (input  (Record.project (record (a 1) (b 2) (c 3)) (a c)))
  (output (: (record (a 1) (c 3)) (Record (a Int64) (c Int64)))))

(case "projecting a record onto an absent field is rejected"
  (doc    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields (2nd sentence):
           a projection naming a field the operand does not contain is a compile-time rejection (CDZ0212),
           so a projection cannot silently produce a field the operand never held. `z` is not a field of
           `(record (a 1) (b 2))`.")
  (input  (Record.project (record (a 1) (b 2)) (a z)))
  (error  CDZ0212))

(case "projecting a record with a duplicate label is rejected"
  (doc    "A record's fields are a fixed SET of statically-known names (type-system.md #A Record Is
           Restricted To A Named Set Of Its Fields), so a projection label list that names a field TWICE
           — `(Record.project (record (a 1) (b 2)) (a a))` — is the same malformedness a record LITERAL
           with a duplicate field `(record (a 1) (a 2))` is rejected for (CDZ0201), not silently
           deduplicated to a single field. A duplicate label is almost always an author error (a typo, a
           copy-paste); the projection label-list check matches the record-literal duplicate-field check.")
  (input  (do
            (def (main) (. (Record.project (record (a 1) (b 2)) (a a)) a))
            (export main)))
  (error  CDZ0201))

(case "dropping fields from a record leaves the remaining fields"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields:
           `Record.without` derives the record of the operand's fields EXCEPT those named. `(Record.without
           (record (a 1) (b 2) (c 3)) (b))` drops `b`, yielding `(record (a 1) (c 3))` — the complement of
           projecting the fields kept.")
  (input  (Record.without (record (a 1) (b 2) (c 3)) (b)))
  (output (: (record (a 1) (c 3)) (Record (a Int64) (c Int64)))))

(case "dropping an absent field from a record is rejected"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence): dropping a field the operand does not contain is a compile-time rejection (CDZ0212),
           not a silent no-op. `z` is not a field of `(record (a 1))`.")
  (input  (Record.without (record (a 1)) (z)))
  (error  CDZ0212))

(case "merging two records with disjoint fields unions their fields"
  (doc    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint:
           `Record.merge` combines two records into one whose field set is the union, each field bound to
           its source's value. `(Record.merge (record (a 1)) (record (b 2)))` yields `(record (a 1) (b 2))`
           — the row analogue of forming a record from two groups of fields.")
  (input  (Record.merge (record (a 1)) (record (b 2))))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

; The merge above builds both operand records from CONSTANT literals, so the union folds to a constant
; record. A record carrying a RUNTIME field value cannot fold — the merge runs on the value heap. These
; read the merged record's fields back down to a scalar (so a parameterized export returns), pinning that
; a runtime `Record.merge` carries EACH operand's field into the result at its own value.

(case "merging records with a runtime field value carries both operands' fields"
  (doc    "`(Record.merge (record (a n)) (record (b 2)))` with `n` a boundary parameter builds the left
           record from a runtime value, so the merge runs on the value heap. Reading field `a` (the runtime
           `n`, from the left operand) plus field `b` (2, from the right) yields `n + 2`: 7+2 = 9, 100+2 =
           102. Pins that a runtime merge unions BOTH operands' fields, each bound to its source's value,
           read back by member access.")
  (input  (do (def (main (: n Int64))
                (+ (. (Record.merge (record (a n)) (record (b 2))) a) (. (Record.merge (record (a n)) (record (b 2))) b))) (export main)))
  (call   main (: 7 Int64)) (output (: 9 Int64))
  (call   main (: 100 Int64)) (output (: 102 Int64)))

(case "merging records with runtime values on both sides binds each field to its own value"
  (doc    "Both operands carry a runtime field: `(Record.merge (record (a x)) (record (b y)))`. Reading `b`
           minus `a` yields `y - x` (10-3 = 7), so each field holds its OWN operand's runtime value — the
           merge does not confuse or alias the two runtime slots. Pins per-field value fidelity on the
           runtime path when neither operand is constant.")
  (input  (do (def (main (: x Int64) (: y Int64))
                (- (. (Record.merge (record (a x)) (record (b y))) b) (. (Record.merge (record (a x)) (record (b y))) a))) (export main)))
  (call   main (: 3 Int64) (: 10 Int64)) (output (: 7 Int64))
  (call   main (: 50 Int64) (: 8 Int64)) (output (: -42 Int64)))

(case "merging records that share a field name is rejected"
  (doc    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint (2nd
           sentence): merging two records that share a field name is a compile-time rejection (CDZ0211), so
           the combined record never has to choose which operand's value the shared field takes — the
           row-operation companion of the duplicate-field literal `(record (a 1) (a 2))` (CDZ0201). `a` is
           shared, so `Record.merge` REJECTS rather than picking a winner (no silent clobber).")
  (input  (Record.merge (record (a 1)) (record (a 2))))
  (error  CDZ0211))

(case "merging with the empty record on the left is the identity"
  (doc    "The empty record `(record)` has no fields, so it is trivially disjoint from any record and is the
           IDENTITY of `Record.merge`: `(Record.merge (record) (record (a 1) (b 2)))` equals `(record (a 1)
           (b 2))` — merging in nothing adds nothing. Pins the empty-operand identity the disjoint-merge
           cases above (which union two non-empty records) do not exercise — the record companion of the
           empty-list / empty-set / empty-tuple identity laws.")
  (input  (= (Record.merge (record) (record (a 1) (b 2))) (record (a 1) (b 2))))
  (output (: true Bool)))

(case "merging with the empty record on the right is the identity"
  (doc    "The mirror: `(Record.merge (record (a 1) (b 2)) (record))` equals `(record (a 1) (b 2))` — the
           empty record is the identity on the right as well as the left. Pins that a merge with an empty
           operand on either side is a no-op on value (merge is symmetric on the empty record).")
  (input  (= (Record.merge (record (a 1) (b 2)) (record)) (record (a 1) (b 2))))
  (output (: true Bool)))

(case "merging two empty records is the empty record"
  (doc    "The degenerate boundary: `(Record.merge (record) (record))` combines two field-less records into
           the empty record `(record)` — a genuine value equal to itself (its type is `(Record)`). Pins that
           merge handles the empty+empty case, the record companion of the empty+empty list/set/tuple
           cases, and that the empty record is a first-class value, not only a type-error foil.")
  (input  (= (Record.merge (record) (record)) (record)))
  (output (: true Bool)))

(case "extending a record adds a new field"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation:
           `Record.extend` adds a field ABSENT from the operand, defined as `(Record.merge r (record (z v)))`.
           `(Record.extend (record (a 1)) #"b" 2)` yields `(record (a 1) (b 2))`. The added field may hold
           any type. The field name is a `#field` label operand (a static label, not a runtime value).")
  (input  (Record.extend (record (a 1)) #"b" 2))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "extending a record with an already-present field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (1st sentence): adding a field the operand already contains is a compile-time rejection (CDZ0211),
           so `extend` never silently overwrites. `a` is already present, so this is a clobber `extend`
           forbids — the author means `Record.with` to replace. Rides the strict `Record.merge` disjointness
           its rewrite uses.")
  (input  (Record.extend (record (a 1)) #"a" 2))
  (error  CDZ0211))

(case "updating a record field replaces its value"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence): `Record.with` replaces a field PRESENT in the operand, defined as `(Record.merge
           (Record.without r (z)) (record (z v)))`. `(Record.with (record (a 1) (b 2)) #"b" 9)` yields
           `(record (a 1) (b 9))` — an explicit update distinct from `extend`.")
  (input  (Record.with (record (a 1) (b 2)) #"b" 9))
  (output (: (record (a 1) (b 9)) (Record (a Int64) (b Int64)))))

(case "updating a record field changes its type to the new value's"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence: 'a new value of a possibly different type'): the result is a new closed record
           whose field `b` has whatever type the new value holds. `(Record.with (record (a 1) (b 2)) #"b"
           true)` retypes `b` from Int64 to Bool, yielding `(record (a 1) (b true))` of type `(Record (a
           Int64) (b Bool))`. Pins that `with` is not constrained to the field's prior type.")
  (input  (Record.with (record (a 1) (b 2)) #"b" true))
  (output (: (record (a 1) (b true)) (Record (a Int64) (b Bool)))))

(case "updating an absent record field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (3rd sentence): updating a field absent from the operand is a compile-time rejection (CDZ0212),
           not an addition, so `with` and `extend` stay distinct. `z` is not a field of `(record (a 1))`,
           so `Record.with` REJECTS — the author means `Record.extend` to add. Rides the `Record.without`
           presence check its rewrite uses.")
  (input  (Record.with (record (a 1)) #"z" 5))
  (error  CDZ0212))

(case "popping a field yields its value and the remaining record"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields and #A Field
           Is Added To Or Replaced In A Record By A Derived Operation: `Record.pop` takes a field OFF a
           record, defined as `(tuple (. r z) (Record.without r (z)))` — the field's value paired with the
           record of the remaining fields. `(Record.pop (record (a 1) (b 2)) a)` yields `(tuple 1 (record
           (b 2)))`. No Option: field presence is static, so a missing field is CDZ0212, not a runtime None
           (contrast `List.at` on a runtime index).")
  (input  (Record.pop (record (a 1) (b 2)) a))
  (output (: (tuple 1 (record (b 2))) (Tuple Int64 (Record (b Int64))))))

(case "popping an absent field is rejected"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence), via `Record.pop`'s `Record.without` rewrite: popping a field the record does not
           contain is a compile-time rejection (CDZ0212), not a runtime None — a record field name is a
           static label, not a runtime index. `z` is absent from `(record (a 1))`.")
  (input  (Record.pop (record (a 1)) z))
  (error  CDZ0212))

(case "record reshaping is subset comparison as explicit projection"
  (doc    "Witnesses type-system.md #Records Are Rows (4th sentence: subset comparison is explicit
           projection-then-`=`, never an overloaded `=`) with `Record.project` as the narrowing operation.
           `(= (Record.project (record (x 1) (y 2)) (x)) (record (x 1)))` projects the shared field to a
           closed one-field record and compares it by ordinary structural equality — true. The
           general-projection form of the plain-`.` subset-comparison case above; `=` is never widened to
           ignore `y`, `Record.project` narrows the shape first.")
  (input  (= (Record.project (record (x 1) (y 2)) (x)) (record (x 1))))
  (output (: true Bool)))

; The cases above pin each record operation in isolation (extend, without, with, merge, project). These pin
; their ALGEBRAIC compositions — the round-trips and inverses where a field-set bookkeeping slip would
; surface: extend-then-without is the identity, `with` preserves the OTHER fields, and merge-then-project
; recovers a merged side. Each composes ≥2 operations, so a result that mis-tracked the field set (dropped,
; duplicated, or reordered a label) fails the structural `=`.

(case "extending a record then dropping the added field returns the original"
  (doc    "`(Record.without (Record.extend (record (a 1)) #\"b\" 2) (b))` = `(record (a 1))` — extend adds
           `b`, without drops it, and the result equals the original by structural `=`. Pins that
           extend/without are inverse on the added field: the field-set bookkeeping adds then removes exactly
           `b`, leaving `a` untouched.")
  (input  (= (Record.without (Record.extend (record (a 1)) #"b" 2) (b)) (record (a 1))))
  (output (: true Bool)))

(case "updating a record field preserves the other fields' values"
  (doc    "`(Record.with (record (a 1) (b 2) (c 3)) #\"b\" 9)` = `(record (a 1) (b 9) (c 3))` — `with`
           replaces only `b`, leaving `a` and `c` at their original values. Pins that an update is local to
           the named field: the surrounding fields (both before and after the updated one) keep their values
           and positions, not just the updated field being correct.")
  (input  (= (Record.with (record (a 1) (b 2) (c 3)) #"b" 9) (record (a 1) (b 9) (c 3))))
  (output (: true Bool)))

(case "merging two disjoint records then projecting one side recovers it"
  (doc    "`(Record.project (Record.merge (record (a 1) (b 2)) (record (c 3))) (a b))` = `(record (a 1)
           (b 2))` — merge unions the disjoint fields, then project narrows back to the left side's labels,
           recovering it exactly. Pins the merge/project round-trip: the merged record carries all three
           fields with their values, and projecting `(a b)` selects the two by name unchanged.")
  (input  (= (Record.project (Record.merge (record (a 1) (b 2)) (record (c 3))) (a b)) (record (a 1) (b 2))))
  (output (: true Bool)))

; --- Tuple reshaping: explicit positional operations yield a new tuple ----------------------
; type-system.md #A Tuple Is Reshaped Positionally By An Explicit Operation Yielding A New Value and its
; companions: `Tuple.concat` concatenates, `Tuple.split-at` splits at a static position, `Tuple.remove` takes
; element 0 off. A tuple's arity is part of its type, so every result arity is fixed statically and there
; is no disjointness constraint (positions are anonymous). `k` is a compile-time position written as a
; literal, exactly as `(. x N)` writes its index; a split outside `0..=len` is a type error, the `(. x N)`
; static-bounds rule (05-compound-types "tuple elements are accessed by index"). These ride
; the same later-generation rows layer and `Tuple.*` is an unbound name to the seed, so it declines them.

(case "concatenating two tuples appends their elements"
  (doc    "Witnesses type-system.md #Two Tuples Are Concatenated Into One Of Their Combined Length:
           `(Tuple.concat (tuple 1 2) (tuple 3 4))` yields `(tuple 1 2 3 4)` of arity 4 — the first tuple's
           elements in order followed by the second's, each keeping its source position's type.")
  (input  (Tuple.concat (tuple 1 2) (tuple 3 4)))
  (output (: (tuple 1 2 3 4) (Tuple Int64 Int64 Int64 Int64))))

(case "concatenating tuples preserves each element's type"
  (doc    "The heterogeneous companion: `(Tuple.concat (tuple 1 true) (tuple \"x\"))` yields `(tuple 1 true
           \"x\")` of type `(Tuple Int64 Bool String)`. Pins that concatenation keeps the type of each
           source position rather than unifying to one element type — a tuple is a heterogeneous product,
           unlike a homogeneous list.")
  (input  (Tuple.concat (tuple 1 true) (tuple "x")))
  (output (: (tuple 1 true "x") (Tuple Int64 Bool String))))

; The concatenation cases above build both operand tuples from CONSTANT literals, so the result folds to a
; constant tuple at compile time. A tuple carrying a RUNTIME element — a boundary parameter — cannot fold:
; the concatenation runs on the value heap, and reading an element back exercises the emitted machinery. A
; case reads the result down to a SCALAR (a projection then arithmetic) so it returns from a parameterized
; export. These pin `Tuple.concat` on a runtime operand — the value companion of the constant cases.

(case "concatenating tuples with a runtime element reads elements from both operands"
  (doc    "`(Tuple.concat (tuple n 2) (tuple 3 4))` with `n` a boundary parameter cannot fold — the first
           element is decided at run time, so the concatenation runs on the value heap. Reading element 0
           (the runtime `n`, from the first operand) and element 3 (4, from the second) and summing them
           yields `n + 4`: 7+4 = 11. Pins that a runtime `Tuple.concat` places BOTH operands' elements into
           the result at their combined positions, read back correctly by projection.")
  (input  (do (def (main (: n Int64))
                (+ (. (Tuple.concat (tuple n 2) (tuple 3 4)) 0) (. (Tuple.concat (tuple n 2) (tuple 3 4)) 3))) (export main)))
  (call   main (: 7 Int64)) (output (: 11 Int64))
  (call   main (: 100 Int64)) (output (: 104 Int64)))

(case "runtime tuple concatenation preserves element order across the seam"
  (doc    "`(. (Tuple.concat (tuple 1 2) (tuple n 4)) 2)` reads position 2 of the concatenation — the FIRST
           element of the second operand (`n`) — which lands just past the first operand's two elements. It
           is `n` for every `n` (99 → 99). Pins that the second operand's elements are appended AFTER the
           first's on the runtime path, so position 2 is the second tuple's element 0, not a first-operand
           element or a shifted slot.")
  (input  (do (def (main (: n Int64)) (. (Tuple.concat (tuple 1 2) (tuple n 4)) 2)) (export main)))
  (call   main (: 99 Int64)) (output (: 99 Int64))
  (call   main (: -7 Int64)) (output (: -7 Int64)))

(case "concatenating an empty tuple on the left is the identity"
  (doc    "The empty tuple `(tuple)` — which IS the unit value (core-semantics.md #The Empty Tuple Is The
           Unit Value) — is the identity of `Tuple.concat`: `(Tuple.concat (tuple) (tuple 1 2))` prepends no
           elements, so the result is `(tuple 1 2)`. Pins the empty-operand identity the existing cat cases
           (which join two non-empty tuples) do not exercise — the tuple companion of the empty-string /
           empty-bytes concatenation-identity cases.")
  (input  (Tuple.concat (tuple) (tuple 1 2)))
  (output (: (tuple 1 2) (Tuple Int64 Int64))))

(case "concatenating an empty tuple on the right is the identity"
  (doc    "The mirror: `(Tuple.concat (tuple 1 2) (tuple))` appends no elements, so the result is `(tuple 1
           2)`. Pins that the empty tuple is the identity on the right as well as the left, so a cat with an
           empty operand on either side is a no-op on value.")
  (input  (Tuple.concat (tuple 1 2) (tuple)))
  (output (: (tuple 1 2) (Tuple Int64 Int64))))

(case "concatenating two empty tuples is the empty tuple"
  (doc    "The degenerate boundary: `(Tuple.concat (tuple) (tuple))` joins nothing to nothing, yielding the
           empty tuple `(tuple)` — the unit value. Pins that cat handles the zero+zero case, not
           underflowing or producing a novel form, the tuple companion of the empty+empty string/bytes/set
           cases.")
  (input  (Tuple.concat (tuple) (tuple)))
  (output (: (tuple) (Tuple))))

(case "splitting a tuple at a position yields a prefix and a suffix"
  (doc    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix:
           `(Tuple.split-at (tuple 1 2 3) 1)` splits at position 1 into a pair — the first element as a
           1-tuple prefix and the rest as a 2-tuple suffix — yielding `(tuple (tuple 1) (tuple 2 3))`. The
           position `k` is a compile-time literal.")
  (input  (Tuple.split-at (tuple 1 2 3) 1))
  (output (: (tuple (tuple 1) (tuple 2 3)) (Tuple (Tuple Int64) (Tuple Int64 Int64)))))

(case "splitting a tuple at zero yields an empty prefix"
  (doc    "The degenerate boundary of #A Tuple Is Split At A Position Into A Prefix And A Suffix: a split at
           position 0 puts no elements before it, so the prefix is the empty tuple — which IS the unit value
           (core-semantics.md #The Empty Tuple Is The Unit Value: `unit` and `()` are the same value) — and
           the suffix is the whole tuple. `(Tuple.split-at (tuple 1 2) 0)` yields `(tuple unit (tuple 1 2))`,
           the prefix typed `Unit`. Pins that 0 is in range and the empty prefix is the unit value, not a
           novel zero-arity tuple form.")
  (input  (Tuple.split-at (tuple 1 2) 0))
  (output (: (tuple unit (tuple 1 2)) (Tuple Unit (Tuple Int64 Int64)))))

(case "splitting a tuple at its full arity yields an empty suffix"
  (doc    "The symmetric boundary of the split-at-zero case: a split at position `k` = the tuple's ARITY
           puts every element before it, so the prefix is the whole tuple and the SUFFIX is the empty tuple
           — the unit value (core-semantics.md #The Empty Tuple Is The Unit Value). `(Tuple.split-at (tuple
           1 2) 2)` yields `(tuple (tuple 1 2) unit)`, the suffix typed `Unit`. Pins that `k` = arity is in
           range (the split point may sit just past the last element) and the empty suffix is unit — the
           k=arity end of the k=0/k=arity boundary the split-at-zero case pins at the other end.")
  (input  (Tuple.split-at (tuple 1 2) 2))
  (output (: (tuple (tuple 1 2) unit) (Tuple (Tuple Int64 Int64) Unit))))

(case "splitting a tuple beyond its arity is rejected"
  (doc    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix (2nd
           sentence): a split position outside the operand's static arity range is a type error (CDZ0201),
           consistent with an out-of-arity positional access `(. x N)` being rejected. `(tuple 1 2)` has
           arity 2, so a split at 5 names a position it does not have — rejected rather than producing a
           short suffix.")
  (input  (Tuple.split-at (tuple 1 2) 5))
  (error  CDZ0201))

(case "accessing through an empty-side split-at is usable, like the equivalent literal"
  (doc    "The empty-prefix split `(Tuple.split-at (tuple 10 20) 0)` yields `(tuple unit (tuple 10 20))` —
           the SAME type and value as the hand-written literal, which is directly accessible. Reading
           through the result — the suffix `.1` then its element 0 — gives 10, matching what
           `(. (. (tuple unit (tuple 10 20)) 1) 0)` gives. The empty side is a `Unit` element; the
           projection through it FOLDS through the constant tuple the operation produced (no runtime
           value-heap build), so a split-at at the k=0 / k=arity boundary is usable, not just renderable.
           Pins that the empty-side result reaches the same representation the byte-identical literal does.")
  (input  (do
            (def (main) (. (. (Tuple.split-at (tuple 10 20) 0) 1) 0))
            (export main)))
  (output (: 10 Int64)))

(case "splitting a tuple with a runtime element addresses the prefix and suffix"
  (doc    "`(Tuple.split-at (tuple n 20 30) 1)` with `n` a boundary parameter splits at position 1 (a
           compile-time literal) into a 1-tuple prefix `(tuple n)` and a 2-tuple suffix `(tuple 20 30)`,
           built on the value heap because `n` is runtime. Reading the prefix's element 0 (`n`) plus the
           suffix's element 1 (30) gives `n + 30`: 5+30 = 35. Pins that a runtime `Tuple.split-at` places
           the operand's runtime element into the correct side and position, read back by nested
           projection — the split boundary is the static `k` regardless of the element values.")
  (input  (do (def (main (: n Int64))
                (+ (. (. (Tuple.split-at (tuple n 20 30) 1) 0) 0) (. (. (Tuple.split-at (tuple n 20 30) 1) 1) 1))) (export main)))
  (call   main (: 5 Int64)) (output (: 35 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64)))

(case "popping a tuple yields element zero and the remaining tuple"
  (doc    "Witnesses type-system.md #A Tuple Is Reshaped Positionally: `Tuple.remove` takes element 0 off,
           `(tuple (. t 0) <rest>)` — the positional analogue of `Record.pop`. `(Tuple.remove (tuple 1 2
           3))` yields `(tuple 1 (tuple 2 3))`. It is `(Tuple.split-at t 1)` with the singleton prefix
           unwrapped to its element.")
  (input  (Tuple.remove (tuple 1 2 3)))
  (output (: (tuple 1 (tuple 2 3)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "popping a tuple with a runtime element separates the head from the rest"
  (doc    "The runtime companion: `(Tuple.remove (tuple n 20 30))` with `n` a boundary parameter splits the
           head element 0 (`n`) from the rest `(tuple 20 30)`, built on the value heap because `n` is
           runtime. Reading the popped head (`.0` = `n`) and the rest's last element (`.1 .1` = 30) and
           summing gives `n + 30`: 9+30 = 39. Pins that a runtime `Tuple.remove` places the operand's element
           0 as the head and the remaining elements as the rest tuple, both read back by projection.")
  (input  (do (def (main (: n Int64))
                (+ (. (Tuple.remove (tuple n 20 30)) 0) (. (. (Tuple.remove (tuple n 20 30)) 1) 1))) (export main)))
  (call   main (: 9 Int64)) (output (: 39 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64)))

(case "a match on an open sum with an open-tail arm is exhaustive"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: an open sum
           is DECLARED with a trailing `.. r` row-variable marker (`(type Vocab (Known Unit) (Unknown
           Unit) .. r)`), which stands for variants the module does not name. A match covering a named
           variant plus an open-tail `_` arm is exhaustive and handles every unnamed variant as data, so
           it yields \"known\" for the `Known` value.")
  (input  (do
            (type Vocab (Known Unit) (Unknown Unit) .. r)
            (def (name-of (: e Vocab))
              (match e
                ((Known _) "known")
                (_         "other")))
            (def (main) (name-of (Known unit))) (export main)))
  (output (: "known" String)))

(case "an open sum's open-tail arm dispatches a variant the specific arms do not name"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm (the open-tail
           arm handles the unnamed variants as data): the open-tail `_` arm is not just an
           exhaustiveness formality — it actually DISPATCHES. A `Vocab` value that is NOT the specific
           `Known` arm falls through to `_`, so `(name-of (Unknown unit))` yields \"other\". This pins the
           dispatch/fold path through the open tail, the runnable companion to the exhaustiveness verdict.")
  (input  (do
            (type Vocab (Known Unit) (Unknown Unit) .. r)
            (def (name-of (: e Vocab))
              (match e
                ((Known _) "known")
                (_         "other")))
            (def (main) (name-of (Unknown unit))) (export main)))
  (output (: "other" String)))

(case "a match on an open sum omitting the open-tail arm is rejected"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open (a match that omits the open-tail arm is a
           compile-time rejection): because an open sum's variant set is not closed, a match covering
           every NAMED variant but omitting the open-tail `_` arm still cannot be exhaustive — the row
           variable stands for variants it cannot enumerate — so it is rejected (CDZ0210) rather than
           run. A closed sum with the same two arms WOULD be exhaustive; the open declaration is what
           mandates the `_`.")
  (input  (do
            (type Vocab (Known Unit) (Unknown Unit) .. r)
            (def (name-of (: e Vocab))
              (match e
                ((Known _)   "known")
                ((Unknown _) "unknown")))
            (def (main) (name-of (Unknown unit))) (export main)))
  (error  CDZ0210))

(case "a SINGLE-variant open sum still requires an open-tail arm"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: a
           single-named-variant CLOSED sum erases to a newtype whose sole constructor pattern is
           irrefutable (no `_` needed). But the SAME sum declared OPEN (`(type Box (Wrap Int64) .. r)`)
           is NOT a newtype — the row variable means a value's variant is not statically `Wrap`, so a
           match covering only `(Wrap n)` without a `_` arm is non-exhaustive (CDZ0210). Pins that
           open-ness suppresses the single-variant newtype erasure for exhaustiveness.")
  (input  (do
            (type Box (Wrap Int64) .. r)
            (def (unwrap (: b Box)) (match b ((Wrap n) n)))
            (def (main) (unwrap (Wrap 42))) (export main)))
  (error  CDZ0210))

(case "a single-variant open sum with an open-tail arm dispatches its named variant"
  (doc    "The runnable companion: the SAME single-variant open sum `(type Box (Wrap Int64) .. r)`, now
           WITH the open-tail `_` arm, is exhaustive and dispatches the named `Wrap` variant to its
           payload — `(unwrap (Wrap 42))` yields 42. Pins that the newtype-erasure suppression (which
           keeps the value a boxed sum) does not break the named variant's own payload read.")
  (input  (do
            (type Box (Wrap Int64) .. r)
            (def (unwrap (: b Box)) (match b ((Wrap n) n) (_ 0)))
            (def (main) (unwrap (Wrap 42))) (export main)))
  (output (: 42 Int64)))

(case "an open sum's open-tail arm dispatches a NAMED-but-uncovered variant, not only unnamed ones"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm (the open-tail
           arm handles the variants not covered as data): the `_` arm covers not just the UNNAMED row-tail
           variants but also any NAMED variant the specific arms omit. `(type Vocab (A Int64) (B Int64)
           .. r)` matched with only an `A` arm plus `_` dispatches a `B` value through `_` → 99. Pins that
           the wildcard is a genuine catch-all over the whole uncovered set, named and unnamed alike.")
  (input  (do
            (type Vocab (A Int64) (B Int64) .. r)
            (def (rd (: v Vocab)) (match v ((A n) n) (_ 99)))
            (def (main) (rd (B 3))) (export main)))
  (output (: 99 Int64)))

(case "a nested pattern under an open sum's named variant still requires the outer open-tail arm"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           #Patterns Compose: an open sum whose named variant carries a compound payload
           (`(type Vocab (Wrap (Option Int64)) .. r)`) is matched by NESTING into the payload
           (`(Wrap (Some n))` / `(Wrap (None))`), and the OUTER open level still needs its `_` arm — the
           row tail is uncovered by any `Wrap` pattern. With the `_` present the match is exhaustive and
           the nested `(Some 5)` payload reads through to 5.")
  (input  (do
            (type Vocab (Wrap (Option Int64)) .. r)
            (def (rd (: v Vocab)) (match v ((Wrap (Some n)) n) ((Wrap (None)) 0) (_ -1)))
            (def (main) (rd (Wrap (Some 5)))) (export main)))
  (output (: 5 Int64)))

(case "an open sum nested as another sum's payload matches with an inner wildcard"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           #Patterns Compose: an OPEN sum can be the payload of another (closed) sum — here an
           `(Option Inner)` where `Inner` is open. A match nesting into the `Some` payload
           (`(Some (A n))`) plus a `(Some _)` arm covering the open Inner's other/unnamed variants plus
           `(None)` is exhaustive: the outer `Option` is CLOSED (Some/None both covered), and the inner
           open `Inner` is covered by the `(Some _)` wildcard. `(rd (Some (B 5)))` falls through `(Some
           (A n))` to `(Some _)` → 0. Pins that an open sum composes as a generic sum's payload and its
           open tail is satisfied by an inner `_` at the nesting level.")
  (input  (do
            (type Inner (A Int64) (B Int64) .. r)
            (def (rd (: o (Option Inner)))
              (match o ((Some (A n)) n) ((Some _) 0) ((None) -1)))
            (def (main) (rd (Some (B 5)))) (export main)))
  (output (: 0 Int64)))

(case "an open sum with a guarded named arm plus an open-tail arm is exhaustive"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           the guarded-arm rule (a guarded arm covers no variant, so it never satisfies exhaustiveness on
           its own): an open sum matched by a GUARDED named arm `(guard (A n) (> n 0))` plus the open-tail
           `_` is exhaustive — the `_` covers both the guard's false-fallthrough and the open/unnamed
           variants. `(rd (A 7))` satisfies the guard (7 > 0) → 7. Pins that the open-tail arm composes
           with a guard exactly as it does for a closed sum's guarded arms.")
  (input  (do
            (type V (A Int64) (B Int64) .. r)
            (def (rd (: v V)) (match v ((guard (A n) (> n 0)) n) (_ 0)))
            (def (main) (rd (A 7))) (export main)))
  (output (: 7 Int64)))

(case "an open sum's payload decodes against a schema to a typed result"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed: a variant's payload is
           decoded against a schema resolved at run time, yielding a typed Ok result on a match. A
           successful decode of an Int64 payload yields (Ok 7).")
  (input  (do
            (type Reading (Measured Int64) (Labeled String) .. r)
            (def (main)
              (decode Int64-schema (payload-of (Measured 7)))) (export main)))
  (output (: (Ok 7) (Result Int64 DecodeError))))

(case "an open sum payload that does not match its schema yields a typed failure, not a trap"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed (a mismatch yields a typed
           failure result rather than a trap): decoding a String payload against an Int64 schema yields
           an Err, so a fold over an open vocabulary handles a malformed payload as data rather than
           halting. The error is `DecodeError`'s `TypeMismatch` kind — a decode failure names its KIND
           (DecodeError is a multi-variant sum `(TypeMismatch | Eof)`, not a payload-carrying newtype),
           so the `Err` renders `(TypeMismatch unit)` (a nullary variant crosses the boundary as `(Name
           unit)`, like `(Pos unit)` for `Sign.Pos`). Never a trap.")
  (input  (do
            (type Reading (Measured Int64) (Labeled String) .. r)
            (def (main)
              (decode Int64-schema (payload-of (Labeled "x")))) (export main)))
  (output (: (Err (TypeMismatch unit)) (Result Int64 DecodeError))))

(case "a schema decode's Ok result is matched to recover the decoded payload"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed composed with #A Match Is
           Exhaustive Against The Sum Type's Variant Set: `decode`'s result is a REAL `(Result Int64
           DecodeError)`, matchable like any Result — not merely a rendered value. Matching the successful
           decode `(decode Int64-schema (payload-of (Measured 9)))` on `(Ok n)` recovers the decoded
           payload `n` = 9. Pins that the decode result flows into ordinary sum matching.")
  (input  (do
            (type Reading (Measured Int64) (Labeled String) .. r)
            (def (main)
              (match (decode Int64-schema (payload-of (Measured 9)))
                ((Ok n) n)
                ((Err _) -1))) (export main)))
  (output (: 9 Int64)))

(case "a schema decode's Err result is handled as data on its own branch"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed (a mismatch is a typed
           failure result, handled as data rather than a trap): the mismatched decode `(decode
           Int64-schema (payload-of (Labeled \"z\")))` yields an `Err`, and matching it takes the `(Err _)`
           branch → -1. Pins that a schema mismatch is a matchable `Err` a fold over an open vocabulary
           handles as data — the whole point of returning a Result rather than trapping.")
  (input  (do
            (type Reading (Measured Int64) (Labeled String) .. r)
            (def (main)
              (match (decode Int64-schema (payload-of (Labeled "z")))
                ((Ok n) n)
                ((Err _) -1))) (export main)))
  (output (: -1 Int64)))

; --- OS2 schema-decode: the runtime, error-kind, and composition faces -----------------------------
; The OS2 cases above pin the Ok/Err decode over constant variants matched to a scalar. These pin the
; neighbors: a RUNTIME-built variant through the decode, the DecodeError KIND dispatch (the
; multi-variant error was a deliberate design call — its kinds must be matchable), the decoded
; payload feeding ordinary arithmetic, and Ok+Err decodes composing in one program.

(case "a runtime-built open-sum variant decodes through the schema"
  (doc    "`(decode Int64-schema (payload-of (Measured (+ k 1))))` with k a PARAMETER — the variant's
           payload is a runtime value, so nothing about the decode's INPUT folds; the schema still
           fixes the target type and the Ok arm recovers k+1 = 7. Pins the decode over a live payload
           (a const-fold-only path that declined runtime payloads would todo here; one that folded a
           placeholder would bind garbage).")
  (input  (do
            (type V (Measured Int64) (Labeled String) .. r)
            (def (main (: k Int64))
              (match (decode Int64-schema (payload-of (Measured (+ k 1))))
                ((Ok v) v)
                ((Err _) -1)))
            (export main)))
  (call   main (: 6 Int64))
  (output (: 7 Int64)))

(case "the decode error's kind is matchable"
  (doc    "`DecodeError` is a MULTI-variant sum (TypeMismatch | Eof) by design — an error names its
           kind. A mismatched decode's Err payload dispatches by kind: `(Err (TypeMismatch _))` → 1,
           the `(Err (Eof _))` arm stays untaken → not 2. Pins the error-kind surface (a
           single-variant or erased error would make the kind arms unreachable or ill-typed).")
  (input  (do
            (type V (Measured Int64) (Labeled String) .. r)
            (def (main (: d Int64))
              (match (decode Int64-schema (payload-of (Labeled "x")))
                ((Ok _) 0)
                ((Err (TypeMismatch _)) 1)
                ((Err (Eof _)) 2)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64)))

(case "a decoded payload feeds ordinary arithmetic"
  (doc    "`(+ (match (decode …) ((Ok v) v) …) 1)` = 8 — the recovered payload is a first-class Int64
           in downstream arithmetic (the schema's type-fixing is real: a payload carried as an opaque
           handle rather than the typed value would fail the add or compute garbage).")
  (input  (do
            (type V (Measured Int64) (Labeled String) .. r)
            (def (main (: d Int64))
              (+ (match (decode Int64-schema (payload-of (Measured 7)))
                   ((Ok v) v)
                   ((Err _) -1))
                 1))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 8 Int64)))

(case "an Ok and an Err decode compose through one shared handler"
  (doc    "TWO decodes in one program — one Ok (Measured 7 → 7), one Err (Labeled → -1) — folded
           through a SHARED `(Result Int64 DecodeError)`-typed helper: 7 + (-1) = 6. Pins the decode
           result as a first-class value crossing a function boundary, and that the two outcome
           shapes coexist in one compilation (a decode specialized per-call-site to only its observed
           outcome would reject or misdispatch the shared helper).")
  (input  (do
            (type V (Measured Int64) (Labeled String) .. r)
            (def (dec (: r (Result Int64 DecodeError)))
              (match r ((Ok v) v) ((Err _) -1)))
            (def (main (: d Int64))
              (+ (dec (decode Int64-schema (payload-of (Measured 7))))
                 (dec (decode Int64-schema (payload-of (Labeled "x"))))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 6 Int64)))

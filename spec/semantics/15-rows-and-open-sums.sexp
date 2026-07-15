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
           `(Record.extend (record (a 1)) (b 2))` yields `(record (a 1) (b 2))`. The added field may hold
           any type.")
  (input  (Record.extend (record (a 1)) (b 2)))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "extending a record with an already-present field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (1st sentence): adding a field the operand already contains is a compile-time rejection (CDZ0211),
           so `extend` never silently overwrites. `a` is already present, so this is a clobber `extend`
           forbids — the author means `Record.with` to replace. Rides the strict `Record.merge` disjointness
           its rewrite uses.")
  (input  (Record.extend (record (a 1)) (a 2)))
  (error  CDZ0211))

(case "updating a record field replaces its value"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence): `Record.with` replaces a field PRESENT in the operand, defined as `(Record.merge
           (Record.without r (z)) (record (z v)))`. `(Record.with (record (a 1) (b 2)) (b 9))` yields
           `(record (a 1) (b 9))` — an explicit update distinct from `extend`.")
  (input  (Record.with (record (a 1) (b 2)) (b 9)))
  (output (: (record (a 1) (b 9)) (Record (a Int64) (b Int64)))))

(case "updating a record field changes its type to the new value's"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence: 'a new value of a possibly different type'): the result is a new closed record
           whose field `b` has whatever type the new value holds. `(Record.with (record (a 1) (b 2)) (b
           true))` retypes `b` from Int64 to Bool, yielding `(record (a 1) (b true))` of type `(Record (a
           Int64) (b Bool))`. Pins that `with` is not constrained to the field's prior type.")
  (input  (Record.with (record (a 1) (b 2)) (b true)))
  (output (: (record (a 1) (b true)) (Record (a Int64) (b Bool)))))

(case "updating an absent record field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (3rd sentence): updating a field absent from the operand is a compile-time rejection (CDZ0212),
           not an addition, so `with` and `extend` stay distinct. `z` is not a field of `(record (a 1))`,
           so `Record.with` REJECTS — the author means `Record.extend` to add. Rides the `Record.without`
           presence check its rewrite uses.")
  (input  (Record.with (record (a 1)) (z 5)))
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

; --- Tuple reshaping: explicit positional operations yield a new tuple ----------------------
; type-system.md #A Tuple Is Reshaped Positionally By An Explicit Operation Yielding A New Value and its
; companions: `Tuple.cat` concatenates, `Tuple.split-at` splits at a static position, `Tuple.pop` takes
; element 0 off. A tuple's arity is part of its type, so every result arity is fixed statically and there
; is no disjointness constraint (positions are anonymous). `k` is a compile-time position written as a
; literal, exactly as `(. x N)` writes its index; a split outside `0..=len` is a type error, the `(. x N)`
; static-bounds rule (05-compound-types "tuple elements are accessed by index"). These ride
; the same later-generation rows layer and `Tuple.*` is an unbound name to the seed, so it declines them.

(case "concatenating two tuples appends their elements"
  (doc    "Witnesses type-system.md #Two Tuples Are Concatenated Into One Of Their Combined Length:
           `(Tuple.cat (tuple 1 2) (tuple 3 4))` yields `(tuple 1 2 3 4)` of arity 4 — the first tuple's
           elements in order followed by the second's, each keeping its source position's type.")
  (input  (Tuple.cat (tuple 1 2) (tuple 3 4)))
  (output (: (tuple 1 2 3 4) (Tuple Int64 Int64 Int64 Int64))))

(case "concatenating tuples preserves each element's type"
  (doc    "The heterogeneous companion: `(Tuple.cat (tuple 1 true) (tuple \"x\"))` yields `(tuple 1 true
           \"x\")` of type `(Tuple Int64 Bool String)`. Pins that concatenation keeps the type of each
           source position rather than unifying to one element type — a tuple is a heterogeneous product,
           unlike a homogeneous list.")
  (input  (Tuple.cat (tuple 1 true) (tuple "x")))
  (output (: (tuple 1 true "x") (Tuple Int64 Bool String))))

; The concatenation cases above build both operand tuples from CONSTANT literals, so the result folds to a
; constant tuple at compile time. A tuple carrying a RUNTIME element — a boundary parameter — cannot fold:
; the concatenation runs on the value heap, and reading an element back exercises the emitted machinery. A
; case reads the result down to a SCALAR (a projection then arithmetic) so it returns from a parameterized
; export. These pin `Tuple.cat` on a runtime operand — the value companion of the constant cases.

(case "concatenating tuples with a runtime element reads elements from both operands"
  (doc    "`(Tuple.cat (tuple n 2) (tuple 3 4))` with `n` a boundary parameter cannot fold — the first
           element is decided at run time, so the concatenation runs on the value heap. Reading element 0
           (the runtime `n`, from the first operand) and element 3 (4, from the second) and summing them
           yields `n + 4`: 7+4 = 11. Pins that a runtime `Tuple.cat` places BOTH operands' elements into
           the result at their combined positions, read back correctly by projection.")
  (input  (do (def (main (: n Int64))
                (+ (. (Tuple.cat (tuple n 2) (tuple 3 4)) 0) (. (Tuple.cat (tuple n 2) (tuple 3 4)) 3))) (export main)))
  (call   main (: 7 Int64)) (output (: 11 Int64))
  (call   main (: 100 Int64)) (output (: 104 Int64)))

(case "runtime tuple concatenation preserves element order across the seam"
  (doc    "`(. (Tuple.cat (tuple 1 2) (tuple n 4)) 2)` reads position 2 of the concatenation — the FIRST
           element of the second operand (`n`) — which lands just past the first operand's two elements. It
           is `n` for every `n` (99 → 99). Pins that the second operand's elements are appended AFTER the
           first's on the runtime path, so position 2 is the second tuple's element 0, not a first-operand
           element or a shifted slot.")
  (input  (do (def (main (: n Int64)) (. (Tuple.cat (tuple 1 2) (tuple n 4)) 2)) (export main)))
  (call   main (: 99 Int64)) (output (: 99 Int64))
  (call   main (: -7 Int64)) (output (: -7 Int64)))

(case "concatenating an empty tuple on the left is the identity"
  (doc    "The empty tuple `(tuple)` — which IS the unit value (core-semantics.md #The Empty Tuple Is The
           Unit Value) — is the identity of `Tuple.cat`: `(Tuple.cat (tuple) (tuple 1 2))` prepends no
           elements, so the result is `(tuple 1 2)`. Pins the empty-operand identity the existing cat cases
           (which join two non-empty tuples) do not exercise — the tuple companion of the empty-string /
           empty-bytes concatenation-identity cases.")
  (input  (Tuple.cat (tuple) (tuple 1 2)))
  (output (: (tuple 1 2) (Tuple Int64 Int64))))

(case "concatenating an empty tuple on the right is the identity"
  (doc    "The mirror: `(Tuple.cat (tuple 1 2) (tuple))` appends no elements, so the result is `(tuple 1
           2)`. Pins that the empty tuple is the identity on the right as well as the left, so a cat with an
           empty operand on either side is a no-op on value.")
  (input  (Tuple.cat (tuple 1 2) (tuple)))
  (output (: (tuple 1 2) (Tuple Int64 Int64))))

(case "concatenating two empty tuples is the empty tuple"
  (doc    "The degenerate boundary: `(Tuple.cat (tuple) (tuple))` joins nothing to nothing, yielding the
           empty tuple `(tuple)` — the unit value. Pins that cat handles the zero+zero case, not
           underflowing or producing a novel form, the tuple companion of the empty+empty string/bytes/set
           cases.")
  (input  (Tuple.cat (tuple) (tuple)))
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
  (doc    "Witnesses type-system.md #A Tuple Is Reshaped Positionally: `Tuple.pop` takes element 0 off,
           `(tuple (. t 0) <rest>)` — the positional analogue of `Record.pop`. `(Tuple.pop (tuple 1 2
           3))` yields `(tuple 1 (tuple 2 3))`. It is `(Tuple.split-at t 1)` with the singleton prefix
           unwrapped to its element.")
  (input  (Tuple.pop (tuple 1 2 3)))
  (output (: (tuple 1 (tuple 2 3)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "popping a tuple with a runtime element separates the head from the rest"
  (doc    "The runtime companion: `(Tuple.pop (tuple n 20 30))` with `n` a boundary parameter splits the
           head element 0 (`n`) from the rest `(tuple 20 30)`, built on the value heap because `n` is
           runtime. Reading the popped head (`.0` = `n`) and the rest's last element (`.1 .1` = 30) and
           summing gives `n + 30`: 9+30 = 39. Pins that a runtime `Tuple.pop` places the operand's element
           0 as the head and the remaining elements as the rest tuple, both read back by projection.")
  (input  (do (def (main (: n Int64))
                (+ (. (Tuple.pop (tuple n 20 30)) 0) (. (. (Tuple.pop (tuple n 20 30)) 1) 1))) (export main)))
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

(case "an open sum's payload decodes against a schema to a typed result"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed: a variant's payload is
           decoded against a schema resolved at run time, yielding a typed Ok result on a match. A
           successful decode of an Int64 payload yields (Ok 7).")
  (input  (do
            (def (main)
              (decode Int64-schema (payload-of (Measured 7)))) (export main)))
  (output (: (Ok 7) (Result Int64 DecodeError))))

(case "an open sum payload that does not match its schema yields a typed failure, not a trap"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed (a mismatch yields a typed
           failure result rather than a trap): decoding a String payload against an Int64 schema yields
           an Err, so a fold over an open vocabulary handles a malformed payload as data rather than
           halting.")
  (input  (do
            (def (main)
              (decode Int64-schema (payload-of (Labeled "x")))) (export main)))
  (output (: (Err (DecodeError unit)) (Result Int64 DecodeError))))

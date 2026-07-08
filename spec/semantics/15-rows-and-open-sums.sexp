; Rows and open sums — witnesses type-system.md #Records Are Rows, Open By Default Under Inference,
; #A Sum Type May Be Open, With A Mandatory Open-Tail Arm, and #An Open Sum's Payload May Be
; Schema-Typed. These are (needs rows) / (needs open-sums) cases a later generation realizes; the seed
; realizes closed records and closed sums (05-compound-types) but not row polymorphism or open sums.
; The primary clause is the recorded oracle: a well-typed program's value, or — for an ill-typed one —
; its (error <CODE>) rejection (a rule a generation does not yet cover is declined, not run).

(case "a function open over a record's extra fields accepts any record with the used field"
  (doc    "Witnesses type-system.md #Records Are Rows, Open By Default Under Inference: `get-x` uses only
           field `x`, so it is typed open over the other fields and accepts a record that also has `y`.
           Row polymorphism, not a fixed shape, is what inference assigns.")
  (needs  rows)
  (input  (module m
            (def (get-x r) (. r x))
            (def (main) (get-x (record (x 1) (y 2))))))
  (output (: 1 Int64)))

(case "subset record comparison is explicit projection, not an overloaded equality"
  (doc    "Witnesses type-system.md #Records Are Rows (subset comparison is explicit projection-then-=):
           comparing a two-field record against a one-field record by first projecting the shared field
           yields true; `=` is never silently widened to ignore the extra field.")
  (needs  rows)
  (input  (module m
            (def (main)
              (= (. (record (x 1) (y 2)) x)
                 (. (record (x 1)) x)))))
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
; literal writes names — not a runtime value. These are `(needs rows)` cases (the same tag the open-record
; cases above carry): the seed does not realize row inference, and `Record.*` is an unbound name to it, so
; it SKIPS them rather than rejecting the unbound prelude name (a gate FAIL) — the `(needs sets)` discipline.

(case "projecting a record restricts it to the named fields"
  (doc    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields: `Record.project`
           narrows a record to exactly the stated field names, each bound to the value the operand holds.
           `(Record.project (record (a 1) (b 2) (c 3)) (a c))` keeps `a` and `c`, dropping `b`, yielding
           the closed record `(record (a 1) (c 3))`. The result renders in canonical key-sorted order.")
  (needs  rows)
  (input  (Record.project (record (a 1) (b 2) (c 3)) (a c)))
  (output (: (record (a 1) (c 3)) (Record (a Int64) (c Int64)))))

(case "projecting a record onto an absent field is rejected"
  (doc    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields (2nd sentence):
           a projection naming a field the operand does not contain is a compile-time rejection (CDZ0212),
           so a projection cannot silently produce a field the operand never held. `z` is not a field of
           `(record (a 1) (b 2))`.")
  (needs  rows)
  (input  (Record.project (record (a 1) (b 2)) (a z)))
  (error  CDZ0212))

(case "dropping fields from a record leaves the remaining fields"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields:
           `Record.without` derives the record of the operand's fields EXCEPT those named. `(Record.without
           (record (a 1) (b 2) (c 3)) (b))` drops `b`, yielding `(record (a 1) (c 3))` — the complement of
           projecting the fields kept.")
  (needs  rows)
  (input  (Record.without (record (a 1) (b 2) (c 3)) (b)))
  (output (: (record (a 1) (c 3)) (Record (a Int64) (c Int64)))))

(case "dropping an absent field from a record is rejected"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence): dropping a field the operand does not contain is a compile-time rejection (CDZ0212),
           not a silent no-op. `z` is not a field of `(record (a 1))`.")
  (needs  rows)
  (input  (Record.without (record (a 1)) (z)))
  (error  CDZ0212))

(case "merging two records with disjoint fields unions their fields"
  (doc    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint:
           `Record.merge` combines two records into one whose field set is the union, each field bound to
           its source's value. `(Record.merge (record (a 1)) (record (b 2)))` yields `(record (a 1) (b 2))`
           — the row analogue of forming a record from two groups of fields.")
  (needs  rows)
  (input  (Record.merge (record (a 1)) (record (b 2))))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "merging records that share a field name is rejected"
  (doc    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint (2nd
           sentence): merging two records that share a field name is a compile-time rejection (CDZ0211), so
           the combined record never has to choose which operand's value the shared field takes — the
           row-operation companion of the duplicate-field literal `(record (a 1) (a 2))` (CDZ0201). `a` is
           shared, so `Record.merge` REJECTS rather than picking a winner (no silent clobber).")
  (needs  rows)
  (input  (Record.merge (record (a 1)) (record (a 2))))
  (error  CDZ0211))

(case "extending a record adds a new field"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation:
           `Record.extend` adds a field ABSENT from the operand, defined as `(Record.merge r (record (z v)))`.
           `(Record.extend (record (a 1)) (b 2))` yields `(record (a 1) (b 2))`. The added field may hold
           any type.")
  (needs  rows)
  (input  (Record.extend (record (a 1)) (b 2)))
  (output (: (record (a 1) (b 2)) (Record (a Int64) (b Int64)))))

(case "extending a record with an already-present field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (1st sentence): adding a field the operand already contains is a compile-time rejection (CDZ0211),
           so `extend` never silently overwrites. `a` is already present, so this is a clobber `extend`
           forbids — the author means `Record.with` to replace. Rides the strict `Record.merge` disjointness
           its rewrite uses.")
  (needs  rows)
  (input  (Record.extend (record (a 1)) (a 2)))
  (error  CDZ0211))

(case "updating a record field replaces its value"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence): `Record.with` replaces a field PRESENT in the operand, defined as `(Record.merge
           (Record.without r (z)) (record (z v)))`. `(Record.with (record (a 1) (b 2)) (b 9))` yields
           `(record (a 1) (b 9))` — an explicit update distinct from `extend`.")
  (needs  rows)
  (input  (Record.with (record (a 1) (b 2)) (b 9)))
  (output (: (record (a 1) (b 9)) (Record (a Int64) (b Int64)))))

(case "updating a record field changes its type to the new value's"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence: 'a new value of a possibly different type'): the result is a new closed record
           whose field `b` has whatever type the new value holds. `(Record.with (record (a 1) (b 2)) (b
           true))` retypes `b` from Int64 to Bool, yielding `(record (a 1) (b true))` of type `(Record (a
           Int64) (b Bool))`. Pins that `with` is not constrained to the field's prior type.")
  (needs  rows)
  (input  (Record.with (record (a 1) (b 2)) (b true)))
  (output (: (record (a 1) (b true)) (Record (a Int64) (b Bool)))))

(case "updating an absent record field is rejected"
  (doc    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (3rd sentence): updating a field absent from the operand is a compile-time rejection (CDZ0212),
           not an addition, so `with` and `extend` stay distinct. `z` is not a field of `(record (a 1))`,
           so `Record.with` REJECTS — the author means `Record.extend` to add. Rides the `Record.without`
           presence check its rewrite uses.")
  (needs  rows)
  (input  (Record.with (record (a 1)) (z 5)))
  (error  CDZ0212))

(case "popping a field yields its value and the remaining record"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields and #A Field
           Is Added To Or Replaced In A Record By A Derived Operation: `Record.pop` takes a field OFF a
           record, defined as `(tuple (. r z) (Record.without r (z)))` — the field's value paired with the
           record of the remaining fields. `(Record.pop (record (a 1) (b 2)) a)` yields `(tuple 1 (record
           (b 2)))`. No Option: field presence is static, so a missing field is CDZ0212, not a runtime None
           (contrast `List.at` on a runtime index).")
  (needs  rows)
  (input  (Record.pop (record (a 1) (b 2)) a))
  (output (: (tuple 1 (record (b 2))) (Tuple Int64 (Record (b Int64))))))

(case "popping an absent field is rejected"
  (doc    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence), via `Record.pop`'s `Record.without` rewrite: popping a field the record does not
           contain is a compile-time rejection (CDZ0212), not a runtime None — a record field name is a
           static label, not a runtime index. `z` is absent from `(record (a 1))`.")
  (needs  rows)
  (input  (Record.pop (record (a 1)) z))
  (error  CDZ0212))

(case "record reshaping is subset comparison as explicit projection"
  (doc    "Witnesses type-system.md #Records Are Rows (4th sentence: subset comparison is explicit
           projection-then-`=`, never an overloaded `=`) with `Record.project` as the narrowing operation.
           `(= (Record.project (record (x 1) (y 2)) (x)) (record (x 1)))` projects the shared field to a
           closed one-field record and compares it by ordinary structural equality — true. The
           general-projection form of the plain-`.` subset-comparison case above; `=` is never widened to
           ignore `y`, `Record.project` narrows the shape first.")
  (needs  rows)
  (input  (= (Record.project (record (x 1) (y 2)) (x)) (record (x 1))))
  (output (: true Bool)))

; --- Tuple reshaping: explicit positional operations yield a new tuple ----------------------
; type-system.md #A Tuple Is Reshaped Positionally By An Explicit Operation Yielding A New Value and its
; companions: `Tuple.cat` concatenates, `Tuple.split-at` splits at a static position, `Tuple.pop` takes
; element 0 off. A tuple's arity is part of its type, so every result arity is fixed statically and there
; is no disjointness constraint (positions are anonymous). `k` is a compile-time position written as a
; literal, exactly as `tuple.N` writes its index; a split outside `0..=len` is a type error, the `tuple.N`
; static-bounds rule (05-compound-types "tuple elements are accessed by index"). `(needs rows)`: these ride
; the same later-generation layer and `Tuple.*` is an unbound name to the seed.

(case "concatenating two tuples appends their elements"
  (doc    "Witnesses type-system.md #Two Tuples Are Concatenated Into One Of Their Combined Length:
           `(Tuple.cat (tuple 1 2) (tuple 3 4))` yields `(tuple 1 2 3 4)` of arity 4 — the first tuple's
           elements in order followed by the second's, each keeping its source position's type.")
  (needs  rows)
  (input  (Tuple.cat (tuple 1 2) (tuple 3 4)))
  (output (: (tuple 1 2 3 4) (Tuple Int64 Int64 Int64 Int64))))

(case "concatenating tuples preserves each element's type"
  (doc    "The heterogeneous companion: `(Tuple.cat (tuple 1 true) (tuple \"x\"))` yields `(tuple 1 true
           \"x\")` of type `(Tuple Int64 Bool String)`. Pins that concatenation keeps the type of each
           source position rather than unifying to one element type — a tuple is a heterogeneous product,
           unlike a homogeneous list.")
  (needs  rows)
  (input  (Tuple.cat (tuple 1 true) (tuple "x")))
  (output (: (tuple 1 true "x") (Tuple Int64 Bool String))))

(case "splitting a tuple at a position yields a prefix and a suffix"
  (doc    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix:
           `(Tuple.split-at (tuple 1 2 3) 1)` splits at position 1 into a pair — the first element as a
           1-tuple prefix and the rest as a 2-tuple suffix — yielding `(tuple (tuple 1) (tuple 2 3))`. The
           position `k` is a compile-time literal.")
  (needs  rows)
  (input  (Tuple.split-at (tuple 1 2 3) 1))
  (output (: (tuple (tuple 1) (tuple 2 3)) (Tuple (Tuple Int64) (Tuple Int64 Int64)))))

(case "splitting a tuple at zero yields an empty prefix"
  (doc    "The degenerate boundary of #A Tuple Is Split At A Position Into A Prefix And A Suffix: a split at
           position 0 puts no elements before it, so the prefix is the empty tuple — which IS the unit value
           (core-semantics.md #The Empty Tuple Is The Unit Value: `unit` and `()` are the same value) — and
           the suffix is the whole tuple. `(Tuple.split-at (tuple 1 2) 0)` yields `(tuple unit (tuple 1 2))`,
           the prefix typed `Unit`. Pins that 0 is in range and the empty prefix is the unit value, not a
           novel zero-arity tuple form.")
  (needs  rows)
  (input  (Tuple.split-at (tuple 1 2) 0))
  (output (: (tuple unit (tuple 1 2)) (Tuple Unit (Tuple Int64 Int64)))))

(case "splitting a tuple beyond its arity is rejected"
  (doc    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix (2nd
           sentence): a split position outside the operand's static arity range is a type error (CDZ0201),
           consistent with an out-of-arity positional access `tuple.N` being rejected. `(tuple 1 2)` has
           arity 2, so a split at 5 names a position it does not have — rejected rather than producing a
           short suffix.")
  (needs  rows)
  (input  (Tuple.split-at (tuple 1 2) 5))
  (error  CDZ0201))

(case "popping a tuple yields element zero and the remaining tuple"
  (doc    "Witnesses type-system.md #A Tuple Is Reshaped Positionally: `Tuple.pop` takes element 0 off,
           `(tuple (tuple.0 t) <rest>)` — the positional analogue of `Record.pop`. `(Tuple.pop (tuple 1 2
           3))` yields `(tuple 1 (tuple 2 3))`. It is `(Tuple.split-at t 1)` with the singleton prefix
           unwrapped to its element.")
  (needs  rows)
  (input  (Tuple.pop (tuple 1 2 3)))
  (output (: (tuple 1 (tuple 2 3)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "a match on an open sum with an open-tail arm is exhaustive"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: an open sum
           carries variants the module does not close; a match covering the known variant plus an
           open-tail arm is exhaustive and handles an unknown variant as data.")
  (needs  open-sums)
  (input  (module m
            (def (name-of e)
              (match e
                ((Known _) "known")
                (_         "other")))
            (def (main) (name-of (Known unit)))))
  (output (: "known" String)))

(case "a match on an open sum omitting the open-tail arm is rejected"
  (doc    "Witnesses type-system.md #A Sum Type May Be Open (a match that omits the open-tail arm is a
           compile-time rejection): because an open sum's variant set is not closed, a match without an
           open-tail arm cannot be exhaustive and is rejected (CDZ0210) rather than run.")
  (needs  open-sums)
  (input  (module m
            (def (name-of e)
              (match e
                ((Known _) "known")))
            (def (main) (name-of (Unknown unit)))))
  (error  CDZ0210))

(case "an open sum's payload decodes against a schema to a typed result"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed: a variant's payload is
           decoded against a schema resolved at run time, yielding a typed Ok result on a match. A
           successful decode of an Int64 payload yields (Ok 7).")
  (needs  open-sums)
  (input  (module m
            (def (main)
              (decode Int64-schema (payload-of (Measured 7))))))
  (output (: (Ok 7) (Result Int64 DecodeError))))

(case "an open sum payload that does not match its schema yields a typed failure, not a trap"
  (doc    "Witnesses type-system.md #An Open Sum's Payload May Be Schema-Typed (a mismatch yields a typed
           failure result rather than a trap): decoding a String payload against an Int64 schema yields
           an Err, so a fold over an open vocabulary handles a malformed payload as data rather than
           halting.")
  (needs  open-sums)
  (input  (module m
            (def (main)
              (decode Int64-schema (payload-of (Labeled "x"))))))
  (output (: (Err (DecodeError unit)) (Result Int64 DecodeError))))

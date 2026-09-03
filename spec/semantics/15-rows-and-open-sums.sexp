; Rows and open sums — witnesses type-system.md #Records Are Rows, Open By Default Under Inference
; and #A Sum Type May Be Open, With A Mandatory Open-Tail Arm. These exercise rows / open-sums, which a later generation realizes; the seed
; realizes closed records and closed sums (05-compound-types) but not row polymorphism or open sums.
; The primary clause is the recorded oracle: a well-typed program's value, or — for an ill-typed one —
; its (error <CODE>) rejection (a rule a generation does not yet cover is declined, not run).
(diagnostic-quality)

(case
  "a function open over a record's extra fields accepts any record with the used field"
  (doc
    "Witnesses type-system.md #Records Are Rows, Open By Default Under Inference: `get-x` uses only
           field `x`, so it is typed open over the other fields and accepts a record that also has `y`.
           Row polymorphism, not a fixed shape, is what inference assigns.")
  (input (do (def (get-x r) r.x) (def (main) (get-x #record((= x 1) (= y 2)))) (export main)))
  (output (: 1 Int64)))

(case
  "an open-row function is applied at TWO different record widths in one program"
  (doc
    "The two-instantiation face of row polymorphism (the case above calls `get-x` at ONE shape):
           `get-x` is applied to a 1-field record `(record (x n))` AND a 3-field `(record (x 10) (y 20)
           (z 30))` in the SAME program, summing both projections — n + 10 = 15 at n=5. The open row must
           instantiate per CALL SITE (the two records have different layouts — field `x` sits at a
           different physical position in the 1-field and the sorted 3-field erasure), so the projection
           must resolve x's slot per instantiation, not once for the def. A single-layout specialization
           would read the wrong slot at one site.")
  (input
    (do
      (def (get-x r) r.x)
      (def
        (main (: n Int64))
        (+ (get-x #record((= x n))) (get-x #record((= x 10) (= y 20) (= z 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a TWO-field open-row projection instantiates at three widths with slot-shifting extras"
  (doc
    "The case above projects ONE field at two widths; here `get-xy` projects x AND y across
           THREE widths whose extras shift BOTH slots differently per instantiation — under the
           sorted-field erasure x sits at slot 0/1/1 and y at 1/2/2 in (x y) / (a x y) / (w x y z).
           A per-def single-layout specialization, or a slot fix-up applied to only ONE projected
           field, misreads at some site; the runtime `a n` in the middle record blocks folding the
           whole program away. 12 + 34·100 + 67·10000 = 673412.")
  (input
    (do
      (def (get-xy r) (+ (* r.x 10) r.y))
      (def
        (main (: n Int64))
        (+
          (get-xy #record((= x 1) (= y 2)))
          (+
            (* (get-xy #record((= a n) (= x 3) (= y 4))) 100)
            (* (get-xy #record((= w 9) (= x 6) (= y 7) (= z 8))) 10000))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 673412 Int64)))

(case
  "an open-row record passed through a projecting helper keeps its EXTRA fields readable"
  (doc
    "The PASS-THROUGH face: `touch` projects x then returns its record UNCHANGED; the caller
           reads x AND both extras (y, and a runtime z) AFTER the round-trip. The helper's open type
           must not narrow the value to just-the-used-fields at the return boundary — a width-
           narrowing specialization, or a project-then-rebuild that drops extras, breaks the
           post-call reads. z=0 face guards the sum encode. 3+40+500 = 543; 3+40+0 = 43.")
  (input
    (do
      (def (touch r) (do (def _probe r.x) r))
      (def
        (main (: n Int64))
        (do (def out (touch #record((= x 3) (= y 40) (= z n)))) (+ out.x (+ out.y out.z))))
      (export main)))
  (call main (: 500 Int64))
  (output (: 543 Int64))
  (call main (: 0 Int64))
  (output (: 43 Int64)))

(case
  "Record.merge carries HEAP-list fields from both operands into the merged layout"
  (doc
    "The merge pins carry scalars; these fields are LISTS (one runtime-valued) — the disjoint
           merge must carry both heap HANDLES into the merged sorted layout, both projected+folded
           after (a merge copying field slots as scalars corrupts a handle).")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def m (Record.merge #record((= xs #list(1 n))) #record((= ys #list(7)))))
          (+ (* (sum-l m.xs 0) 10) (sum-l m.ys 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 37 Int64))
  (call main (: 0 Int64))
  (output (: 17 Int64))
  (live-objects 0))

(case
  "Record.with REPLACES one heap-list field while the sibling and the ORIGINAL stay live"
  (doc
    "The update path's heap discipline: one LIST field replaced, the runtime-valued sibling
           carries through, and the ORIGINAL record's replaced field stays live (persistence: r
           reads [1 2] after r2 replaced it) — three heap handles across two record generations.")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def r #record((= xs #list(1 2)) (= ys #list(7 n))))
          (def r2 (Record.with r #"xs" #list(9)))
          (+ (* (sum-l r2.xs 0) 100) (+ (* (sum-l r2.ys 0) 10) (sum-l r.xs 0)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1003 Int64))
  (call main (: 0 Int64))
  (output (: 973 Int64))
  (live-objects 0))

(case
  "an OPEN-sum value stored as a MAP VALUE matches back out with its open-tail arm"
  (doc
    "The nesting pins put open sums in SUM payloads; this is the collection slot — stored as
           map values, looked up, matched with named + open-tail + miss arms (the rep round-trips
           the CHAMP value slot with its tag intact).")
  (input
    (do
      (type Ev (A Int64) (B Int64) .. r)
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 (Ev.A 10)) (= 2 (Ev.B 7))))
          (match (Map.lookup m k) ((Some (A n)) (* n 10)) ((Some _) 7) ((None _u) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 100 Int64))
  (call main (: 2 Int64))
  (output (: 7 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a record whose field is a List projects the list handle and indexes it, alongside a scalar field"
  (doc
    "A record field may itself be a variable-length collection — distinct from a fixed-shape tuple
           field (which the ABI flattens depth-first): a `List` field is a HEAP HANDLE that must round-trip
           through the record's field slot. `r = (record (xs (list 10 20 30)) (n 5))`: `(. r xs)` projects
           the list handle, `List.at … i` indexes it at run time, and `(. r n)` reads the sibling scalar
           independently. Encodes `100·xs[i] + n`: i=0 → 100·10+5 = 1005, i=1 → 2005, i=2 → 3005. Pins that a
           heap-collection record field survives projection as a usable list (the record analogue of the
           Map-with-list-value case), and the sibling scalar reads independently on both backends.")
  (input
    (do
      (def
        (main (: i Int64))
        (let
          ((r #record((= xs #list(10 20 30)) (= n 5))))
          (+ (* 100 (Option.expect (List.at r.xs i) "idx")) r.n)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1005 Int64))
  (call main (: 1 Int64))
  (output (: 2005 Int64))
  (call main (: 2 Int64))
  (output (: 3005 Int64)))

(case
  "subset record comparison is explicit projection, not an overloaded equality"
  (doc
    "Witnesses type-system.md #Records Are Rows (subset comparison is explicit projection-then-=):
           comparing a two-field record against a one-field record by first projecting the shared field
           yields true; `=` is never silently widened to ignore the extra field.")
  (input (do (def (main) (= (. #record((= x 1) (= y 2)) x) (. #record((= x 1)) x))) (export main)))
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
(case
  "projecting a record restricts it to the named fields"
  (doc
    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields: `Record.project`
           narrows a record to exactly the stated field names, each bound to the value the operand holds.
           `(Record.project (record (a 1) (b 2) (c 3)) (a c))` keeps `a` and `c`, dropping `b`, yielding
           the closed record `(record (a 1) (c 3))`. The result renders in canonical key-sorted order.")
  (input (Record.project #record((= a 1) (= b 2) (= c 3)) (a c)))
  (output (: #record((= a 1) (= c 3)) (Record (: a Int64) (: c Int64)))))

(case
  "a row op over a constant record folds through a single-use let binding"
  (doc
    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields (the compile-time
           fold): a `Record.project`-then-`.` over a record bound by a `let` folds to the field's value when
           the binding is a compile-time-constant record. `r` is `(record (f 5) (g 8))`; projecting `f` and
           reading `.f` yields `5`. The control for the multi-use case below — a fold that must see through a
           let binding, not only a record literal written inline at the projection site.")
  (input
    (do
      (def (main) (let ((r #record((= f 5) (= g 8)))) (. (Record.project r (f)) f)))
      (export main)))
  (output (: 5 Int64)))

(case
  "a row op over a constant record folds through a multi-use let binding"
  (doc
    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields (the compile-time
           fold sees through a SHARED binding): a constant record bound once and PROJECTED TWICE folds at
           each site, not just the first. `r` is `(record (f 5) (g 8))`; `.f` over `(project r (f))` is `5`
           and `.g` over `(project r (g))` is `8`, summing to `13`. Guards against a regression where a
           multi-use `let` binding is seen as a runtime (Core::LocalRef) record and the row op wrongly
           declines with a misleading \"runtime record\" message even though every value is constant — the
           fold must follow the binder to the constant record at every use, not only a singly-used one.")
  (input
    (do
      (def
        (main)
        (let
          ((r #record((= f 5) (= g 8))))
          (+ (. (Record.project r (f)) f) (. (Record.project r (g)) g))))
      (export main)))
  (output (: 13 Int64)))

; The project/without cases above are all CONST-foldable (record literals, or let-bound constants the
; fold sees through). These pin the RUNTIME-leaf path: boundary parameters flow into the record's
; fields, so the row op must EMIT (build the restricted/reduced record from a live heap record), not
; fold — the shape a compiler pass takes restricting a runtime environment record.
(case
  "Record.project over runtime field values keeps the named fields and their live values"
  (doc
    "`(record (x a) (y b) (z 99))` with a and b RUNTIME parameters — the record is a live heap value,
           not a foldable constant. Projecting `(x y)` builds the restricted record; both projections read
           the runtime values: 10·a + b = 37 at a=3, b=7. Pins the runtime `Record.project` emit (heap
           record in, restricted heap record out, field values preserved) — the const cases above never
           reach it. Expected: 37.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((r #record((= x a) (= y b) (= z 99))))
          (let ((p (Record.project r (x y)))) (+ (* 10 p.x) p.y))))
      (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: 37 Int64)))

(case
  "Record.without over runtime field values drops the named field and keeps the rest live"
  (doc
    "The `without` twin: `(record (x a) (y 2))` with a runtime `a`, dropping `y` — the reduced
           record still carries the LIVE runtime value of `x` (5 at a=5). Pins the runtime `Record.without`
           emit; a fold-only implementation (or one that rebuilt the record from stale constants) would
           decline or lose the boundary value. Expected: 5.")
  (input
    (do
      (def
        (main (: a Int64))
        (let ((r #record((= x a) (= y 2)))) (let ((w (Record.without r (y)))) w.x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "projecting a record onto an absent field is rejected"
  (doc
    "Witnesses type-system.md #A Record Is Restricted To A Named Set Of Its Fields (2nd sentence):
           a projection naming a field the operand does not contain is a compile-time rejection (CDZ0212),
           so a projection cannot silently produce a field the operand never held. `z` is not a field of
           `(record (a 1) (b 2))`.")
  (input (Record.project #record((= a 1) (= b 2)) (a z)))
  (error CDZ0212))

; The `.`-ACCESS twin: a `.`-access of an ABSENT field on a genuine record is the SAME user error as a
; Record.project onto an absent field, so it gets the SAME code CDZ0212 (AbsentField), not the generic
; CDZ0201 (a code-keying inconsistency the flip fixed). The flip is NARROW — it fires ONLY on a genuine
; record (member_category = "field"); a module MEMBER miss and a sum-type VARIANT miss keep CDZ0201 (their
; own category word). Migrated from rcdzc an_absent_record_field_access_is_cdz0212_like_record_project.
(case
  "a dot-access of an absent record field is CDZ0212, the Record.project twin"
  (input (do (def (main) (. #record((= x 1)) z)) (export main)))
  (error CDZ0212))

(case
  "a dot-access of an absent field on a let-bound record is also CDZ0212"
  (input (do (def (main) (let ((p #record((= x 1)))) p.z)) (export main)))
  (error CDZ0212))

(case
  "a user-module member miss stays CDZ0201 (not a record field)"
  (input
    (do
      (module m
        (def (pub x) (+ x 1))

        (export pub))
      (def (main) (m.secret 5))
      (export main)))
  (error CDZ0201))

(case
  "a sum-type variant miss stays CDZ0201 (not a record field)"
  (input (do (type T (A Int64) (B Int64)) (def (main) T.nonesuch) (export main)))
  (error CDZ0201))

(case
  "projecting a record with a duplicate label is rejected"
  (doc
    "A record's fields are a fixed SET of statically-known names (type-system.md #A Record Is
           Restricted To A Named Set Of Its Fields), so a projection label list that names a field TWICE
           — `(Record.project (record (a 1) (b 2)) (a a))` — is the same malformedness a record LITERAL
           with a duplicate field `(record (a 1) (a 2))` is rejected for (CDZ0201), not silently
           deduplicated to a single field. A duplicate label is almost always an author error (a typo, a
           copy-paste); the projection label-list check matches the record-literal duplicate-field check.")
  (input (do (def (main) (. (Record.project #record((= a 1) (= b 2)) (a a)) a)) (export main)))
  (error CDZ0201 (message "more than once")))

(case
  "projecting a record with a TRIPLED label is rejected (the duplicate-label check is not adjacency-limited to a pair)"
  (doc
    "The duplicate-projection-label reject fires on ANY repeat count, not just a pair: `(a a a)` names
           `a` three times and is CDZ0201 like `(a a)`. Pins that the label-list linearity check counts
           occurrences rather than only comparing adjacent neighbours.")
  (input (do (def (main) (. (Record.project #record((= a 1) (= b 2)) (a a a)) a)) (export main)))
  (error CDZ0201 (message "more than once")))

(case
  "projecting a record with a NON-ADJACENT duplicate label is rejected (a repeat separated by another label still rejects)"
  (doc
    "The duplicate is caught regardless of position: `(a b a)` repeats `a` with `b` between the two
           occurrences and is CDZ0201 — the check is over the whole label SET, not adjacent pairs. Guards
           against a naive neighbour-only scan that would miss a separated repeat.")
  (input
    (do (def (main) (. (Record.project #record((= a 1) (= b 2) (= c 3)) (a b a)) a)) (export main)))
  (error CDZ0201 (message "more than once")))

(case
  "dropping fields from a record leaves the remaining fields"
  (doc
    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields:
           `Record.without` derives the record of the operand's fields EXCEPT those named. `(Record.without
           (record (a 1) (b 2) (c 3)) (b))` drops `b`, yielding `(record (a 1) (c 3))` — the complement of
           projecting the fields kept.")
  (input (Record.without #record((= a 1) (= b 2) (= c 3)) (b)))
  (output (: #record((= a 1) (= c 3)) (Record (: a Int64) (: c Int64)))))

(case
  "dropping an absent field from a record is rejected"
  (doc
    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence): dropping a field the operand does not contain is a compile-time rejection (CDZ0212),
           not a silent no-op. `z` is not a field of `(record (a 1))`.")
  (input (Record.without #record((= a 1)) (z)))
  (error CDZ0212))

; A CDZ0212 absent-field label carries the same two-tier did-you-mean a member access `(. r k)` gets — the
; closed set is the operand record's OWN fields. A NEAR-MISS of a real field (`alpa` vs `alpha`) gets a
; confident "did you mean `alpha`?" plus an APPLICABLE replace fix on the label occurrence (`alpa` → `alpha`);
; a FAR label with no plausible neighbour (`zzzzzz`) gets NO fix and no confident single, but LISTS the
; available fields ("closest matches: `alpha`") so the author sees what exists instead of dead-ending.
; (Migrated from rcdzc record_project_narrows_to_named_fields_absent_field_is_cdz0212.)
(case
  "a near-miss absent field in Record.without suggests the near field with a replace fix"
  (input (do (def (main) (Record.without #record((= alpha 1) (= beta 2)) (alpa))) (export main)))
  (error CDZ0212 (message "did you mean `alpha`?") (fix (kind replace) (replacement "alpha"))))

(case
  "a far absent-field label in Record.without lists the available fields with no confident fix"
  (input (do (def (main) (Record.without #record((= alpha 1)) (zzzzzz))) (export main)))
  (error CDZ0212 (message "closest matches: `alpha`") (no-fix)))

; The MEMBER-ACCESS face of the far-miss: `(. r zzzzzz)` where no field is within the edit-distance cutoff
; gets NEITHER a confident "did you mean?" (which would be a baseless guess) NOR a bare dead-end "no field",
; but LISTS the record's actual fields ("closest matches: `height`, `width`") — a closed, small field set is
; signal an author acts on. No fix (a list of options is not one mechanical edit) and no false single. The
; member-access twin of the far Record.without label above. (Migrated from rcdzc
; a_field_with_no_close_match_lists_the_available_fields.)
(case
  "a member-access field with no close match lists the available fields with no confident fix"
  (input (. #record((= width 10) (= height 20)) zzzzzz))
  (error
    CDZ0212
    (message "closest matches:")
    (message "`height`")
    (message "`width`")
    (not "did you mean")
    (no-fix)))

; The MEMBER-ACCESS companions of the near-miss field-typo above: a `(. r k)` where `k` is a near-miss of a
; real field carries the SAME confident "did you mean `<near>`?" + a HEURISTIC replace fix on the key token
; (diagnostics.md §A Diagnostic Carries A Route To A Fix) — the record analogue of the unbound-name did-you-
; mean. It fires on a COMPILE-TIME-VISIBLE record literal AND on a RUNTIME record reached through an untyped
; def parameter (the field typo is caught on the inferred record type). The rename guess is heuristic, so its
; fix is UNVERIFIED. (Migrated from rcdzc a_misspelled_field_on_a_visible_record_suggests_the_nearest_field +
; a_misspelled_field_on_a_runtime_record_type_suggests_the_nearest_field.)
(case
  "a misspelled field on a visible record literal suggests the nearest field with a heuristic rename fix"
  (input (. #record((= width 10) (= height 20)) heigth))
  (error
    CDZ0212
    (message "did you mean `height`?")
    (fix (kind replace) (replacement "height") (unverified))))

(case
  "a misspelled field on a runtime record reached through a parameter suggests the nearest field the same way"
  (input
    (do
      (def (get-h r) r.heigth)
      (def (main) (get-h #record((= width 10) (= height 20))))
      (export main)))
  (error
    CDZ0212
    (message "did you mean `height`?")
    (fix (kind replace) (replacement "height") (unverified))))

; The CONSTRUCTION twin of the member-access field typo above: a record literal supplied to a variant
; constructor whose payload is a `(Record …)` type, with one key a plausible typo of the expected field
; (`yy` for `y`). The reject (CDZ0201) (a) carries the structural field-diff TAIL (which fields are
; missing / not expected) so the reader is not left to diff two whole record renders, and (b) offers the same
; heuristic RENAME fix on the misspelled KEY token that a `(. r yy)` access typo gets — correcting the key to
; `y` clears the fault (the replace fix's target). An AMBIGUOUS two-field slip (`aa`/`bb`, neither a confident
; near-miss) still guides with the field-diff but offers NO auto-fix (not one confident edit). (Migrated from
; rcdzc a_misspelled_field_in_a_constructed_record_names_the_field_and_offers_a_rename.)
(case
  "a misspelled field in a constructed record names the field-diff and offers a rename fix"
  (input
    (do
      (type P (Mk (Record (: x Int64) (: y Int64))))
      (def (f) (P.Mk #record((= x 1) (= yy 2))))
      (export f)))
  (error
    CDZ0201
    (message "missing field `y`")
    (message "no such field `yy`")
    (fix (kind replace) (replacement "y") (unverified))))

(case
  "an ambiguous two-field constructed-record slip guides with the field-diff but offers no confident fix"
  (input
    (do
      (type P (Mk (Record (: x Int64) (: y Int64))))
      (def (f) (P.Mk #record((= aa 1) (= bb 2))))
      (export f)))
  (error CDZ0201 (message "missing fields") (no-fix)))

(case
  "merging two records with disjoint fields unions their fields"
  (doc
    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint:
           `Record.merge` combines two records into one whose field set is the union, each field bound to
           its source's value. `(Record.merge (record (a 1)) (record (b 2)))` yields `(record (a 1) (b 2))`
           — the row analogue of forming a record from two groups of fields.")
  (input (Record.merge #record((= a 1)) #record((= b 2))))
  (output (: #record((= a 1) (= b 2)) (Record (: a Int64) (: b Int64)))))

; The merge above builds both operand records from CONSTANT literals, so the union folds to a constant
; record. A record carrying a RUNTIME field value cannot fold — the merge runs on the value heap. These
; read the merged record's fields back down to a scalar (so a parameterized export returns), pinning that
; a runtime `Record.merge` carries EACH operand's field into the result at its own value.
(case
  "merging records with a runtime field value carries both operands' fields"
  (doc
    "`(Record.merge (record (a n)) (record (b 2)))` with `n` a boundary parameter builds the left
           record from a runtime value, so the merge runs on the value heap. Reading field `a` (the runtime
           `n`, from the left operand) plus field `b` (2, from the right) yields `n + 2`: 7+2 = 9, 100+2 =
           102. Pins that a runtime merge unions BOTH operands' fields, each bound to its source's value,
           read back by member access.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (. (Record.merge #record((= a n)) #record((= b 2))) a)
          (. (Record.merge #record((= a n)) #record((= b 2))) b)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 9 Int64))
  (call main (: 100 Int64))
  (output (: 102 Int64)))

(case
  "merging records with runtime values on both sides binds each field to its own value"
  (doc
    "Both operands carry a runtime field: `(Record.merge (record (a x)) (record (b y)))`. Reading `b`
           minus `a` yields `y - x` (10-3 = 7), so each field holds its OWN operand's runtime value — the
           merge does not confuse or alias the two runtime slots. Pins per-field value fidelity on the
           runtime path when neither operand is constant.")
  (input
    (do
      (def
        (main (: x Int64) (: y Int64))
        (-
          (. (Record.merge #record((= a x)) #record((= b y))) b)
          (. (Record.merge #record((= a x)) #record((= b y))) a)))
      (export main)))
  (call main (: 3 Int64) (: 10 Int64))
  (output (: 7 Int64))
  (call main (: 50 Int64) (: 8 Int64))
  (output (: -42 Int64)))

; The row-op cases above each exercise ONE op and read the result directly. These pin COMPOSITION:
; a row-op result flowing into an open-row function's parameter (the row variable must unify with the
; op's RESULT row, not a source literal's), and a multi-op pipeline whose intermediate rows exist only
; between ops. (An ANNOTATED closed-record parameter `(: r (Record (: x Int64)))` correctly rejects a
; wider argument — CDZ0203, width is exact when annotated; the open unannotated form is what composes.)
(case
  "an open-row function accepts a runtime MERGE result and reads the carried field"
  (doc
    "`get-x` (unannotated, open row) applied to `(Record.merge (record (x a)) (record (y 100)))` —
           the argument's row is the merge's RESULT union {x,y}, built at run time, not a source literal.
           The row variable must instantiate against the op's result row and resolve x's slot in the
           merged layout (sorted 2-field erasure). a=7 → 7. Pins row-polymorphic application over a
           computed record, the composition seam between the merge pins above and the open-row pins at
           the file top.")
  (input
    (do
      (def (get-x r) r.x)
      (def (main (: a Int64)) (get-x (Record.merge #record((= x a)) #record((= y 100)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64)))

(case
  "an open-row function over a Record.with result reads the UPDATED value"
  (doc
    "The update composition: `(Record.with (record (x 1) (y 2)) #\"x\" a)` replaces x with the runtime
           `a`, and the open-row `get-x` reads the update's result — 42, never the stale literal 1. Pins
           that the with-result's row (same fields, new value) flows through a row-variable application
           and the projection reads the POST-update slot.")
  (input
    (do
      (def (get-x r) r.x)
      (def (main (: a Int64)) (get-x (Record.with #record((= x 1) (= y 2)) #"x" a)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64)))

(case
  "a Record.with whose TARGET is a runtime record (a projection) builds a fresh record, not declines"
  (doc
    "The runtime-record row-op face (breaker l6; v-inference 49d6eec14). `Record.with` whose target is
           a genuinely RUNTIME record — here `(. outer pos)`, a PROJECTION of a nested-record param, not a
           compile-time literal — used to DECLINE 'a record row operation over a runtime record is not yet
           built' (lower.rs:22368): lower's row-ops only folded over a compile-time-visible Core::Record
           (const_record_fields, following LocalRef binders). The fix builds a FRESH record from per-field
           projections when const_record_fields misses but type_of is a concrete Ty::Record — the field set
           is static and the heap is immutable-shared, so no new runtime primitive. `bump` updates the inner
           `pos.y` through the projected sub-record `(. outer pos)`; the updated field takes the new value,
           the sibling `x` is preserved. `(bump p0 5)` → pos.y = 2+5 = 7, both backends. The inline-literal
           and flat-param forms always worked; this is the first to exercise a row-op on a DERIVED runtime
           record. project/without/merge/pop over a runtime record are a separate follow-up.")
  (input
    (do
      (def
        (bump
          (:
            outer
            (Record
              (: pos (Record (: x Int64) (: y Int64)))
              (: vel (Record (: x Int64) (: y Int64)))))
          (: d Int64))
        (Record.with outer #"pos" (Record.with outer.pos #"y" (+ outer.pos.y d))))
      (def
        (main (: d Int64))
        (do
          (def p0 #record((= pos #record((= x 1) (= y 2))) (= vel #record((= x 30) (= y 40)))))
          (. (. (bump p0 d) pos) y)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "a Record.with over a runtime record with MULTIPLE preserved fields evaluates the operand once"
  (doc
    "The materialize-once discipline for a runtime-record row-op (v-inference 13fd27095, after the
           reviewer's re-emit finding on 49d6eec14). `runtime_record_fields` builds a fresh record from a
           `(. record field)` projection for EVERY unchanged field; before the fix the raw operand was
           re-emitted once per preserved field (the backend has no CSE — each Core::Proj re-calls
           emit(operand)), an N-fold redundant eval (perf cliff for a pure operand). The fix let-binds the
           runtime operand ONCE (self-keyed Core::Let) so every projection reads a shared LocalRef. Here
           `(mk v)` is a 3-field runtime record; `Record.with … #\"a\" 99` updates `a` and leaves TWO preserved
           fields (`b`, `c`), so the operand would re-emit twice without the fix. Reading the preserved `c`
           → v+2 = 12 at v=10 (value correct either way; the pin LOCKS the multi-preserved-field path the
           single-field l6 case does not exercise). Both backends. The effect-count face is SHIELDED by the
           effect lowering (an effectful operand is materialized once by out-state threading) and guarded
           structurally by rcdzc's emit-once lib test, so no runtime perform-count row is needed.")
  (input
    (do
      (def (mk (: n Int64)) #record((= a n) (= b (+ n 1)) (= c (+ n 2))))
      (def (main (: v Int64)) (. (Record.with (mk v) #"a" 99) c))
      (export main)))
  (call main (: 10 Int64))
  (output (: 12 Int64)))

; ---- The rest of the row-op-over-runtime-record matrix (v-inference dc685f3a5 project/without +
; ef3fcdcdf merge/pop, batch #79). All route through the shared materialize_row_op_operand, so the operand
; emits once (no per-field re-emit). `mk`/`mkA`/`mkB` are recursion-forced so the record is genuinely
; runtime (not a folded literal); the distinctive `(+ v N)` operand constants are eval-once probes that
; v-inference's lib tests assert emit 1x — the corpus rows assert VALUE.
(case
  "a Record.project over a runtime record keeping two fields reads a kept field's value"
  (doc
    "Record.project over a RUNTIME record (recursion-forced `mk`) keeping {a,c} — was a decline 'not
           yet built'. Materializes the operand once; reads the kept first field `a` → 1.")
  (input
    (do
      (def (mk (: n Int64)) (if (= n 0) #record((= a 1) (= b 2) (= c 3)) (mk (- n 1))))
      (def (upd (: v Int64)) (. (Record.project (mk (+ v 987654321)) (a c)) a))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a Record.project over a runtime record reads a kept NON-FIRST field's value"
  (doc
    "The preserved-sibling face: the same runtime Record.project keeping {a,c}, reading the non-first
           kept field `c` → 3 — a kept field other than the first also reads correctly through the
           materialized operand.")
  (input
    (do
      (def (mk (: n Int64)) (if (= n 0) #record((= a 1) (= b 2) (= c 3)) (mk (- n 1))))
      (def (upd (: v Int64)) (. (Record.project (mk (+ v 987654321)) (a c)) c))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a Record.without over a runtime record dropping one field reads a surviving field's value"
  (doc
    "Record.without over a RUNTIME record dropping {b}, keeping {a,c} — was a decline. Materializes
           the operand once; reads the surviving first field `a` → 1. The drop-shifts-layout twin of the
           project case.")
  (input
    (do
      (def (mk (: n Int64)) (if (= n 0) #record((= a 1) (= b 2) (= c 3)) (mk (- n 1))))
      (def (upd (: v Int64)) (. (Record.without (mk (+ v 987654321)) (b)) a))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a Record.without over a runtime record reads a surviving NON-FIRST field after the drop"
  (doc
    "The surviving-sibling face: the same runtime Record.without dropping the MIDDLE field {b},
           reading the surviving `c` → 3 — after the drop shifts c's position it must still read correctly
           through the materialized operand.")
  (input
    (do
      (def (mk (: n Int64)) (if (= n 0) #record((= a 1) (= b 2) (= c 3)) (mk (- n 1))))
      (def (upd (: v Int64)) (. (Record.without (mk (+ v 987654321)) (b)) c))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a Record.pop over a runtime record splits off the named field's value"
  (doc
    "Record.pop over a RUNTIME record (recursion-forced `mk`) splits it into (popped-value, rest) at
           field `a` — was a decline. Materializes the operand once; reads tuple element 0 (the popped `a`)
           → 1.")
  (input
    (do
      (def (mk (: n Int64)) (if (= n 0) #record((= a 1) (= b 2) (= c 3)) (mk (- n 1))))
      (def (upd (: v Int64)) (. (Record.pop (mk (+ v 987654321)) a) 0))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a Record.merge of two runtime records reads a field from the second operand"
  (doc
    "Record.merge unions two disjoint RUNTIME records (both recursion-forced) — was a decline. Each
           operand materializes once (distinct eval-once probe constants per operand). Reads `c` from the
           SECOND operand `(mkB …)` = {c,d} → 3 — a field from the second operand's row survives the union
           at its correct slot.")
  (input
    (do
      (def (mkA (: n Int64)) (if (= n 0) #record((= a 1) (= b 2)) (mkA (- n 1))))
      (def (mkB (: n Int64)) (if (= n 0) #record((= c 3) (= d 4)) (mkB (- n 1))))
      (def (upd (: v Int64)) (. (Record.merge (mkA (+ v 987654321)) (mkB (+ v 111222333))) c))
      (def (main (: v Int64)) (upd v))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a row-op pipeline — merge then without then two projections — over runtime values"
  (doc
    "Three row ops composed, intermediates never named in source types: merge unions {x} and {y,z}
           (both carrying runtime values), `without` drops z, and BOTH survivors project out of the
           let-bound result — a + b = 42. Each op's result row feeds the next op's operand row; a
           pipeline that recomputed layout from a source literal (rather than the previous op's result)
           would misread a slot after the drop shifted positions.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((r (Record.without (Record.merge #record((= x a)) #record((= y b) (= z 9))) (z))))
          (+ r.x r.y)))
      (export main)))
  (call main (: 30 Int64) (: 12 Int64))
  (output (: 42 Int64)))

; --- Row ops over a record read out of a collection (CHAMP-boxed base) — finding #45 ------
; The composition pins above take their base record from a source literal or a merge result. These
; pin the row-op family over a base record read OUT OF A MAP (a CHAMP value slot) — the extend-of-
; without CHAIN in particular. The without-result must MATERIALIZE a fresh record rather than alias
; the CHAMP-boxed base, or the follow-on extend indexes off the wrong layout: finding #45 was a wasm
; miscompile (the chain over a map-looked-up record trapped OOB) fixed by materializing the intermediate
; (collect_row_op_field_dups, trunk 2fad5d246). Each op ALONE on the map-borne record, and the same
; chain over a list-borne record, always worked — so the trigger was exactly the composed chain over
; the CHAMP-boxed base. Pinned as regression witnesses beside the row-op composition matrix.
(case
  "Record.extend chained onto Record.without of a map-borne record computes, not traps"
  (doc
    "Finding #45 regression witness (breaker; wasm fix v-wasm-opt 2fad5d246, v-memory-safety
           co-verified). `r` is read out of a Map (a CHAMP value slot); the chain drops `qty` then re-adds
           it as `(. r qty) + 5` = 8. Pre-fix the wasm row-op emit reused the CHAMP-boxed base pointer for
           the without-result, so the follow-on extend indexed off the wrong base and trapped 'out of bounds
           memory access'; rust/rust-async always computed 8. The fix materializes the without intermediate
           before the extend. The single-op and list-borne faces (below) were the green perimeter.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def inv #map((= 1 #record((= name "widget") (= qty k)))))
          (def r (Option.expect (Map.lookup inv 1) "slot"))
          (. (Record.extend (Record.without r (qty)) #"qty" (+ r.qty 5)) qty)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 8 Int64))
  (live-objects known-leak))

(case
  "the same without-extend chain on a LIST-borne record computes"
  (doc
    "Finding #45 control: the identical extend-of-without chain, but the base record is read out of a
           LIST (List.at) rather than a Map. This face computed 8 on ALL backends even pre-fix — a
           list-borne record hands out a fresh handle, so the without-result never aliased a boxed base.
           Pins that the fix's trigger was specifically the CHAMP-boxed (map-valued) base, not row-op
           composition in general.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def r0 #record((= name "widget") (= qty k)))
          (def inv (List.push #list() r0))
          (def r (Option.expect (List.at inv 0) "slot"))
          (. (Record.extend (Record.without r (qty)) #"qty" (+ r.qty 5)) qty)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 8 Int64)))

(case
  "single row ops on a map-borne record compute — without alone and extend alone"
  (doc
    "Finding #45 control: each row op ALONE on the map-borne record — `Record.without r (qty)` then
           project name (byte-len 6, ×10) plus `Record.extend r #\"extra\" 5` then project extra (5) = 65.
           Both single ops computed correctly on all backends pre-fix; only the without→extend CHAIN over
           the CHAMP-boxed base trapped. Pins the single-op perimeter so the fix cannot regress it.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def inv #map((= 1 #record((= name "widget") (= qty k)))))
          (def r (Option.expect (Map.lookup inv 1) "slot"))
          (+
            (* 10 (String.byte-len (. (Record.without r (qty)) name)))
            (. (Record.extend r #"extra" 5) extra))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 65 Int64))
  (live-objects known-leak))

(case
  "a list-aliased record read after Record.with sees the ORIGINAL value (no in-place clobber)"
  (doc
    "The ALIAS face of Record.with persistence: the existing persistence pins read the original
           through its own BINDING; here the original is ALSO aliased into a LIST before the update, and
           the post-update read goes through the ALIAS (`List.at` then project). A Perceus reuse that
           treated the record as uniquely owned at the `with` (missing the list's dup) would update the
           shared payload in place and the alias would read the NEW x — this pins the alias reads the OLD
           one. done.x = k+1 = 4; alias.x = k = 3 → 4 + 100·3 = 304.")
  (input
    (do
      (def (bump-x (: r (Record (: x Int64) (: y Int64)))) (Record.with r #"x" (+ r.x 1)))
      (def
        (main (: k Int64))
        (let
          ((seed #record((= x k) (= y 100))))
          (let
            ((alias #list(seed)))
            (let
              ((done (bump-x seed)))
              (+ done.x (* 100 (match (List.at alias 0) ((Some a) a.x) ((None _u) -1))))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 304 Int64)))

(case
  "a record threaded through self-recursion with per-step Record.with leaves the SEED intact"
  (doc
    "The RECURSION face of Record.with persistence, guarding the owned-heap-param drop-epilogue
           seam: `go-n` is a single-member self-recursion whose record param is rebuilt per step
           (`bump-x` = a `with` over the previous value) and RETURNED at the base case — the exact shape
           the recursion-param epilogue gates on. The caller's seed must still read its original x after
           the recursion returns (the epilogue must not treat the borrowed-and-escaping param as uniquely
           dead, and the per-step `with` must not reuse the shared seed's cell). done.x = k+5 = 8;
           seed.x = k = 3 → 8 + 1000·3 = 3008.")
  (input
    (do
      (def (bump-x (: r (Record (: x Int64) (: y Int64)))) (Record.with r #"x" (+ r.x 1)))
      (def
        (go-n (: r (Record (: x Int64) (: y Int64))) (: n Int64))
        (if (> n 0) (go-n (bump-x r) (- n 1)) r))
      (def
        (main (: k Int64))
        (let
          ((seed #record((= x k) (= y 100))))
          (let ((done (go-n seed 5))) (+ done.x (* 1000 seed.x)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3008 Int64))
  (live-objects known-leak))

(case
  "the without-extend chain on a NESTED-map-borne record computes (boundary — double lookup materializes fresh)"
  (doc
    "Finding #45 boundary: the same chain but `r` is read via a DOUBLE lookup (a map nested in a map).
           This depth computed 8 on all backends even pre-fix, while the single-lookup form (first witness)
           trapped — consistent with the aliasing diagnosis: a record-valued CHAMP leaf handed out its boxed
           base (aliased, wrong), while a map-valued leaf handed out a fresh handle (materialized, correct).
           Pins the working depth so the wasm fix cannot regress it.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def inner #map((= 2 #record((= name "widget") (= qty k)))))
          (def outer #map((= 1 inner)))
          (def r (Option.expect (Map.lookup (Option.expect (Map.lookup outer 1) "o") 2) "i"))
          (. (Record.extend (Record.without r (qty)) #"qty" (+ r.qty 5)) qty)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 8 Int64))
  (live-objects known-leak))

(case
  "a without-extend re-wrap inside a sum payload keeps the map-borne record's new field value"
  (doc
    "Finding #45 witness 2 — the borrowed-operand face (wasm fix v-wasm-opt 8f18044a3, v-memory-safety
           co-verified). The extend-of-without chain runs INSIDE a sum-payload construction via a helper:
           `bump-qty` matches a `(Slot.Filled r)` (r read out of a Map) and rebuilds `(Slot.Filled (Record.extend
           (Record.without r (qty)) #\"qty\" (+ (. r qty) d)))`, then main re-projects qty = 3 + 5 = 8. Pre-fix
           wasm silently returned 5 (the borrowed `r`'s qty field was read AFTER the without dropped it — the
           new value was lost, k + d collapsed to d); rust/rust-async always computed 8. The SILENT wrong value
           (not a trap) made this soundness-priority. This closes #45's second face; the first (witness 1, the
           owned-operand field-dup trap) is the sibling pin above.")
  (input
    (do
      (type Slot (Filled (Record (: name String) (: qty Int64))) (Empty))
      (def
        (bump-qty (: s Slot) (: d Int64))
        (match
          s
          ((Slot.Filled r)
            (Slot.Filled (Record.extend (Record.without r (qty)) #"qty" (+ r.qty d))))
          ((Slot.Empty _u) (Slot.Empty unit))))
      (def
        (main (: k Int64))
        (do
          (def
            inv
            (Map.insert
              Map.empty
              1
              (Slot.Filled #record((= name (String.concat "wid" "get")) (= qty k)))))
          (def v (bump-qty (Option.expect (Map.lookup inv 1) "slot") 5))
          (match v ((Slot.Filled r) r.qty) ((Slot.Empty _u) -1))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 8 Int64))
  (live-objects known-leak))

(case
  "merging records that share a field name is rejected"
  (doc
    "Witnesses type-system.md #Two Records Are Combined Only When Their Field Sets Are Disjoint (2nd
           sentence): merging two records that share a field name is a compile-time rejection (CDZ0211), so
           the combined record never has to choose which operand's value the shared field takes — the
           row-operation companion of the duplicate-field literal `(record (a 1) (a 2))` (CDZ0201). `a` is
           shared, so `Record.merge` REJECTS rather than picking a winner (no silent clobber).")
  (input (Record.merge #record((= a 1)) #record((= a 2))))
  (error CDZ0211))

(case
  "merging with the empty record on the left is the identity"
  (doc
    "The empty record `(record)` has no fields, so it is trivially disjoint from any record and is the
           IDENTITY of `Record.merge`: `(Record.merge (record) (record (a 1) (b 2)))` equals `(record (a 1)
           (b 2))` — merging in nothing adds nothing. Pins the empty-operand identity the disjoint-merge
           cases above (which union two non-empty records) do not exercise — the record companion of the
           empty-list / empty-set / empty-tuple identity laws.")
  (input (= (Record.merge #record() #record((= a 1) (= b 2))) #record((= a 1) (= b 2))))
  (output (: true Bool)))

(case
  "merging with the empty record on the right is the identity"
  (doc
    "The mirror: `(Record.merge (record (a 1) (b 2)) (record))` equals `(record (a 1) (b 2))` — the
           empty record is the identity on the right as well as the left. Pins that a merge with an empty
           operand on either side is a no-op on value (merge is symmetric on the empty record).")
  (input (= (Record.merge #record((= a 1) (= b 2)) #record()) #record((= a 1) (= b 2))))
  (output (: true Bool)))

(case
  "merging two empty records is the empty record"
  (doc
    "The degenerate boundary: `(Record.merge (record) (record))` combines two field-less records into
           the empty record `(record)` — a genuine value equal to itself (its type is `(Record)`). Pins that
           merge handles the empty+empty case, the record companion of the empty+empty list/set/tuple
           cases, and that the empty record is a first-class value, not only a type-error foil.")
  (input (= (Record.merge #record() #record()) #record()))
  (output (: true Bool)))

(case
  "extending a record adds a new field"
  (doc
    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation:
           `Record.extend` adds a field ABSENT from the operand, defined as `(Record.merge r (record (z v)))`.
           `(Record.extend (record (a 1)) #"
    b" 2)` yields `(record (a 1) (b 2))`. The added field may hold\n           any type. The field name is a `#field` label operand (a static label, not a runtime value).")
  (input (Record.extend #record((= a 1)) #"b" 2))
  (output (: #record((= a 1) (= b 2)) (Record (: a Int64) (: b Int64)))))

(case
  "extending a record with an already-present field is rejected"
  (doc
    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (1st sentence): adding a field the operand already contains is a compile-time rejection (CDZ0211),
           so `extend` never silently overwrites. `a` is already present, so this is a clobber `extend`
           forbids — the author means `Record.with` to replace. Rides the strict `Record.merge` disjointness
           its rewrite uses.")
  (input (Record.extend #record((= a 1)) #"a" 2))
  (error CDZ0211))

(case
  "updating a record field replaces its value"
  (doc
    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence): `Record.with` replaces a field PRESENT in the operand, defined as `(Record.merge
           (Record.without r (z)) (record (z v)))`. `(Record.with (record (a 1) (b 2)) #"
    b" 9)` yields\n           `(record (a 1) (b 9))` \xe2\x80\x94 an explicit update distinct from `extend`.")
  (input (Record.with #record((= a 1) (= b 2)) #"b" 9))
  (output (: #record((= a 1) (= b 9)) (Record (: a Int64) (: b Int64)))))

(case
  "updating a record field changes its type to the new value's"
  (doc
    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (2nd sentence: 'a new value of a possibly different type'): the result is a new closed record
           whose field `b` has whatever type the new value holds. `(Record.with (record (a 1) (b 2)) #"
    b"\n           true)` retypes `b` from Int64 to Bool, yielding `(record (a 1) (b true))` of type `(Record (: a Int64) (: b Bool))`. Pins that `with` is not constrained to the field's prior type.")
  (input (Record.with #record((= a 1) (= b 2)) #"b" true))
  (output (: #record((= a 1) (= b true)) (Record (: a Int64) (: b Bool)))))

(case
  "Record.with over a RUNTIME field leaves the original record readable (persistence)"
  (doc
    "The runtime + persistence face of `Record.with` (the pins above are const-folded whole-value
           outputs): the operand record carries a RUNTIME field `x = n`, `Record.with` replaces `x` with
           99, and BOTH records are then read — the updated `r2.x` (99) and `r2.y` (20, the untouched field
           carried over), AND the ORIGINAL `r.x` (still `n` — the update must not mutate its operand
           through the shared binding). Encodes 100·r2.x + 10·r2.y + r.x = 9900+200+5 = 10105 at n=5. The
           record companion of the Map/Set persistence pins.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((r #record((= x n) (= y 20))))
          (let ((r2 (Record.with r #"x" 99))) (+ (* 100 r2.x) (+ (* 10 r2.y) r.x)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10105 Int64)))

(case
  "updating an absent record field is rejected"
  (doc
    "Witnesses type-system.md #A Field Is Added To Or Replaced In A Record By A Derived Operation
           (3rd sentence): updating a field absent from the operand is a compile-time rejection (CDZ0212),
           not an addition, so `with` and `extend` stay distinct. `z` is not a field of `(record (a 1))`,
           so `Record.with` REJECTS — the author means `Record.extend` to add. Rides the `Record.without`
           presence check its rewrite uses.")
  (input (Record.with #record((= a 1)) #"z" 5))
  (error CDZ0212))

(case
  "Record.with replaces a nested-record field wholesale and the original keeps its inner"
  (doc
    "The `with` cases above replace SCALAR fields in const records; here the replaced field's value
           is itself a RECORD and the operands are runtime: `r1 = {x ↦ {p a, q 2}, y 5}` updated with
           `{p 30, q 40}` at `x` — the update swaps the WHOLE inner record (r2.x.p = 30, r2.x.q = 40),
           and the ORIGINAL r1 still reads its old inner (r1.x.p = a = 7) — persistence through the
           nesting (an in-place inner mutation or a shared-cell update would corrupt r1). Encodes
           1000·r2.x.p + 100·r1.x.p + 10·r2.y + r2.x.q = 30790 at a=7. Expected: 30790.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((inner1 #record((= p a) (= q 2))) (inner2 #record((= p 30) (= q 40))))
          (let
            ((r1 #record((= x inner1) (= y 5))))
            (let
              ((r2 (Record.with r1 #"x" inner2)))
              (+ (* 1000 r2.x.p) (+ (* 100 r1.x.p) (+ (* 10 r2.y) r2.x.q)))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 30790 Int64))
  (live-objects 0))

(case
  "Record.with grows a LIST field by pushing onto the projected old value"
  (doc
    "The collection-field update idiom: `(Record.with r #\"items\" (List.push (. r items) a))` — the
           new field value is BUILT FROM the projection of the old (push onto the existing list), the
           read-modify-write a stateful record accumulates by. The updated record's list has 3 elements
           while the ORIGINAL record's field still has 2 (persistence of the record AND the shared list
           handle: the push must not mutate the field in place). 3·10 + 2 = 32.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((r #record((= items #list(1 2)) (= tag 7))))
          (let
            ((r2 (Record.with r #"items" (List.push r.items a))))
            (+ (* 10 (List.len r2.items)) (List.len r.items)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 32 Int64))
  (live-objects known-leak))

(case
  "chained Record.with updates on one field compose with the last write winning"
  (doc
    "`(Record.with (Record.with r #\"x\" 10) #\"x\" 20)` — two updates of the SAME field chained:
           the outer sees the inner's result, so the last write wins (r2.x = 20) while the ORIGINAL
           binding keeps its runtime value (r.x = a = 7). 10·r2.x + r.x = 207. Pins that `with` composes
           through its own result (each update derives a fresh record from the previous — a rewrite that
           batched or reordered same-field updates would still get 20, but one that updated the ORIGINAL
           twice independently would lose the chain). Expected: 207.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((r #record((= x a) (= y 2))))
          (let ((r2 (Record.with (Record.with r #"x" 10) #"x" 20))) (+ (* 10 r2.x) r.x))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 207 Int64)))

(case
  "a nested Record.with rewrites an inner record field through the outer, inline"
  (doc
    "The write-through composite: `(Record.with p0 #\"pos\" (Record.with (. p0 pos) #\"y\" d))` — the
           inner `with` derives a fresh POS record from the projected one, and the outer `with` seats
           it back into a fresh OUTER record. Both levels are functional: p1.pos.y = d (5) while
           p1.pos.x rides through unchanged (1) → 51. The chained-with case above composes on ONE
           record level; this nests the derivation through a record-valued FIELD — the update shape a
           position/velocity struct takes. (NB the SAME composite through a nested-record-annotated
           function PARAMETER currently declines — the inline form pinned here is the working face
           that must not regress while that surface lands.)")
  (input
    (do
      (def
        (main (: d Int64))
        (do
          (def p0 #record((= pos #record((= x 1) (= y 2))) (= vel #record((= x 30) (= y 40)))))
          (def p1 (Record.with p0 #"pos" (Record.with p0.pos #"y" d)))
          (+ (* p1.pos.y 10) p1.pos.x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 51 Int64))
  (live-objects known-leak))

(case
  "a nested-record parameter projects its inner fields through two dot levels"
  (doc
    "The read face of the nested-record param surface: a fn whose parameter is annotated
           `(Record (: pos (Record …)) (: vel (Record …)))` projects `(. (. outer pos) y)` through
           TWO dot levels of the parameter. The annotation must ground both record layers for the
           inner projection to resolve (2 + d = 7 at d=5). This is the PASSING half of the
           nested-param surface — the write-through half (Record.with through the same parameter)
           is a known decline; pinning the read guards the boundary from regressing further.")
  (input
    (do
      (def
        (gety
          (:
            outer
            (Record
              (: pos (Record (: x Int64) (: y Int64)))
              (: vel (Record (: x Int64) (: y Int64))))))
        outer.pos.y)
      (def
        (main (: d Int64))
        (do
          (def p0 #record((= pos #record((= x 1) (= y 2))) (= vel #record((= x 30) (= y 40)))))
          (+ (gety p0) d)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "chained Record.with at TWO DIFFERENT fields holds both updates"
  (doc
    "The cross-field chain (the one-field chain above pins last-write-wins): `(Record.with
           (Record.with r #\"x\" a) #\"y\" b)` updates x and y with runtime values while z rides through untouched
           — 100a + 10b + 3 = 453. A chain that rebuilt from the ORIGINAL record on the second with
           (rather than the first with's result) would lose the x update.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        (let
          ((r (Record.with (Record.with #record((= x 1) (= y 2) (= z 3)) #"x" a) #"y" b)))
          (+ (* 100 r.x) (+ (* 10 r.y) r.z))))
      (export main)))
  (call main (: 4 Int64) (: 5 Int64))
  (output (: 453 Int64)))

(case
  "the pre-update record survives a CONDITIONAL Record.with in a branch join"
  (doc
    "Persistence through a runtime branch: `(if b (Record.with r #\"x\" 99) r)` — reading BOTH `r` and
           the join result after: b=true → r still 10, r2 is 99 (1099); b=false → both 10 (1010). An
           in-place update would clobber `r` on the true path (reading 9999); a join that copied the
           un-updated record into both slots would read 1010 on both.")
  (input
    (do
      (def
        (main (: b Bool))
        (let
          ((r #record((= x 10) (= y 20))))
          (let ((r2 (if b (Record.with r #"x" 99) r))) (+ (* 100 r.x) r2.x))))
      (export main)))
  (call main (: true Bool))
  (output (: 1099 Int64))
  (call main (: false Bool))
  (output (: 1010 Int64))
  (live-objects 0))

(case
  "the OLD 2-operand `Record.with (name value)` pair form is rejected (migrated to 3 operands)"
  (doc
    "Witnesses DESIGN-record-update-syntax.md §2/§6 (operator DECISION 2026-07-15): the field-pair
           row ops now take THREE positional operands `r #field value`, and the OLD grouped `(name value)`
           pair spelling is MIGRATED + REJECTED, never kept as a second accepted spelling
           (canonical-form discipline — the `(price 9)` pair rendered call-like `price(9)` in ML). A
           2-operand `Record.with` is now an ARITY error (CDZ0201) whose message routes the migration
           (`… now takes three operands r #b v — replace the (name value) pair with a #field label and a
           value`). This negative case PINS the rejection so a future change can't silently re-accept the
           old form and reintroduce the call-like render.")
  (input (Record.with #record((= a 1) (= b 2)) (b 9)))
  (error CDZ0201))

(case
  "the OLD 2-operand `Record.extend (name value)` pair form is rejected (migrated to 3 operands)"
  (doc
    "Witnesses DESIGN-record-update-syntax.md §1/§6: `Record.extend` gets the same 3-operand
           treatment as `Record.with` for family uniformity, so its OLD grouped `(name value)` pair is
           also migrated + rejected as an arity error (CDZ0201) with the same migration-routing message.
           Pins the rejection alongside the `with` sibling.")
  (input (Record.extend #record((= a 1)) (b 2)))
  (error CDZ0201))

(case
  "extending a record with a non-#label (bare identifier) field-name operand is rejected"
  (doc
    "The field-name operand of `Record.extend`/`Record.with` is a `#field` LABEL (a static label, NOT
           a runtime value — this case above, 'extending a record adds a new field'). A BARE identifier in
           that position — `(Record.extend (record (x 10)) fname k)`, where `fname` is neither a `#label` nor
           a declared binding — used to be silently PUNNED into a field literally named `fname` (the
           reinterpret-instead-of-reject footgun: a user passing a computed Symbol expecting dynamic field
           naming got a field named after their variable). The fix (v-inference, CDZ0215
           `RecordFieldNameNotLabel`) REJECTS a non-`#label` name-introduction operand with a coded
           diagnostic naming the static-label rule. Scoped to the name-INTRODUCTION operand of extend/with;
           the read/drop ops `(. r x)`/`pop`/`without`/`project` legitimately take a bare label and stay
           valid.")
  (input
    (do
      (def (main (: k Int64)) (let ((wide (Record.extend #record((= x 10)) fname k))) wide.fname))
      (export main)))
  (call main (: 7 Int64))
  (error CDZ0215))

(case
  "popping a field yields its value and the remaining record"
  (doc
    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields and #A Field
           Is Added To Or Replaced In A Record By A Derived Operation: `Record.pop` takes a field OFF a
           record, defined as `(tuple (. r z) (Record.without r (z)))` — the field's value paired with the
           record of the remaining fields. `(Record.pop (record (a 1) (b 2)) a)` yields `(tuple 1 (record
           (b 2)))`. No Option: field presence is static, so a missing field is CDZ0212, not a runtime None
           (contrast `List.at` on a runtime index).")
  (input (Record.pop #record((= a 1) (= b 2)) a))
  (output (: #tuple(1 #record((= b 2))) (Tuple Int64 (Record (: b Int64))))))

(case
  "popping an absent field is rejected"
  (doc
    "Witnesses type-system.md #A Record Is Reduced By Dropping A Named Set Of Its Fields (2nd
           sentence), via `Record.pop`'s `Record.without` rewrite: popping a field the record does not
           contain is a compile-time rejection (CDZ0212), not a runtime None — a record field name is a
           static label, not a runtime index. `z` is absent from `(record (a 1))`.")
  (input (Record.pop #record((= a 1)) z))
  (error CDZ0212))

; The DERIVED row ops carry the same two-tier absent-field did-you-mean the `Record.without`/dot-access
; labels get. A near-miss field on `Record.pop` (`alpa` for `alpha`) is CDZ0212 naming the pop context AND
; a confident "did you mean `alpha`?" with a heuristic replace fix on the label. `Record.with` adds a
; distinguishing hint — a near-miss update field ALSO names the extend-vs-update distinction ("use
; `Record.extend` to add"), since the likeliest other intent for an absent field is to ADD it. (Migrated
; from rcdzc record_extend_with_pop_are_derived_row_ops_with_presence_checks's did-you-mean faces; the
; presence-check faces — extend present→CDZ0211/absent→runs, with present→runs/absent→CDZ0212, pop
; present→runs/absent→CDZ0212 — are the extend/with/pop cases already in this chapter.)
(case
  "a near-miss field in Record.pop suggests the near field with a replace fix"
  (input (do (def (main) (Record.pop #record((= alpha 1) (= beta 2)) alpa)) (export main)))
  (error
    CDZ0212
    (message "did you mean `alpha`?")
    (fix (kind replace) (replacement "alpha") (unverified))))

(case
  "a near-miss field in Record.with suggests the near field and keeps the extend hint"
  (input (do (def (main) (Record.with #record((= alpha 1) (= beta 2)) #"alpa" 9)) (export main)))
  (error
    CDZ0212
    (message "did you mean `alpha`?")
    (message "use `Record.extend`")
    (fix (kind replace) (replacement "alpha") (unverified))))

(case
  "record reshaping is subset comparison as explicit projection"
  (doc
    "Witnesses type-system.md #Records Are Rows (4th sentence: subset comparison is explicit
           projection-then-`=`, never an overloaded `=`) with `Record.project` as the narrowing operation.
           `(= (Record.project (record (x 1) (y 2)) (x)) (record (x 1)))` projects the shared field to a
           closed one-field record and compares it by ordinary structural equality — true. The
           general-projection form of the plain-`.` subset-comparison case above; `=` is never widened to
           ignore `y`, `Record.project` narrows the shape first.")
  (input (= (Record.project #record((= x 1) (= y 2)) (x)) #record((= x 1))))
  (output (: true Bool)))

; The cases above pin each record operation in isolation (extend, without, with, merge, project). These pin
; their ALGEBRAIC compositions — the round-trips and inverses where a field-set bookkeeping slip would
; surface: extend-then-without is the identity, `with` preserves the OTHER fields, and merge-then-project
; recovers a merged side. Each composes ≥2 operations, so a result that mis-tracked the field set (dropped,
; duplicated, or reordered a label) fails the structural `=`.
(case
  "extending a record then dropping the added field returns the original"
  (doc
    "`(Record.without (Record.extend (record (a 1)) #\"b\" 2) (b))` = `(record (a 1))` — extend adds
           `b`, without drops it, and the result equals the original by structural `=`. Pins that
           extend/without are inverse on the added field: the field-set bookkeeping adds then removes exactly
           `b`, leaving `a` untouched.")
  (input (= (Record.without (Record.extend #record((= a 1)) #"b" 2) (b)) #record((= a 1))))
  (output (: true Bool)))

(case
  "updating a record field preserves the other fields' values"
  (doc
    "`(Record.with (record (a 1) (b 2) (c 3)) #\"b\" 9)` = `(record (a 1) (b 9) (c 3))` — `with`
           replaces only `b`, leaving `a` and `c` at their original values. Pins that an update is local to
           the named field: the surrounding fields (both before and after the updated one) keep their values
           and positions, not just the updated field being correct.")
  (input (= (Record.with #record((= a 1) (= b 2) (= c 3)) #"b" 9) #record((= a 1) (= b 9) (= c 3))))
  (output (: true Bool)))

(case
  "merging two disjoint records then projecting one side recovers it"
  (doc
    "`(Record.project (Record.merge (record (a 1) (b 2)) (record (c 3))) (a b))` = `(record (a 1)
           (b 2))` — merge unions the disjoint fields, then project narrows back to the left side's labels,
           recovering it exactly. Pins the merge/project round-trip: the merged record carries all three
           fields with their values, and projecting `(a b)` selects the two by name unchanged.")
  (input
    (=
      (Record.project (Record.merge #record((= a 1) (= b 2)) #record((= c 3))) (a b))
      #record((= a 1) (= b 2))))
  (output (: true Bool)))

; --- Tuple reshaping: explicit positional operations yield a new tuple ----------------------
; type-system.md #A Tuple Is Reshaped Positionally By An Explicit Operation Yielding A New Value and its
; companions: `Tuple.concat` concatenates, `Tuple.split-at` splits at a static position, `Tuple.remove` takes
; element 0 off. A tuple's arity is part of its type, so every result arity is fixed statically and there
; is no disjointness constraint (positions are anonymous). `k` is a compile-time position written as a
; literal, exactly as `(. x N)` writes its index; a split outside `0..=len` is a type error, the `(. x N)`
; static-bounds rule (05-compound-types "tuple elements are accessed by index"). These ride
; the same later-generation rows layer and `Tuple.*` is an unbound name to the seed, so it declines them.
(case
  "concatenating two tuples appends their elements"
  (doc
    "Witnesses type-system.md #Two Tuples Are Concatenated Into One Of Their Combined Length:
           `(Tuple.concat (tuple 1 2) (tuple 3 4))` yields `(tuple 1 2 3 4)` of arity 4 — the first tuple's
           elements in order followed by the second's, each keeping its source position's type.")
  (input (Tuple.concat #tuple(1 2) #tuple(3 4)))
  (output (: #tuple(1 2 3 4) (Tuple Int64 Int64 Int64 Int64))))

(case
  "concatenating tuples preserves each element's type"
  (doc
    "The heterogeneous companion: `(Tuple.concat (tuple 1 true) (tuple \"x\"))` yields `(tuple 1 true
           \"x\")` of type `(Tuple Int64 Bool String)`. Pins that concatenation keeps the type of each
           source position rather than unifying to one element type — a tuple is a heterogeneous product,
           unlike a homogeneous list.")
  (input (Tuple.concat #tuple(1 true) #tuple("x")))
  (output (: #tuple(1 true "x") (Tuple Int64 Bool String))))

; The concatenation cases above build both operand tuples from CONSTANT literals, so the result folds to a
; constant tuple at compile time. A tuple carrying a RUNTIME element — a boundary parameter — cannot fold:
; the concatenation runs on the value heap, and reading an element back exercises the emitted machinery. A
; case reads the result down to a SCALAR (a projection then arithmetic) so it returns from a parameterized
; export. These pin `Tuple.concat` on a runtime operand — the value companion of the constant cases.
(case
  "concatenating tuples with a runtime element reads elements from both operands"
  (doc
    "`(Tuple.concat (tuple n 2) (tuple 3 4))` with `n` a boundary parameter cannot fold — the first
           element is decided at run time, so the concatenation runs on the value heap. Reading element 0
           (the runtime `n`, from the first operand) and element 3 (4, from the second) and summing them
           yields `n + 4`: 7+4 = 11. Pins that a runtime `Tuple.concat` places BOTH operands' elements into
           the result at their combined positions, read back correctly by projection.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (. (Tuple.concat #tuple(n 2) #tuple(3 4)) 0)
          (. (Tuple.concat #tuple(n 2) #tuple(3 4)) 3)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 11 Int64))
  (call main (: 100 Int64))
  (output (: 104 Int64)))

(case
  "runtime tuple concatenation preserves element order across the seam"
  (doc
    "`(. (Tuple.concat (tuple 1 2) (tuple n 4)) 2)` reads position 2 of the concatenation — the FIRST
           element of the second operand (`n`) — which lands just past the first operand's two elements. It
           is `n` for every `n` (99 → 99). Pins that the second operand's elements are appended AFTER the
           first's on the runtime path, so position 2 is the second tuple's element 0, not a first-operand
           element or a shifted slot.")
  (input (do (def (main (: n Int64)) (. (Tuple.concat #tuple(1 2) #tuple(n 4)) 2)) (export main)))
  (call main (: 99 Int64))
  (output (: 99 Int64))
  (call main (: -7 Int64))
  (output (: -7 Int64)))

(case
  "concatenating an empty tuple on the left is the identity"
  (doc
    "The empty tuple `(tuple)` — which IS the unit value (core-semantics.md #The Empty Tuple Is The
           Unit Value) — is the identity of `Tuple.concat`: `(Tuple.concat (tuple) (tuple 1 2))` prepends no
           elements, so the result is `(tuple 1 2)`. Pins the empty-operand identity the existing cat cases
           (which join two non-empty tuples) do not exercise — the tuple companion of the empty-string /
           empty-bytes concatenation-identity cases.")
  (input (Tuple.concat #tuple() #tuple(1 2)))
  (output (: #tuple(1 2) (Tuple Int64 Int64))))

(case
  "concatenating an empty tuple on the right is the identity"
  (doc
    "The mirror: `(Tuple.concat (tuple 1 2) (tuple))` appends no elements, so the result is `(tuple 1
           2)`. Pins that the empty tuple is the identity on the right as well as the left, so a cat with an
           empty operand on either side is a no-op on value.")
  (input (Tuple.concat #tuple(1 2) #tuple()))
  (output (: #tuple(1 2) (Tuple Int64 Int64))))

(case
  "concatenating two empty tuples is the empty tuple"
  (doc
    "The degenerate boundary: `(Tuple.concat (tuple) (tuple))` joins nothing to nothing, yielding the
           empty tuple `(tuple)` — the unit value. Pins that cat handles the zero+zero case, not
           underflowing or producing a novel form, the tuple companion of the empty+empty string/bytes/set
           cases.")
  (input (Tuple.concat #tuple() #tuple()))
  (output (: #tuple() (Tuple))))

(case
  "splitting a tuple at a position yields a prefix and a suffix"
  (doc
    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix:
           `(Tuple.split-at (tuple 1 2 3) 1)` splits at position 1 into a pair — the first element as a
           1-tuple prefix and the rest as a 2-tuple suffix — yielding `(tuple (tuple 1) (tuple 2 3))`. The
           position `k` is a compile-time literal.")
  (input (Tuple.split-at #tuple(1 2 3) 1))
  (output (: #tuple(#tuple(1) #tuple(2 3)) (Tuple (Tuple Int64) (Tuple Int64 Int64)))))

(case
  "splitting a tuple at zero yields an empty prefix"
  (doc
    "The degenerate boundary of #A Tuple Is Split At A Position Into A Prefix And A Suffix: a split at
           position 0 puts no elements before it, so the prefix is the empty tuple — which IS the unit value
           (core-semantics.md #The Empty Tuple Is The Unit Value: `unit` and `()` are the same value) — and
           the suffix is the whole tuple. `(Tuple.split-at (tuple 1 2) 0)` yields `(tuple unit (tuple 1 2))`,
           the prefix typed `Unit`. Pins that 0 is in range and the empty prefix is the unit value, not a
           novel zero-arity tuple form.")
  (input (Tuple.split-at #tuple(1 2) 0))
  (output (: #tuple(unit #tuple(1 2)) (Tuple Unit (Tuple Int64 Int64)))))

(case
  "splitting a tuple at its full arity yields an empty suffix"
  (doc
    "The symmetric boundary of the split-at-zero case: a split at position `k` = the tuple's ARITY
           puts every element before it, so the prefix is the whole tuple and the SUFFIX is the empty tuple
           — the unit value (core-semantics.md #The Empty Tuple Is The Unit Value). `(Tuple.split-at (tuple
           1 2) 2)` yields `(tuple (tuple 1 2) unit)`, the suffix typed `Unit`. Pins that `k` = arity is in
           range (the split point may sit just past the last element) and the empty suffix is unit — the
           k=arity end of the k=0/k=arity boundary the split-at-zero case pins at the other end.")
  (input (Tuple.split-at #tuple(1 2) 2))
  (output (: #tuple(#tuple(1 2) unit) (Tuple (Tuple Int64 Int64) Unit))))

(case
  "splitting a tuple beyond its arity is rejected"
  (doc
    "Witnesses type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix (2nd
           sentence): a split position outside the operand's static arity range is a type error (CDZ0201),
           consistent with an out-of-arity positional access `(. x N)` being rejected. `(tuple 1 2)` has
           arity 2, so a split at 5 names a position it does not have — rejected rather than producing a
           short suffix.")
  (input (Tuple.split-at #tuple(1 2) 5))
  (error CDZ0201))

(case
  "accessing through an empty-side split-at is usable, like the equivalent literal"
  (doc
    "The empty-prefix split `(Tuple.split-at (tuple 10 20) 0)` yields `(tuple unit (tuple 10 20))` —
           the SAME type and value as the hand-written literal, which is directly accessible. Reading
           through the result — the suffix `.1` then its element 0 — gives 10, matching what
           `(. (. (tuple unit (tuple 10 20)) 1) 0)` gives. The empty side is a `Unit` element; the
           projection through it FOLDS through the constant tuple the operation produced (no runtime
           value-heap build), so a split-at at the k=0 / k=arity boundary is usable, not just renderable.
           Pins that the empty-side result reaches the same representation the byte-identical literal does.")
  (input (do (def (main) (. (. (Tuple.split-at #tuple(10 20) 0) 1) 0)) (export main)))
  (output (: 10 Int64)))

(case
  "splitting a tuple with a runtime element addresses the prefix and suffix"
  (doc
    "`(Tuple.split-at (tuple n 20 30) 1)` with `n` a boundary parameter splits at position 1 (a
           compile-time literal) into a 1-tuple prefix `(tuple n)` and a 2-tuple suffix `(tuple 20 30)`,
           built on the value heap because `n` is runtime. Reading the prefix's element 0 (`n`) plus the
           suffix's element 1 (30) gives `n + 30`: 5+30 = 35. Pins that a runtime `Tuple.split-at` places
           the operand's runtime element into the correct side and position, read back by nested
           projection — the split boundary is the static `k` regardless of the element values.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (. (. (Tuple.split-at #tuple(n 20 30) 1) 0) 0)
          (. (. (Tuple.split-at #tuple(n 20 30) 1) 1) 1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 35 Int64))
  (call main (: 0 Int64))
  (output (: 30 Int64)))

(case
  "popping a tuple yields element zero and the remaining tuple"
  (doc
    "Witnesses type-system.md #A Tuple Is Reshaped Positionally: `Tuple.remove` takes element 0 off,
           `(tuple (. t 0) <rest>)` — the positional analogue of `Record.pop`. `(Tuple.remove (tuple 1 2
           3))` yields `(tuple 1 (tuple 2 3))`. It is `(Tuple.split-at t 1)` with the singleton prefix
           unwrapped to its element.")
  (input (Tuple.remove #tuple(1 2 3)))
  (output (: #tuple(1 #tuple(2 3)) (Tuple Int64 (Tuple Int64 Int64)))))

(case
  "popping a tuple with a runtime element separates the head from the rest"
  (doc
    "The runtime companion: `(Tuple.remove (tuple n 20 30))` with `n` a boundary parameter splits the
           head element 0 (`n`) from the rest `(tuple 20 30)`, built on the value heap because `n` is
           runtime. Reading the popped head (`.0` = `n`) and the rest's last element (`.1 .1` = 30) and
           summing gives `n + 30`: 9+30 = 39. Pins that a runtime `Tuple.remove` places the operand's element
           0 as the head and the remaining elements as the rest tuple, both read back by projection.")
  (input
    (do
      (def
        (main (: n Int64))
        (+ (. (Tuple.remove #tuple(n 20 30)) 0) (. (. (Tuple.remove #tuple(n 20 30)) 1) 1)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 39 Int64))
  (call main (: 0 Int64))
  (output (: 30 Int64)))

; A tuple reshape op over a DEFINITE non-tuple operand is a KIND error, the tuple twin of "member access
; requires a record, found Int64" (05-compound-types). `(Tuple.concat n …)`/`(Tuple.remove n)`/`(Tuple.split-at
; n 1)` for `n : Int64` names the op AND the non-tuple type it got (CDZ0201, "`Tuple.<op>` requires a tuple,
; found Int64") — not the dead-end "over a runtime tuple is not yet built" it once compiled to. (An
; unconstrained `Any` parameter is NOT rejected — its projection/reshape check defers to the call site,
; pinned by the bare-parameter helper cases in 09-functions.) (Migrated from rcdzc
; a_tuple_row_op_over_a_non_tuple_names_the_kind.)
(case
  "Tuple.concat over a non-tuple operand names the op and the non-tuple type"
  (input (do (def (g (: n Int64)) (Tuple.concat n n)) (export g)))
  (error CDZ0201 (message "`Tuple.concat` requires a tuple") (message "Int64")))

(case
  "Tuple.remove over a non-tuple operand names the op and the non-tuple type"
  (input (do (def (g (: n Int64)) (Tuple.remove n)) (export g)))
  (error CDZ0201 (message "`Tuple.remove` requires a tuple") (message "Int64")))

(case
  "Tuple.split-at over a non-tuple operand names the op and the non-tuple type"
  (input (do (def (g (: n Int64)) (Tuple.split-at n 1)) (export g)))
  (error CDZ0201 (message "`Tuple.split-at` requires a tuple") (message "Int64")))

; --- Tuple arity: `Tuple.size` reports a tuple's element count -------------------------------
; type-system.md #The Arity Of A Tuple Positional Operation's Result Must Be Determined Statically: a
; tuple carries no runtime length — its arity is part of its TYPE. `Tuple.size t` reports that arity as
; an `Int64`, and because the arity is static the result is a compile-time constant even when the tuple
; holds RUNTIME elements. It is the tuple companion of `List.len` (a homogeneous list has a runtime
; spine length; a heterogeneous tuple has a static one). A non-tuple operand is the same CDZ0201 kind
; error the other `Tuple.*` ops give.
(case
  "the size of a heterogeneous tuple is its element count"
  (doc
    "Witnesses #The Arity Of A Tuple Positional Operation's Result Must Be Determined Statically:
           `(Tuple.size (tuple 1 \"a\" true))` is `3` — the number of positions — regardless of the
           elements' types. Pins that a tuple's arity is observable as an `Int64`.")
  (input (Tuple.size #tuple(1 "a" true)))
  (output (: 3 Int64)))

(case
  "the size of the empty tuple is zero"
  (doc
    "The degenerate boundary: the empty tuple `(tuple)` — which IS the unit value (core-semantics.md
           #The Empty Tuple Is The Unit Value) — has arity 0, so `(Tuple.size (tuple))` is `0`. Pins that
           size handles the zero case rather than underflowing.")
  (input (Tuple.size #tuple()))
  (output (: 0 Int64)))

(case
  "a tuple's size is static even when it holds a runtime element"
  (doc
    "The arity is a property of the TYPE, not the values, so `(Tuple.size (tuple n 2 3 4 5))` with `n`
           a boundary parameter is `5` for every `n` — it folds to the constant arity without evaluating
           the tuple. Pins that a runtime element does not defeat the static-arity read (the tuple
           companion of `List.len` on a constant-spine list), for two different `n`.")
  (input (do (def (main (: n Int64)) (Tuple.size #tuple(n 2 3 4 5))) (export main)))
  (call main (: 7 Int64))
  (output (: 5 Int64))
  (call main (: -100 Int64))
  (output (: 5 Int64)))

(case
  "Tuple.size over a non-tuple operand names the op and the non-tuple type"
  (input (do (def (g (: n Int64)) (Tuple.size n)) (export g)))
  (error CDZ0201 (message "`Tuple.size` requires a tuple") (message "Int64")))

(case
  "a match on an open sum with an open-tail arm is exhaustive"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: an open sum
           is DECLARED with a trailing `.. r` row-variable marker (`(type Vocab (Known Unit) (Unknown
           Unit) .. r)`), which stands for variants the module does not name. A match covering a named
           variant plus an open-tail `_` arm is exhaustive and handles every unnamed variant as data, so
           it yields \"known\" for the `Known` value.")
  (input
    (do
      (type Vocab (Known Unit) (Unknown Unit) .. r)
      (def (name-of (: e Vocab)) (match e ((Known _) "known") (_ "other")))
      (def (main) (name-of (Known unit)))
      (export main)))
  (output (: "known" String)))

(case
  "an open sum's open-tail arm dispatches a variant the specific arms do not name"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm (the open-tail
           arm handles the unnamed variants as data): the open-tail `_` arm is not just an
           exhaustiveness formality — it actually DISPATCHES. A `Vocab` value that is NOT the specific
           `Known` arm falls through to `_`, so `(name-of (Unknown unit))` yields \"other\". This pins the
           dispatch/fold path through the open tail, the runnable companion to the exhaustiveness verdict.")
  (input
    (do
      (type Vocab (Known Unit) (Unknown Unit) .. r)
      (def (name-of (: e Vocab)) (match e ((Known _) "known") (_ "other")))
      (def (main) (name-of (Unknown unit)))
      (export main)))
  (output (: "other" String)))

; The dispatch case above pins the open-tail `_` handles an uncovered variant AS DATA. These pin that the
; value BOUND by an open-tail arm carries its ACTUAL variant intact THROUGH a function call and a RE-MATCH:
; a value that falls to a `rest` binder, is forwarded to another function, and is matched there must still
; be the variant it was (the open-tail bind must not erase the discriminant or corrupt the payload — the
; row variable crosses the call boundary). This is the forwarding idiom a layered dispatcher uses (handle
; what I know, pass the rest on). Both the named-arm-hit and match-neither paths are checked.
(case
  "an open-sum value bound by an open-tail arm is forwarded to another matcher with its variant intact"
  (doc
    "`classify` matches `(A n)` and forwards everything else via a `rest` binder to `extract`, which
           matches `(B m)`. `(B 42)` falls to `classify`'s open-tail `rest`, crosses the call to `extract`,
           and matches `(B 42)` there → 42. Pins that the open-tail bind preserves the actual variant and
           payload across a function forward + re-match — a layered-dispatcher idiom; a bind that erased the
           discriminant or dropped the payload would lose the `B 42` and mis-dispatch in `extract`.")
  (input
    (do
      (type V (A Int64) (B Int64) .. r)
      (def (extract (: v V)) (match v ((B m) m) (_ -1)))
      (def (classify (: v V)) (match v ((A n) n) (rest (extract rest))))
      (def (main) (classify (B 42)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a named arm still dispatches directly rather than forwarding through the open tail"
  (doc
    "The named-hit companion: `(A 7)` matches `classify`'s OWN `(A n)` arm → 7, never reaching the
           `rest` forward. Pins that forwarding is only for the open-tail fall-through; a value the current
           matcher names is handled locally, not passed on.")
  (input
    (do
      (type V (A Int64) (B Int64) .. r)
      (def (extract (: v V)) (match v ((B m) m) (_ -1)))
      (def (classify (: v V)) (match v ((A n) n) (rest (extract rest))))
      (def (main) (classify (A 7)))
      (export main)))
  (output (: 7 Int64)))

(case
  "an open-sum value matching no named arm in either matcher reaches the forwarded sentinel"
  (doc
    "`(C 99)` matches neither `classify`'s `(A n)` nor (after forwarding) `extract`'s `(B m)`, so it
           reaches `extract`'s own open-tail `_` → -1. Pins the forward composes across TWO layers of
           open-tail fall-through — the value crosses both matchers as data and lands on the final sentinel,
           its variant never named but never lost.")
  (input
    (do
      (type V (A Int64) (B Int64) (C Int64) .. r)
      (def (extract (: v V)) (match v ((B m) m) (_ -1)))
      (def (classify (: v V)) (match v ((A n) n) (rest (extract rest))))
      (def (main) (classify (C 99)))
      (export main)))
  (output (: -1 Int64)))

(case
  "a match on an open sum omitting the open-tail arm is rejected"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open (a match that omits the open-tail arm is a
           compile-time rejection): because an open sum's variant set is not closed, a match covering
           every NAMED variant but omitting the open-tail `_` arm still cannot be exhaustive — the row
           variable stands for variants it cannot enumerate — so it is rejected (CDZ0210) rather than
           run. A closed sum with the same two arms WOULD be exhaustive; the open declaration is what
           mandates the `_`.")
  (input
    (do
      (type Vocab (Known Unit) (Unknown Unit) .. r)
      (def (name-of (: e Vocab)) (match e ((Known _) "known") ((Unknown _) "unknown")))
      (def (main) (name-of (Unknown unit)))
      (export main)))
  (error CDZ0210))

(case
  "a SINGLE-variant open sum still requires an open-tail arm"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm: a
           single-named-variant CLOSED sum erases to a newtype whose sole constructor pattern is
           irrefutable (no `_` needed). But the SAME sum declared OPEN (`(type Box (Wrap Int64) .. r)`)
           is NOT a newtype — the row variable means a value's variant is not statically `Wrap`, so a
           match covering only `(Wrap n)` without a `_` arm is non-exhaustive (CDZ0210). Pins that
           open-ness suppresses the single-variant newtype erasure for exhaustiveness.")
  (input
    (do
      (type Box (Wrap Int64) .. r)
      (def (unwrap (: b Box)) (match b ((Wrap n) n)))
      (def (main) (unwrap (Wrap 42)))
      (export main)))
  (error CDZ0210))

(case
  "a single-variant open sum with an open-tail arm dispatches its named variant"
  (doc
    "The runnable companion: the SAME single-variant open sum `(type Box (Wrap Int64) .. r)`, now
           WITH the open-tail `_` arm, is exhaustive and dispatches the named `Wrap` variant to its
           payload — `(unwrap (Wrap 42))` yields 42. Pins that the newtype-erasure suppression (which
           keeps the value a boxed sum) does not break the named variant's own payload read.")
  (input
    (do
      (type Box (Wrap Int64) .. r)
      (def (unwrap (: b Box)) (match b ((Wrap n) n) (_ 0)))
      (def (main) (unwrap (Wrap 42)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a CLOSED single-variant sum's sole-constructor arm is exhaustive without a `_` (the newtype-erasure control)"
  (doc
    "The closed-sum CONTROL that isolates open-ness as the cause of the open case's CDZ0210 above: the
           SAME `(Wrap n)` sole-constructor arm over a CLOSED `(type Box (Wrap Int64))` (no `.. r`) IS
           exhaustive with NO `_` arm — a single-variant closed sum erases to a newtype whose sole
           constructor pattern is irrefutable (type-system.md #A Sum Type May Be Open …). Pins that ONLY the
           open declaration mandates the open-tail arm; without the row variable the newtype erasure makes the
           match exhaustive, so it compiles clean and reads the `Wrap` payload → 42. Without this control the
           open case's CDZ0210 could be a spurious over-fire on ALL single-variant sums rather than open ones.")
  (input
    (do
      (type Box (Wrap Int64))
      (def (unwrap (: b Box)) (match b ((Wrap n) n)))
      (def (main) (unwrap (Wrap 42)))
      (export main)))
  (output (: 42 Int64)))

(case
  "an open sum's open-tail arm dispatches a NAMED-but-uncovered variant, not only unnamed ones"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm (the open-tail
           arm handles the variants not covered as data): the `_` arm covers not just the UNNAMED row-tail
           variants but also any NAMED variant the specific arms omit. `(type Vocab (A Int64) (B Int64)
           .. r)` matched with only an `A` arm plus `_` dispatches a `B` value through `_` → 99. Pins that
           the wildcard is a genuine catch-all over the whole uncovered set, named and unnamed alike.")
  (input
    (do
      (type Vocab (A Int64) (B Int64) .. r)
      (def (rd (: v Vocab)) (match v ((A n) n) (_ 99)))
      (def (main) (rd (B 3)))
      (export main)))
  (output (: 99 Int64)))

(case
  "a nested pattern under an open sum's named variant still requires the outer open-tail arm"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           #Patterns Compose: an open sum whose named variant carries a compound payload
           (`(type Vocab (Wrap (Option Int64)) .. r)`) is matched by NESTING into the payload
           (`(Wrap (Some n))` / `(Wrap (None))`), and the OUTER open level still needs its `_` arm — the
           row tail is uncovered by any `Wrap` pattern. With the `_` present the match is exhaustive and
           the nested `(Some 5)` payload reads through to 5.")
  (input
    (do
      (type Vocab (Wrap (Option Int64)) .. r)
      (def (rd (: v Vocab)) (match v ((Wrap (Some n)) n) ((Wrap (None)) 0) (_ -1)))
      (def (main) (rd (Wrap (Some 5))))
      (export main)))
  (output (: 5 Int64)))

(case
  "an open sum nested as another sum's payload matches with an inner wildcard"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           #Patterns Compose: an OPEN sum can be the payload of another (closed) sum — here an
           `(Option Inner)` where `Inner` is open. A match nesting into the `Some` payload
           (`(Some (A n))`) plus a `(Some _)` arm covering the open Inner's other/unnamed variants plus
           `(None)` is exhaustive: the outer `Option` is CLOSED (Some/None both covered), and the inner
           open `Inner` is covered by the `(Some _)` wildcard. `(rd (Some (B 5)))` falls through `(Some
           (A n))` to `(Some _)` → 0. Pins that an open sum composes as a generic sum's payload and its
           open tail is satisfied by an inner `_` at the nesting level.")
  (input
    (do
      (type Inner (A Int64) (B Int64) .. r)
      (def (rd (: o (Option Inner))) (match o ((Some (A n)) n) ((Some _) 0) ((None) -1)))
      (def (main) (rd (Some (B 5))))
      (export main)))
  (output (: 0 Int64)))

(case
  "an open sum with a guarded named arm plus an open-tail arm is exhaustive"
  (doc
    "Witnesses type-system.md #A Sum Type May Be Open, With A Mandatory Open-Tail Arm composed with
           the guarded-arm rule (a guarded arm covers no variant, so it never satisfies exhaustiveness on
           its own): an open sum matched by a GUARDED named arm `(guard (A n) (> n 0))` plus the open-tail
           `_` is exhaustive — the `_` covers both the guard's false-fallthrough and the open/unnamed
           variants. `(rd (A 7))` satisfies the guard (7 > 0) → 7. Pins that the open-tail arm composes
           with a guard exactly as it does for a closed sum's guarded arms.")
  (input
    (do
      (type V (A Int64) (B Int64) .. r)
      (def (rd (: v V)) (match v ((guard (A n) (> n 0)) n) (_ 0)))
      (def (main) (rd (A 7)))
      (export main)))
  (output (: 7 Int64)))

(case
  "OPEN-sum values as SET elements hash by tag and payload"
  (doc
    "The map-VALUE pin round-trips an open sum through a CHAMP slot without hashing it; a SET
           element must HASH it — tag AND payload: {A 10, B 7, A k} dedupes at k=10 (len 2 + contains
           A 10 → 21) and holds three at k=3 (31). A hash over only the payload collides A 10 with a
           hypothetical B 10; over only the tag it collapses A 10 / A 3 — either flips a row. The
           open-tail row var is instantiated but unused here: the hash must work on the CLOSED
           prefix representation the constructors produce.")
  (input
    (do
      (type Ev (A Int64) (B Int64) .. r)
      (def
        (main (: k Int64))
        (do
          (def s #set((Ev.A 10) (Ev.B 7) (Ev.A k)))
          (+ (* 10 (Set.len s)) (if (Set.contains s (Ev.A 10)) 1 0))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 21 Int64))
  (call main (: 3 Int64))
  (output (: 31 Int64)))

; --- CLOSURE fields through the row-op family: a fn value (handle + env cell) rides merge's
; union layout, pop's value-yielding removal, row-polymorphic access at two widths, and extend's
; slot growth. A row op that copied fn slots by scalar width, or re-sorted fields without moving
; the env pointer, calls the wrong target or reads a sibling field as the env.
(case
  "Record.merge carries a CLOSURE field into the union layout and it applies"
  (doc
    "The fn-field face of merge (the heap-field merge pin carries LISTS): the left record's
           `f` holds a k-capturing closure; merge unions it with a scalar record and the MERGED
           record's `f` applies (3+k → ×10), plus the merged `b`=7 → 87 at k=5, 37 at k=0. The union
           layout must place the fn handle + its env cell correctly among re-sorted fields — a merge
           that copied the fn slot by scalar width (or re-ordered fields without moving the env
           pointer) calls the wrong target or reads b as the env.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m (Record.merge #record((= f (fn ((: y Int64)) (+ y k)))) #record((= b 7))))
          (+ (* 10 (m.f 3)) m.b)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 87 Int64))
  (call main (: 0 Int64))
  (output (: 37 Int64))
  (live-objects known-leak))

(case
  "Record.pop hands back a CLOSURE field as the popped value and it applies"
  (doc
    "The fn-field face of the value-yielding removal: `(Record.pop r f)` returns
           `(tuple <popped-closure> <rest>)` — the popped slot IS the k-capturing fn, applied
           directly from the tuple projection (3k·10), while the rest-record's `b` survives the
           field removal (157 at k=5; k=0 zeroes the closure face → 7). A pop that returned the
           fn slot by scalar copy (dropping the env) or rebuilt the rest-record over the fn's
           slot corrupts one side. Completes the row-op × fn-field row: with/merge/project apply
           closures through records; pop EXTRACTS one.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def r #record((= f (fn ((: y Int64)) (* y k))) (= b 7)))
          (def p (Record.pop r f))
          (+ (* 10 ((. p 0) 3)) (. (. p 1) b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 157 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a row-polymorphic accessor applies a FN field across two record widths"
  (doc
    "Rows × fn values: `call-f r = ((. r f) 10)` reads a CLOSURE out of a record field through
           row polymorphism and applies it — once on the exact record (a k-capturing closure, 10+k)
           and once on a WIDER record with an extra field (a doubling closure, 20): 1520 at k=5,
           1020 at k=0. The row-poly pins project scalars/lists; a FN field composes the row access
           with call_indirect — a field offset computed against the narrow layout reads the wrong
           slot on the wide record and calls garbage (or the extra field).")
  (input
    (do
      (def (call-f r) (r.f 10))
      (def
        (main (: k Int64))
        (+
          (* 100 (call-f #record((= f (fn ((: y Int64)) (+ y k))))))
          (call-f #record((= f (fn ((: y Int64)) (* y 2))) (= extra 99)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1520 Int64))
  (call main (: 0 Int64))
  (output (: 1020 Int64)))

(case
  "Record.extend ADDS a closure field beside two existing ones and all three apply"
  (doc
    "The extend face of the fn-field row-op family (merge/pop/row-poly-access are the siblings): the vtable-style record GROWS by one fn slot, and the re-sorted 3-field layout must keep each fn handle paired with ITS env cell — a k-capturing add beside two capture-free fns, all three applied positionally (907 at k=3; any slot/env mix-up diverges).")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def ops #record((= add (fn ((: x Int64)) (+ x k))) (= dbl (fn ((: x Int64)) (* x 2)))))
          (def r2 (Record.extend ops #"neg" (fn ((: x Int64)) (- 0 x))))
          (+ (* 100 (r2.add 5)) (+ (* 10 (r2.dbl 5)) (r2.neg -7)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 907 Int64))
  (live-objects known-leak))

; --- Open sums through program structure: a three-module concrete chain and a generic tuple
; slot; Record.extend widening a map-extracted record into a new map. ---
(case
  "an open-sum type crosses THREE modules concretely and matches at every stop"
  (doc
    "Open-tail sum × the module chain: `types` declares the OPEN `(A Int64) (B Int64) .. r`
           and exports it CONCRETELY (`(export (. Ev *))` — the bare handle alone would be abstract,
           CDZ0214 on construction); `base` imports it and matches with an open-tail arm; `mid`
           re-exports the classifier; the ENTRY imports BOTH the type (to construct) and the
           chained classifier (to consume) — A routes ·10, B routes bare (50 at k=5, 7 at k=0).
           The open row's tail var survives two type-import hops and a re-export.")
  (input
    (do
      (import "types" (Ev))
      (import "mid" (classify))
      (def (main (: k Int64)) (classify (if (> k 0) (Ev.A k) (Ev.B 7))))
      (export main)))
  (module "types"
    (do (type Ev (A Int64) (B Int64) .. r) (export Ev) (export Ev.*)))
  (module "base"
    (do
      (import "types" (Ev))
      (def (classify v) (match v ((A n) (* n 10)) ((B n) n) (_ -1)))
      (export classify)))
  (module "mid"
    (do (import "base" (classify)) (export classify)))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "an open-sum value rides a generic tuple and matches from both slots"
  (doc
    "Open-tail sums × the generic container family: `(Ev.A k)` — a value of the OPEN type
           `(A Int64) (B Int64) .. r` — duplicates through the unannotated `dup`, and BOTH tuple
           projections match it (slot 0 by the named arm ·10, slot 1 by a fuller arm set ·2 →
           12k: 60 at k=5, 0 at k=0). The generic instantiates at an open-row type — a
           monomorphizer that closed the row at instantiation (dropping the tail var) would
           reject the open-tail `_` arm; one that boxed the tagged value as a scalar breaks a
           projection.")
  (input
    (do
      (type Ev (A Int64) (B Int64) .. r)
      (def (dup x) #tuple(x x))
      (def
        (main (: k Int64))
        (do
          (def p (dup (Ev.A k)))
          (+
            (* 10 (match (. p 0) ((A n) n) (_ -1)))
            (match (. p 1) ((A n) (* n 2)) ((B n) n) (_ -3)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 60 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "Record.extend widens a map-extracted record and the wide row enters a new map"
  (doc
    "The EXTEND (widening) leg completing the row-op extraction family: the record comes out
           of `{1 -> (x 10)}`, `(Record.extend r #\"y\" k)` ADDS the y field (row widened), the
           wide record keys a NEW map, and the ORIGINAL map's narrow record is untouched
           (100·y-from-wide + x-from-original: 510 at k=5, 10 at k=0). An extend that wrote the new
           field over the shared narrow layout (or re-typed the original's slot) breaks a read —
           with the without-leg pin this closes the widen/narrow round-trip through collections.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 #record((= x 10)))))
          (def r (Option.expect (Map.lookup m 1) "p"))
          (def wide (Record.extend r #"y" k))
          (def m2 #map((= 1 wide)))
          (+
            (* 100 (. (Option.expect (Map.lookup m2 1) "p") y))
            (. (Option.expect (Map.lookup m 1) "p") x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 510 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

; --- Record.without on a map-extracted record, both maps staying typed. ---
(case
  "Record.without strips a field on a map-extracted record and both maps stay typed"
  (doc
    "The WITHOUT leg of the extract-edit-reinsert family (with/pop are pinned): the record
           comes out of a map, `(Record.without r (y))` drops y (narrowing the row), and the SLIM
           record enters a NEW map at its narrower type while the ORIGINAL map's record keeps its y
           (100·x-from-slim + y-from-original: 1005 at k=5, 1000 at k=0). A without that left a
           stale y slot in the layout (or narrowed the ORIGINAL's type) breaks a read. (Note
           `without` takes a field-name LIST — `(y)` — the CDZ0201 for a bare name teaches the
           form.)")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def m #map((= 1 #record((= x 10) (= y k)))))
          (def r (Option.expect (Map.lookup m 1) "p"))
          (def slim (Record.without r (y)))
          (def m2 #map((= 1 slim)))
          (+
            (* 100 (. (Option.expect (Map.lookup m2 1) "p") x))
            (. (Option.expect (Map.lookup m 1) "p") y))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1005 Int64))
  (call main (: 0 Int64))
  (output (: 1000 Int64))
  (live-objects known-leak))

; --- Open-row projection over COLLECTION-borne records. ---
(case
  "an open-row projection reads list-element records in a fold AND a wider record at another site"
  (doc
    "The open-row instantiation pins use DIRECT literal args; this projects records pulled OUT OF A COLLECTION — get-x applied to (List (Record (: x Int64) (: t Int64))) elements inside a recursive fold (match-bound heap elements, not literals) plus a third 3-field width at a direct site keeping the def polymorphic. A per-def single-layout specialization or literal-only projection misreads.")
  (input
    (do
      (def (get-x r) r.x)
      (def
        (sum-xs (: rs (List (Record (: x Int64) (: t Int64)))) (: i Int64) (: acc Int64))
        (match
          (List.at rs i)
          ((Option.Some r) (sum-xs rs (+ i 1) (+ acc (get-x r))))
          ((Option.None _u) acc)))
      (def
        (main (: n Int64))
        (+
          (sum-xs #list(#record((= x n) (= t 1)) #record((= x 20) (= t 2))) 0 0)
          (get-x #record((= x 5) (= y 6) (= z 7)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 28 Int64))
  (live-objects known-leak))

; --- Construction-path equality for row-op results with RUNTIME leaves (the const twins
; above fold before emit; these run the heap path-copies). ---
(case
  "without- and with-REACHED records with runtime leaves equal the directly-built record"
  (doc
    "The runtime-leaf face of row-op result canonicalization (the const twins fold before emit):
           `(Record.without {a:n, b:2} (b))` — a genuine heap path-copy dropping a field — must equal
           the directly-built {a:n} (tens digit), and `(Record.with {a:1, b:n} a 9)` must equal
           {a:9, b:n} (ones digit) → 11. A row-op that left the dropped field's slot behind (or copied
           the updated record onto a different field layout than direct construction) breaks a leg.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (* 10 (if (= (Record.without #record((= a n) (= b 2)) (b)) #record((= a n))) 1 0))
          (if (= (Record.with #record((= a 1) (= b n)) #"a" 9) #record((= a 9) (= b n))) 1 0)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 11 Int64)))

(case
  "merge- and extend-REACHED records with runtime leaves equal the directly-built record"
  (doc
    "The growing row-ops: `(Record.merge {a:n} {b:2})` unions two runtime records (tens digit) and
           `(Record.extend {a:n} b 2)` adds a field (ones digit) — both must land on the identical byte
           form as the directly-written {a:n, b:2} → 11. Merge assembles from two heap operands and
           extend from one plus a fresh leaf; either landing on a different canonical field order than
           literal construction would compare unequal while projecting identically.")
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (*
            10
            (if (= (Record.merge #record((= a n)) #record((= b 2))) #record((= a n) (= b 2))) 1 0))
          (if (= (Record.extend #record((= a n)) #"b" 2) #record((= a n) (= b 2))) 1 0)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 11 Int64)))

; --- Cross-domain composition: an EDIT-REACHED collection nested INSIDE a record field. The
; remove-path canonicalization pins (05-compound-types) compare whole maps at top level; this one
; drops the via-remove map ONE level down, where the record-equality walk must descend into the
; CHAMP field and see the same canonical structure direct construction gives. ---
(case
  "a record field holding a via-remove map equals the direct-field record"
  (doc
    "Cross-domain composition of remove-path canonicalization: the edit-reached collection sits
           INSIDE another compound — a record whose field `m` holds a map reached via insert-then-remove
           must equal the record built with the directly-constructed map in that field (tens digit, ∀a),
           while a decoy record differing only in the OTHER field stays unequal (ones digit) → 10. The
           record-equality walk descends into the CHAMP field; a remove that left non-canonical structure
           one level down would flip the tens digit while top-level fields still agree.")
  (input
    (do
      (def (via (: a Int64)) (Map.remove #map((= 1 a) (= 2 20)) 2))
      (def
        (main (: a Int64))
        (let
          ((recv #record((= m (via a)) (= t 1)))
            (recd #record((= m #map((= 1 a))) (= t 1)))
            (decoy #record((= m #map((= 1 a))) (= t 2))))
          (+ (* 10 (if (= recv recd) 1 0)) (if (= recv decoy) 1 0))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 10 Int64))
  (call main (: 7 Int64))
  (output (: 10 Int64)))

(case
  "a computed (runtime-branched) label operand to Record.with is rejected — labels are static"
  (doc
    "The computed-expression companion of the bare-identifier CDZ0215 pin: `(Record.with r
           (if b #\"x\" #\"y\") 9)` supplies the name-introduction operand as an `if` over two genuine
           `#label` literals — a well-typed Symbol expression, but not a STATIC label. The static-label
           rule (a field name is part of the record's TYPE, so it cannot be runtime data) rejects it
           CDZ0215 exactly as it rejects the bare-identifier pun, on all targets uniformly. Guards the
           other half of the footgun: a user computing a Symbol and expecting dynamic field naming gets
           the coded static-label diagnostic, not a silent pun or a backend-dependent behavior.
           (Dynamic key→value association is what Map is for.)")
  (input
    (do
      (def
        (main (: b Bool))
        (let ((r2 (Record.with #record((= x 1) (= y 2)) (if b #"x" #"y") 9))) (+ (* 10 r2.x) r2.y)))
      (export main)))
  (call main (: true Bool))
  (error CDZ0215))

(case
  "records reached via MERGE and via WITHOUT dedupe with the direct build as one Set element"
  (doc
    "Construction-path canonicalization for RECORDS (the set/bytes/string/BigInt siblings are pinned
           in their files): `(Record.merge (record (a n)) (record (b 2)))`, `(Record.without (record (a n)
           (b 2) (c 9)) (c))`, and the direct `(record (a n) (b 2))` are THREE derivation paths to one
           value — a Set holding all three has len 1. A merge that assembled an unsorted field layout, or
           a without that left a tombstone slot, splits an element off (2 or 3).")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def via-merge (Record.merge #record((= a n)) #record((= b 2))))
          (def via-without (Record.without #record((= a n) (= b 2) (= c 9)) (c)))
          (Set.len #set(via-merge via-without #record((= a n) (= b 2))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a record reached VIA Record.with keys a Map like the directly-built record"
  (doc
    "The Map-KEY face of row-op canonicalization (the merge/without Set-dedupe case above pins
           3-paths-1-element): a record derived by `Record.with` (replacing field `a` in a runtime base)
           must hit a Map keyed by the directly-built `(record (a 5) (b 2))` (42) AND compare `=` to it
           (1) → 421. A with that produced a content-equal but layout-divergent record (a replaced slot
           left unsorted) misses the CHAMP lookup while possibly still passing a structural walk.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def base #record((= a n) (= b 2)))
          (def derived (Record.with base #"a" 5))
          (+
            (*
              10
              (match
                (Map.lookup (Map.insert Map.empty #record((= a 5) (= b 2)) 42) derived)
                ((Some v) v)
                ((None _u) -1)))
            (if (= derived #record((= a 5) (= b 2))) 1 0))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 421 Int64))
  (live-objects 0))

(case
  "a record reached via a with-CHAIN keys like the final direct build, generations intact"
  (doc
    "Three `Record.with` generations replace EVERY field of `g0` in turn (a→1, b→2, c→3); the
           final generation must key a Map like the direct `(record (a 1) (b 2) (c 3))` (7) while the
           ORIGINAL g0 still reads its own field (9) → 79. Persistence (g0 untouched through three
           derivations) composed with canonicalization (the chain result is byte-identical to the
           direct build as a CHAMP key).")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def g0 #record((= a n) (= b n) (= c n)))
          (def g3 (Record.with (Record.with (Record.with g0 #"a" 1) #"b" 2) #"c" 3))
          (+
            (*
              10
              (match
                (Map.lookup (Map.insert Map.empty #record((= a 1) (= b 2) (= c 3)) 7) g3)
                ((Some v) v)
                ((None _u) -1)))
            g0.a)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 79 Int64)))

; -- runtime-leaf row ops: without/project/merge over computed leaves + canonical-order value equality (breaker batch 371, from the 2026-07-17 banked candidate) --
(case
  "rrow1 Record.without over RUNTIME leaves drops the field and the rest projects"
  (input
    (do
      (def (main (: n Int64)) (. (Record.without #record((= a n) (= b (* n 2)) (= c 7)) (b)) a))
      (export main)))
  (call main (: 21 Int64))
  (output (: 21 Int64)))

(case
  "rrow2 Record.project over RUNTIME leaves keeps the named fields readable"
  (input
    (do
      (def
        (main (: n Int64))
        (+
          (. (Record.project #record((= a n) (= b (* n 2)) (= c 7)) (a c)) a)
          (. (Record.project #record((= a n) (= b (* n 2)) (= c 7)) (a c)) c)))
      (export main)))
  (call main (: 30 Int64))
  (output (: 37 Int64)))

(case
  "rrow3 a Record.merge result with a COMPUTED runtime leaf VALUE-EQUALS the directly-written literal across field order"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (= (Record.merge #record((= b (* n 2))) #record((= a 1))) #record((= a 1) (= b 42)))
          1
          0))
      (export main)))
  (call main (: 21 Int64))
  (output (: 1 Int64)))

; -- breaker batch 497 (2026-08-27): open-row COMPOSITIONS. orp1 = an open-row projection that
; IGNORES a heap-list sibling — the dead heap field is eliminated at compile time (the whole
; record folds; branch-select does not stop it), a correct dead-heap elimination through row
; polymorphism. orp2 = an open-row helper projecting a field whose value is a performing DRAW —
; the effects fold composes with row-poly projection.
(case
  "orp1 an open-row projection ignoring a heap-list sibling eliminates the dead field at compile time"
  (input
    (do
      (def (get-x r) r.x)
      (def
        (main (: n Int64))
        (let
          ((rec
              (if (> n 0) #record((= x n) (= ys #list(n (+ n 1)))) #record((= x 9) (= ys #list(1))))))
          (get-x rec)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "orp2 an open-row helper projects a field carrying a performing draw through the fold"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (get-x r) r.x)
      (def
        (main (: n Int64))
        (handle St 10 ((get (u) s (resume s s))) (get-x #record((= x (St.get)) (= y n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

; -- reading THROUGH an empty-side split-at folds like the equivalent literal (migrated from rcdzc
; accessing_through_an_empty_side_split_at_folds_like_the_literal): a split at k=0/k=arity has a Unit empty
; side, yielding (Tuple Unit (Tuple …)); projecting THROUGH it (.1 .0 / .0 .0) once declined ("a Unit tuple
; element needs the value heap") but now folds through the constant tuple split-at produces, reaching the
; same representation the byte-identical literal does. (The split-at operation itself is covered @982/@989.)
(case
  "sat1 reading through an empty-PREFIX split-at folds to the projected element"
  (doc
    "`(Tuple.split-at (tuple 10 20) 0)` = (tuple unit (tuple 10 20)); `.1 .0` reads the suffix's first
           element = 10.")
  (input (. (. (Tuple.split-at #tuple(10 20) 0) 1) 0))
  (output (: 10 Int64)))

(case
  "sat2 reading through an empty-SUFFIX split-at folds to the projected element"
  (doc
    "`(Tuple.split-at (tuple 10 20) 2)` = (tuple (tuple 10 20) unit); `.0 .0` reads the prefix's first
           element = 10.")
  (input (. (. (Tuple.split-at #tuple(10 20) 2) 0) 0))
  (output (: 10 Int64)))

(case
  "sat3 an interior split's suffix element is read correctly (not an empty-side-only fold)"
  (doc "`(Tuple.split-at (tuple 10 20 30) 2)` = (tuple (tuple 10 20) (tuple 30)); `.1 .0` = 30.")
  (input (. (. (Tuple.split-at #tuple(10 20 30) 2) 1) 0))
  (output (: 30 Int64)))

; ── breaker batch 578: row-polymorphism × census (the file is census-light: 19/117). An open-row
; accessor dispatched over fifty frames of MIXED-width runtime records reclaims completely, and a
; HEAP field read through the row reclaims too — row-poly access is census-clean at both faces.
(case
  "rwc1 an open-row accessor over runtime records of two widths, fifty frames, reclaims completely"
  (input
    (do
      (def (get-x r) r.x)
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (if
              (= (% k 2) 0)
              (get-x #record((= x k)))
              (get-x #record((= x (* k 10)) (= y k) (= z 7))))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 6900 Int64))
  (live-objects 0))

(case
  "rwc2 an open-row accessor reads a HEAP (list) field through the row, fifty frames, reclaims"
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def (get-xs r) (List.len r.xs))
      (def
        (frames (: k Int64))
        (if (= k 0) 0 (+ (get-xs #record((= xs (bld 3)) (= tag k))) (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 150 Int64))
  (live-objects 0))

; (migrated from rcdzc an_open_sum_match_requires_an_open_tail_wildcard_arm — type-system.md §A Sum Type May
;  Be Open, With A Mandatory Open-Tail Arm. An OPEN sum's row variable stands for unnamed variants, so a match
;  covering every NAMED variant but omitting `_` is still non-exhaustive; the SAME arm set over a CLOSED sum IS
;  exhaustive. Both halves are backend-agnostic.)
(case
  "an open sum match without an open-tail wildcard arm is non-exhaustive"
  (input
    (do
      (type V (Known Int64) (Unknown Int64) .. r)
      (def (f (: v V)) (match v ((Known n) n) ((Unknown n) n)))
      (def (main) (f (Known 1)))
      (export main)))
  (error CDZ0210))

(case
  "a closed sum match covering every named variant is exhaustive without a wildcard"
  (input
    (do
      (type V (Known Int64) (Unknown Int64))
      (def (f (: v V)) (match v ((Known n) n) ((Unknown n) n)))
      (def (main) (f (Known 1)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "an open sum match WITH an open-tail wildcard arm is exhaustive and compiles"
  (input
    (do
      (type V (Known Int64) (Unknown Int64) .. r)
      (def (f (: v V)) (match v ((Known n) n) (_ 0)))
      (def (main) (f (Known 1)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

; rrx1: open-row instantiation x the EFFECT fold — one row-polymorphic projector applied at TWO
; slot-shifted widths whose `t` fields are PERFORMS. The projector instantiates per call site
; ((v t) vs (a v t): both projected slots shift), and the effect lowering must thread the handler
; state left-to-right through the two record LITERALS' field inits: the first record's tick reads n,
; the second n+1, regardless of layout. 1 + 10n + 1000*(2 + 10*(n+1)) = 12001 + 10010n. A
; single-layout specialization misreads a slot; a wrong material-ization order swaps the ticks.
; (breaker probe rr2, verified tri-target exact + byte-idempotent; the record-returning performing
; HELPER face (let + projection, 2101 + 110n) verified same tick — projections carry no binder, so
; the eg1-collision capture surface does not exist here.)
(case
  "an open-row projector at two widths reads performing field inits in program order"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (get-vt r) (+ r.v (* 10 r.t)))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (+
            (get-vt #record((= v 1) (= t (C.tick))))
            (* 1000 (get-vt #record((= a 9) (= v 2) (= t (C.tick))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 12001 Int64))
  (call main (: 3 Int64))
  (output (: 42031 Int64)))

; osx1: OPEN-SUM dispatch x the EFFECT fold — the open-sum sibling of orp2 (which composes the fold
; with open-ROW projection). A performing helper returns an open-sum value ((A (+ k tick)) or
; (B tick), ascribed to the open V), and the caller matches named + OPEN-TAIL arms, with the second
; match's open-tail arm containing a THIRD performing match. The handler state must thread through
; the variant construction AND the open-tail dispatch in program order: tick1 = n builds A(1+n)
; (named arm), tick2 = n+1 builds B(n+1) (falls to the open tail), tick3 = n+2 builds A(4+n) inside
; the tail. (1+n) + 1000*(4+n) = 4001 + 1001n. A fold that reorders the performs across the open-tail
; boundary, or an open-tail bind that disturbs the pending handler state, shifts a digit. (breaker
; probe os1, verified tri-target exact + byte-idempotent.)
(case
  "open-tail dispatch of a performing open-sum helper threads handler state in program order"
  (input
    (do
      (type V (A Int64) (B Int64) .. r)
      (effect C (op tick (-> Int64)))
      (def (mk (: k Int64)) (: (if (> k 0) (A (+ k (C.tick))) (B (C.tick))) V))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (+
            (match (mk 1) ((A x) x) (_ -50))
            (* 1000 (match (mk 0) ((A x) x) (_ (match (mk 2) ((A y) y) (_ -70))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 4001 Int64))
  (call main (: 3 Int64))
  (output (: 7004 Int64)))

; rwx1: the row-op CHAIN x the EFFECT fold — merge/with/pop over a base whose fields, merge operand,
; and with-value each PERFORM against one advancing handler. Program order threads tick1 into the
; base literal (a = n), tick2 into the MERGE operand's field (c = n+1), tick3 into the WITH value
; (the b override = n+2); pop #"b" then yields that third draw and the rest keeps a/c/d. The chain
; pins ordering through OP ARGUMENTS (the literal-init face is rrx1's, the projection-draw face is
; orp2's): (n+2) + 100n + 10000(n+1) + 7000000. A fold reordering the operand materialization
; across the chain shifts a digit; Record.merge's disjointness (CDZ0211, pinned above) is respected
; by construction. (breaker probe rw2, verified tri-target exact — rust differential 30303 — with a
; benign h1->h2-stabilizing hop drift (values identical both generations) and census TRUE-0 on the
; fresh debug runtime.)
(case
  "a merge-with-pop chain threads three performing operands in program order"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (let
            ((r #record((= a (C.tick)) (= b 100))))
            (match
              (Record.pop
                (Record.with (Record.merge r #record((= c (C.tick)) (= d 7))) #"b" (C.tick))
                #"b")
              (#tuple(v rest) (+ v (+ (* 100 rest.a) (+ (* 10000 rest.c) (* 1000000 rest.d)))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7010002 Int64))
  (call main (: 3 Int64))
  (output (: 7040305 Int64)))

; tcx1: a RUNTIME Tuple.concat — the const concat cases above fold entirely at compile time (the
; concat never runs); this forces the runtime tuple-build with a RUNTIME first element `n`, then reads
; Tuple.size of the result AND destructures all four mixed-type elements. Pins that concat EXECUTES at
; runtime preserving a runtime element's value (not just its static type/arity): concat #tuple(n 20)
; ++ #tuple(true #"x") = a size-4 #tuple(n 20 true #"x"); size=4, and a=n / b=20 / c=true /
; d=#"x" flow through. 1000*4 + n + 10*20 + 100*1 + byte-len("x")=1 = 4301 + n. (breaker probe tp1,
; verified tri-target exact + byte-idempotent — benign hop drift, identical values both generations.)
(case
  "a runtime Tuple.concat builds the tuple, reports size, and destructures mixed elements"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((t (Tuple.concat #tuple(n 20) #tuple(true #"x"))))
          (+
            (* 1000 (Tuple.size t))
            (match
              t
              (#tuple(a b c d)
                (+
                  (Int64.of a)
                  (+
                    (* 10 (Int64.of b))
                    (+ (* 100 (if c 1 0)) (String.byte-len (Symbol.to-string d))))))
              (_ -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4306 Int64))
  (call main (: 0 Int64))
  (output (: 4301 Int64)))

; tcx2: Tuple.concat x the EFFECT fold — the concat's operand-tuple ELEMENTS perform, so the two
; performing elements (one in each concat operand) must thread the handler state in program order:
; first operand's `(C.tick)` reads n, second operand's reads n+1, and concat interleaves them at the
; right positions. #tuple((C.tick) 100) ++ #tuple((C.tick)) = #tuple(n 100 n+1); a + 10*b + 1000*c =
; n + 1000 + 1000*(n+1) = 1000n + n + 2000... = 2000 at n=0, 7005 at n=5. The runtime-Tuple.concat
; face (tcx1) fixed the operands; this pins that PERFORMING operand elements thread state correctly
; across the concat (a materialization that reordered the two operands' performs would swap n and
; n+1). (breaker probe ex3, verified tri-target exact + byte-idempotent, scalar.)
(case
  "Tuple.concat threads performing operand elements in program order under a handler"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (let
            ((t (Tuple.concat #tuple((C.tick) 100) #tuple((C.tick)))))
            (match t (#tuple(a b c) (+ a (+ (* 10 b) (* 1000 c)))) (_ -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2000 Int64))
  (call main (: 5 Int64))
  (output (: 7005 Int64)))

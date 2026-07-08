# A record's field type is not checked against a contradicting annotation

*2026-07-08*

**What happened.** Adversarial probing of the annotation checker found that a provably-contradictory
annotation on a RECORD's field type is silently accepted. `(: (record (a 1)) (Record (a Bool)))`
annotates a `(Record (a Int64))` value as `(Record (a Bool))` — the head `Record` and the field name
`a` agree, but the field's type `Int64` cannot unify with `Bool` — and it runs to `(record (a 1))`
under the declared wrong type, instead of being rejected CDZ0203. The exact same contradiction on the
two sibling structural forms IS caught: `(: (list 1 2) (List Bool))` and `(: (Some true) (Option
Int64))` both correctly decline "annotation's parameter type contradicts the value."

Probing wider, the record-annotation check is even coarser than a missing field-type descent — it does
not check the head or the field set either: `(: (record (a 1)) (Tuple Int64))` (record annotated as a
tuple), `(: (record (a 1)) (Record (b Int64)))` (wrong field name), and `(: (record (a 1)) (Record (a
Int64) (b Bool)))` (extra field) are all accepted. Only the coarse scalar-vs-compound split fires —
`(: (record (a 1)) Int64)` correctly declines. So a record annotation is effectively unchecked beyond
"is it a compound at all."

**Why it is a break.** type-system.md #Annotations Constrain, Never Contradict: "A program whose
annotation cannot be unified with the type inference determines MUST be rejected rather than have the
annotation silently replace the inferred type." `(Record (a Int64))` cannot unify with `(Record (a
Bool))`, so the program MUST be rejected (CDZ0203). Accepting it is exactly the
annotation-silently-replaces-inference the section forbids. A record is one of the three structural
types (type-system.md #The Structural Types Are Record, Tuple, And Sum) beside the tuple and the sum;
the annotation-parameter check the corpus already pins for a tuple position, a sum payload, and a list
element must also cover a record field.

**Root cause (likely) — the annotation-unification descent covers tuple/list/sum parameters but not
record fields.** The seed's annotation-contradicts check (`matches_annotation` /
`annotation_payload_param`, the descent recorded in `[[annotation-contradicts-recurses-all-shapes]]`)
recurses into a tuple's positions, a list's element, and a sum's payload, comparing each against the
value's inferred shape. The record case is not in that descent: a `(Record …)` annotation is matched
only at the coarse "is the value a record" level, so neither the field set nor the per-field types are
unified. The fix is to add the record arm to the same descent — unify the annotation's field NAMES with
the value's field set and each field's declared type with the field value's inferred type, rejecting
CDZ0203 on any provable mismatch, exactly as the tuple/list/sum arms do.

**The lesson (the recurring family).** A check pinned on some members of a family but not all — here the
three structural types, with the annotation-parameter check landed for tuple positions, list elements,
and sum payloads but NOT record fields. This is the same shape as the collection-growth-operator, the
if/match unselected-alternative, the int/float separator, and the call/perform argument-type findings:
a rule proven on one form must carry to every sibling. The tell: the identical contradiction
(`Int64` where `Bool` is declared) rejects under a list/option annotation but runs under a record
annotation — the checker changed its thoroughness with the container, not the contradiction.

**Corpus case added.** `spec/semantics/07-type-system.sexp` §"a record annotated with the wrong field
type is rejected" — `(: (record (a 1)) (Record (a Bool)))` MUST reject CDZ0203, the record companion of
the existing list-element and option-payload annotation-descent cases. Native seed; the behavior gate
catches it (expected reject CDZ0203, observed a running component). A generation that does not yet check
a record field's type declines rather than accepting.

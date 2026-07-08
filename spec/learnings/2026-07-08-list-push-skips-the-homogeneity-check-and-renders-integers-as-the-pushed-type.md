# List.push skips the homogeneity check and renders integers as the pushed type

*2026-07-08*

**What happened.** Adversarial probing of list functional construction found a wrong-value
miscompile: `(List.push (list 1 2) true)` returns `(list true true true)`. The two Int64 elements
`1` and `2` come back rendered as `true`. `(List.push (list 10 20) false)` → `(list true true
false)` is the sharpest witness — the distinct integers 10 and 20 both project as `true`, while
the pushed `false` projects as `false`. The homogeneous control `(List.push (list 1 2) 3)` →
`(list 1 2 3)` is correct, and the *literal* `(list 1 true)` correctly rejects CDZ0201 — only
`List.push` of a differently-typed element is broken.

**Why it is a break.** collections-and-text.md #A List Is An Ordered Homogeneous Sequence: "A
list MUST be an ordered sequence whose elements share one type." #A List Is Grown By Functional
Construction: `List.push` "MUST produce a new list value" — a list value, which must be
homogeneous. So `(List.push (list 1 2) true)` builds a non-homogeneous list, the same violation
as the `(list 1 true)` literal the corpus already rejects (CDZ0201). `List.push` does not enforce
the element-type check the literal enforces, so it slips through — and worse, it does not merely
build a heterogeneous list: it renders the RESULT at the pushed element's type, projecting the
stored integers `1 2` back out as booleans. That is exactly the "projecting a mixed element back
out at a different type" hazard the homogeneity rule exists to prevent (corpus comment at
05-compound-types §"A list is homogeneous"). A wrong value, strictly worse than a missing
rejection.

**Root cause — the type check lives on the literal path, not the push path.** The `(list …)`
literal has a homogeneity check (it rejects `(list 1 true)`), but `List.push`'s lowering appends
the new element to the runtime list without checking its type against the operand list's element
type, and the emitted renderer walks the whole result at the pushed element's type (so an Int
element rendered as Bool prints its nonzero value as `true`). The fix is to give `List.push`
(and `List.update`, which replaces an element and has the same exposure) the same element-type
check the literal has: a pushed/replacement element whose type differs from the list's element
type is CDZ0201, or a decline if not yet checked — never a build that renders the old elements at
the new type.

**The lesson (a sibling of the module-value-def and if-branch findings).** A type rule proven on
one construction path (the `(list …)` literal) must hold on every other path that builds the same
value kind (`List.push`, `List.update`). The literal and the functional-construction operators
produce the *same* list value form, so the homogeneity invariant is a property of the value, not
of one syntax — but the check was written only for the literal. This is the recurring shape:
an invariant enforced where it was first written, not at every site that must maintain it. The
render corruption is the tell that the value's element type and its rendering type had drifted
apart — the push set the element without reconciling the list's type, and the renderer trusted
the pushed type for the whole list.

**Corpus cases added.** `spec/semantics/05-compound-types.sexp` §"pushing an element of a different
type onto a list is a type error" — `(List.push (list 1 2) true)` MUST reject CDZ0201, as the
functional-construction companion of the `(list 1 true)` literal case. Native seed; the behavior
gate catches it (expected reject CDZ0201, observed a running component that returns the corrupted
`(list true true true)`).

**Confirmed sibling (2026-07-08, next cycle): `List.update` has the identical bug.** #A List Is
Grown By Functional Construction pairs "append an element" (`push`) with "replace the element at an
index" (`update`), so `update` carries the same homogeneity obligation. `(List.update (list 1 2 3)
1 true)` returns `(list true true true)`, and `(List.update (list 10 20 30) 0 false)` returns
`(list false true true)` — the untouched integers 20 and 30 project as booleans, the same render
corruption. Pinned as §"updating a list slot with an element of a different type is a type error"
(CDZ0201). The fix must give BOTH functional-construction operators the literal's element-type
check; a fix to `push` alone leaves `update` corrupting. (Verified this cycle that the sibling
operators `Bytes.of`/`Bytes.concat`/`String.concat` and the `List.at` index all correctly reject a
wrong-typed operand — only the two list-growth operators skip the check.)

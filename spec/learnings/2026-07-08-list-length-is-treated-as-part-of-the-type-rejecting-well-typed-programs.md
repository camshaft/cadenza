# List length is treated as part of the type, rejecting well-typed programs

*2026-07-08*

**What happened.** Adversarial probing found the compiler treats a list's LENGTH as part of its
type — as if a list were a fixed-arity tuple — and wrongly rejects well-typed programs that
compare or branch between two same-element-type lists of different length. Both surfaced as the
gate's "wrongly rejected a valid program":
- `(= (list 1 2) (list 1 2 3))` → rejected CDZ0201 "comparison between values of different shapes",
  where it must compute `false` (two `(List Int64)` values, unequal by their elements).
- `(if true (list 1 2) (list 3 4 5))` → rejected CDZ0201 "conditional branches have different
  shapes", where it must yield `(list 1 2)` (both branches are `(List Int64)`).

**Why it is a break.** A list is a VARIABLE-LENGTH sequence typed by its ELEMENT type
(collections-and-text.md #A List Is An Ordered Homogeneous Sequence: "an ordered sequence whose
elements share one type"; #A List Is Grown By Functional Construction: length varies at runtime via
`List.push`). So two lists of the same element type are the SAME type `(List <elem>)` regardless of
length. Equality on two same-type values must be TOTAL (core-semantics.md #Equality Is Structural —
comparable when their types match), yielding `false` for different elements, never a type error; and
a conditional whose branches share a type is well-typed. Rejecting these is a false rejection of a
well-typed program (§A Well-Typed Program Does Not Go Wrong, contrapositive: a well-typed program is
not rejected).

**Root cause — the tuple-arity shape check was reused on lists.** The structural shape-compatibility
check (`shapes_incompatible`, used by both `gen_eq` and the if-branch-agreement check) compares two
compound values and, for a tuple, treats a different ARITY as an incompatible shape — correctly,
because a tuple's arity IS part of its type. The same check applies a length comparison to lists,
but a list's length is NOT part of its type. So it reports two different-length lists as
"different shapes" and both the equality path and the if-branch path reject. The fix is for
`shapes_incompatible` to compare lists by their ELEMENT type only (recursing into a representative
element), never by length — the tuple rule (arity is significant) and the list rule (length is not)
must be distinct arms, not one length-comparing path shared across both.

**The distinction the check must honor.** Tuple arity is part of the type; list length is not. The
corpus pins both sides elsewhere — a tuple pattern/branch of the wrong arity is a type error
(different types), while `(list 1 2 3)` and a pushed-longer list are the same type. Conflating them
in one shape check makes the list side reject what it must accept. This is the mirror of the earlier
list-homogeneity findings: there the ELEMENT TYPE must be checked (and was skipped on the growth
operators); here the LENGTH must NOT be checked (and is wrongly checked) — element type is the
list's type, length is not.

**Corpus cases added.** `spec/semantics/05-compound-types.sexp` §"two lists of different length are
unequal, not a type error" (`(= (list 1 2) (list 1 2 3))` → `false`) and
`spec/semantics/02-binding-and-control.sexp` §"a conditional with two list branches of different
length is well-typed" (`(if true (list 1 2) (list 3 4 5))` → `(list 1 2)`), both the list
counterpoints to the tuple-arity cases (where a length/arity difference IS a type error for tuples).
Native seed; the behavior gate catches both as "wrongly rejected a valid program."

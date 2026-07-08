# A tuple-typed variant payload is not checked

*2026-07-08*

**What happened.** After the constructor-payload-type check was generalized (c51 landed scalar/String/
List; c53 landed built-in Ast; probing showed Record payloads checked too), adversarial re-probing found
one payload shape still uncovered: a TUPLE-typed payload. `(type T (Pair (Tuple Int64 Int64)))` declares
`T.Pair` with payload type `(Tuple Int64 Int64)`, but `(T.Pair (tuple 1 2 3))` — a three-element tuple
where a two-element one is declared — is accepted and constructs `(T.Pair (tuple 1 2 3))`; matching it and
projecting `(tuple.2 p)` yields `3`, a position the declared two-element payload type does not have.
`(T.Pair 5)` (a scalar where the tuple payload is declared) and `(T.Pair (tuple 1 true))` (wrong element
type) slip through the same way. Every other payload shape is now checked: scalar (`(T.Mk "x")`), List
(`(T.W 42)`), Record (`(T.R 5)`), and the built-in Ast constructors all reject.

**Why it is a break.** A constructor is a single-arity function whose argument is type-checked
(core-semantics.md #A Sum Type Constructor Is A Single-Arity Function + #Applying A Function Binds Its
Parameter To Its Argument), and a tuple's length is part of its type (type-system.md #A Tuple Is Reshaped
Positionally, whose length is part of its type; #The Structural Types Are Record, Tuple, And Sum — a
tuple's shape is its element types in order). So `(Tuple Int64 Int64)` cannot unify with a three-element
tuple, and `(T.Pair (tuple 1 2 3))` is a type mismatch, CDZ0201. Constructing it and then reading
position 2 (which the declared arity forbids) is a wrong-value miscompile.

**Root cause (likely) — the generalized payload-type check compares scalar/String/List/Record shapes but
not Tuple.** The c51/c53 fixes routed the constructor argument through a payload-type comparison that now
covers scalars, String, List (element type), Record (field types), and the Ast constructors — but the
Tuple case (arity + positional element types) was not added to that comparison, so a tuple payload of any
shape passes. The fix is to include the tuple shape (arity and per-position element types) in the same
payload-type comparison, reusing the tuple annotation-contradiction descent (which already checks tuple
arity and element types for `(: (tuple …) (Tuple …))`).

**The lesson (the recurring family — the master pattern).** A check generalized across most variations of
a dimension left one variation uncovered: the payload-type check reached scalar/String/List/Record/Ast
but not the Tuple payload shape. This is the same "a check proven on one variation must carry to every
sibling variation" master pattern seen throughout this run (across form, type, position, name-set,
codepath, depth) — here across the *payload type shape*. When a check is generalized to "the full declared
type", every type shape the language has (scalar, String, List, Tuple, Record, sum) must be in the
comparison, or the omitted shape is a silent hole. The tell: `(T.Mk "x")` (scalar payload) rejects but
`(T.Pair (tuple 1 2 3))` (tuple payload) — the same wrong-payload construction — is accepted.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a unary variant applied to a wrong-arity
tuple payload is a type error" — `(T.Pair (tuple 1 2 3))` for `(type T (Pair (Tuple Int64 Int64)))` MUST
reject CDZ0201, the tuple-payload companion of the scalar unary-variant case above it. Gated `(needs
sum-type-declaration)`, which the seed realizes, so the behavior gate runs and catches it (expected reject
CDZ0201, observed a running component). A generation that does not yet check a tuple-typed payload
declines rather than constructing the mistyped value.

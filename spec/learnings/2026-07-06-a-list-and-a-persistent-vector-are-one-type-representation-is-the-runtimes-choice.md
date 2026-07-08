# A list and a persistent vector are one type — representation is the runtime's choice, not the author's

*2026-07-06*

**What happened.** The language had grown *two* surfaces for the same idea — an ordered, homogeneous,
immutable, indexed sequence — and a review asked whether both were needed. They were not.

The first surface is the **list**: the `(list 1 2 3)` literal, `List.at` (fallible, Option-returning),
rendered `(list …)`, and specified in `collections-and-text.md` §"A List Is An Ordered Homogeneous
Sequence". At run time a list is a flat positional array — the *same* array the runtime uses for a
tuple and a record (the runtime's own note says so: "the ONE runtime shape for TUPLE, RECORD, and
LIST … a list is the same array; it differs from a tuple only in its static type").

The second surface is the **persistent vector** (`Vec`): `Vec.empty`/`Vec.push`/`Vec.update`/`Vec.len`/
`Vec.get`, rendered `(vec …)`, realized as a 32-way radix trie in the value-heap runtime and
documented in [a persistent collection fits the tagless heap](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md).
It arrived as the output accumulator a self-hosting compiler wants — a functional, structurally-shared
growable sequence with O(log₃₂ N) push/update — and in doing so quietly acquired a *surface type*, a
*render form*, and an *API namespace* of its own.

Three observations decided the merge:

1. **Only one of the two was ever specified.** `collections-and-text.md` defines Lists, Maps, and Sets;
   there is no Vector requirement anywhere. `Vec` exists only in the semantics corpus (the
   `(needs persistent-vector)` cases in `05-compound-types.sexp`) and one learning. So the
   *specification* already commits to exactly one ordered-homogeneous-sequence type. The second type
   was an implementation artifact — an accumulator — that grew a public face without ever earning a
   requirement.

2. **A flat array is the trie's base case, not a different representation.** A 32-way radix trie holding
   ≤ 32 elements is a *single leaf node* — an array of handles — which is byte-for-byte what a list
   already is at run time. `list` and `Vec` are therefore not two representations sitting across a type
   boundary; one is the small case of the other. Choosing between them at authoring time is choosing a
   point on a single data structure's size curve and calling the two ends different types.

3. **Their *observable* contracts are identical.** Both are ordered, homogeneous, immutable, indexed by
   position, fallible on out-of-bounds read, and length-queryable; the vector adds functional
   `push`/`update` that produce a new version while leaving the old one untouched — but a *list* is
   equally immutable and equally entitled to a functional append. Nothing an author can observe
   distinguishes them **except the performance curve**, which is exactly the thing the architecture has
   staked itself on keeping invisible.

So the two collapse into **one sequence type**. It keeps the list's name, its `(list …)` literal, and
its `(list …)` render; it absorbs the functional growth operations (`push`/`update`) and the radix-trie
representation. The runtime is free to back a small or literal sequence with a flat array (a single
leaf) and a large or push-heavy one with a full trie — *invisibly*, choosing and even migrating
between them as an unobservable representation decision. `Vec` disappears as a surface type, `(vec …)`
disappears as a value form, and the trie becomes the *representation* of a list, never a second type
the author selects.

**Why.** Two surface sequence types is not a small redundancy — it *contradicts the central bet of the
whole runtime architecture.* The [runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md),
the memory model's #Sharing Is Not Observable requires that a value that shares storage and a value
that copies it be "indistinguishable by every operation … including equality, length, indexing, and
the value's canonical byte form", and the persistent-collections learning states the thesis outright:
"the representation can change freely (size classes, Perceus reference counting, CHAMP/RRB collections)
with zero emitted-byte impact, because nothing outside it observes a tag." Exposing `list` and `Vec` as
distinct types with distinct render forms makes representation observable *at the surface* — it lets a
program branch on, and serialize differently, two values that differ only in how their identical
contents are stored. That is precisely the observability the design spent the tag-free runtime, the
frozen ABI, and three prior learnings eliminating. The redundancy did not merely duplicate a type; it
punched a hole in the invariant the runtime's cheapness depends on.

There is a coherent language design that keeps multiple sequence types on purpose — Clojure's `list`
vs `vector`, Rust's `Vec` vs `im::Vector` — where the *author* deliberately picks the performance
profile and accepts that the two are different, non-interconvertible types. That is a legitimate
philosophy, but it is the *opposite* of this project's: it makes representation an author-visible,
type-level choice, whereas Cadenza's memory model, ABI, and collections learnings all insist
representation is the runtime's private concern. Cadenza did not choose author-controlled
representation deliberately; it drifted into it through an accumulator that was never meant to be a
public type. Merging is not giving something up — it is refusing to smuggle in, through a back door, a
design commitment the specification never made and its architecture actively argues against. If
author-controlled representation is ever wanted, it should be a deliberate, specified decision with its
own requirement, not the residue of an implementation convenience.

The reason the merge is *cheap* is the same tag-free property that made the vector cheap to add in the
first place. Because the runtime holds structure and data but no type identity, one sequence type whose
representation varies by size costs nothing to dispatch: the compiler knows the static type at every
use site (it is a sequence), and the runtime picks a leaf-array or a trie behind the opaque handle with
no runtime check and no observable difference. The vector's operations were always defined by *what*
they do (append, replace-at-index, read, count), never *how* they store it — which is exactly the seam
the collections learning said would let a representation grow "with zero emitted-byte impact." Folding
the trie in as the list's representation is that prediction paid out one more time: the seam stays the
list's observable operations; the trie moves underneath it.

**The requirement it drove.** `collections-and-text.md` §"A List Is An Ordered Homogeneous Sequence" is
the single home for the merged type and gains the obligations the vector carried:

- The sequence is **growable by functional construction** — an operation that appends an element, and
  one that replaces the element at an index, each MUST produce a new sequence value that leaves the
  operand unchanged (persistence), so a list is immutable under growth exactly as it is under read.
- A list's **representation is unspecified** — a conforming runtime MAY back it with a flat array, a
  radix trie, or any structure, and MUST keep the choice unobservable, so that two lists with equal
  elements in the same order are indistinguishable by every operation (equality, length, indexing, and
  canonical byte form) regardless of how each is stored. This is a *realization* of
  `memory-and-resource-model.md` #Sharing Is Not Observable applied to the sequence type, not a new
  cross-cutting rule.
- The existing element-in-order equality and fallible-indexing requirements are unchanged and now cover
  the one type.

`deterministic-value-form.md` loses the `(vec …)` value form: `(list …)` is the sole canonical text for
an ordered homogeneous sequence, so the byte-form surface *shrinks* by one variant rather than growing.
No new diagnostic — a heterogeneous list is the CDZ0201 it already is.

Downstream (the implementation, tracked separately since `implementation/` is disposable — this landed
with all four gates green: behavior 438/0, ignition byte-identical, component-check 443 agree, cargo
test 13/0). The corpus `(needs persistent-vector)` cases in `05-compound-types.sexp` folded into `list`
operations (`List.push`/`List.update`/`List.len` over `(list …)`, gated `(needs list-growth)`); the
`Vec` surface type, the `Vec.*` namespace, and the `(vec …)` render form are gone; the seed's realized-
capability set renamed `persistent-vector` → `list-growth`. Because the runtime already exposed both the
flat-array (`arr-*`) and trie (`vec-*`) operation sets, **no component-envelope re-derivation was forced**
— the compiler only re-pointed which operations a list lowers to, and `xtask build` confirmed the runtime
hash, the compiler hash, and the generated envelope/opcode blobs all unchanged. Implementing it sharpened
one point the argument above left soft: because the tag-free runtime cannot branch on how a value was
built, a runtime list must have a *single* representation — so `List.push` on a function parameter reads
the same structure whether the list was a literal or grown. Only the trie grows, so a **runtime list is
the trie uniformly** (`vec-empty` + a `vec-push` per element even for a `(list …)` literal; the renderer
walks it via `vec-len`/`vec-get`), and the flat `arr-*` array is reserved for the fixed-arity tuple/record.
The "flat array for a small list" freedom is real at the level of the specification's unobservability rule,
but the seed exercises the simpler uniform choice; the point that matters is that the *choice* is the
runtime's and invisible, not that both options are wired today. A split representation would have
reintroduced the exact observability bug the merge exists to kill — `(List.push (list 1 2 3) x)` reading a
flat-array header as a trie — which is why one representation per runtime value is not an optimization but
a correctness requirement under a tag-free heap.

This composes with — and is the natural next step after —
[a persistent collection fits the tagless heap](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md)
and [the runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md):
the vector was added cheaply *because* representation is invisible, and the same invariant now says it
should never have been a separate type. The lesson to carry to the next collection is the sharper form
of the tag-free thesis: **a new representation is a new way to store an existing type, not a new type —
if adding a data structure adds an author-visible type and render form, the representation has leaked
through the seam, and the fix is to fold it under the type whose contract it already satisfies.**

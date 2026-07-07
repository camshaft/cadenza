# The reader's whole foundation is built and verified — gated on a single inference bug, as dead code that comes alive when it's fixed

*2026-07-07*

**What happened.** The compiler-in-Cadenza spike finished the *foundation* of its reader — the last
piece before self-hosting — and the way it did so is itself the lesson. The reader's three
sub-capabilities are all written and verified against real canonical-AST bytes (`(quote 42)` =
`83 01 80 18 2A`):

- **Head decode** — `cbor-major` / `cbor-info` / `cbor-arg` / `be-bytes` / `cbor-head-len` extract an
  item's major type and (possibly multi-byte big-endian) argument
  ([[2026-07-07-the-reader-decodes-cbor-as-the-input-dual-of-the-output-spine]]).
- **Structural navigation** — `cbor-skip` / `skip-elems` (mutually recursive) walk *past* a whole
  item to the next offset, recursing into arrays element by element; verified `cbor-skip 0 = 5` on the
  whole `(quote 42)` array and `= 5` through a nested `[[1 2] 3]`.
- **Name resolution** — `prelude-entry` / `name-eq` locate the Nth prelude symbol and byte-compare it
  to a known operator name (`b"+"`, …) — the read-time "resolve names to codes" step, on raw bytes,
  needing no runtime `String`.

But `name-eq` is written in its natural `(if (= a b) (recurse) false)` shape, which **declines when
reached** — it is the recursive-Bool return-kind race
([[2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent]], SPEC-BACKLOG item 14).
So the spike left it in place as **dead code**: the compiler still builds (nothing calls `name-eq`
yet), and it "comes alive" the moment item 14 is fixed and the reader's top-level `read : Bytes → Node`
walk is wired to call it. The reader is thus *fully scaffolded and blocked on exactly one seed
inference bug* — every primitive it needs is written and independently verified; only the one function
whose shape trips the kind race is inert.

**Why.** This is a deliberate and healthy way to work against a known blocker, and worth recording as a
method (a sibling of "route around the blocker to prove the backend",
[[2026-07-06-the-compiler-emits-a-multi-function-module-with-a-real-call]]). Rather than stall on item
14, the spike built and verified *everything else* the reader needs and parked the one blocked function
as dead code — so when the kind-race fix lands, the reader is one wiring step from complete, not a
from-scratch effort. The cost is the honest caveat the spike itself records in `compiler.cdz`'s header:
the reader "foundation is built" is not "the reader works" — `name-eq` declines, so `bytes → AST` does
not yet run end-to-end, and calling that state "done" would be the modeled-subsystem trap
([[2026-07-02-a-modeled-subsystem-passes-a-shape-check]]). The status is precise: head decode + navigation
proven and *live* (a corpus case runs each), name resolution written but *inert* pending item 14. The
critical path to self-hosting is now a single named seed bug plus the wiring that follows it — the
narrowest the gap has been.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR skip walks past a whole
nested item to the next offset"* — pins the navigation primitive: mutually recursive `cbor-skip` /
`skip-elems` over `82 82 01 02 03` (an array whose first element is the array `[1 2]` and whose second
is the scalar `3`) returning offset 5, so the recursive element-walk through a nested array is a
durable gate obligation. It PASSES (navigation is live today) and joins the already-pinned head-decode
case as the two proven halves of `bytes → AST`: value extraction and structural navigation. The name
matcher's case is already pinned as the recursive-Bool item (todo, item 14). No new backlog item — this
records that the reader foundation is complete and consolidates the self-hosting gate as items 12
(symbol-table `from-bytes`, though the reader routes around it for structure), 13 (list patterns,
ergonomic), and 14 (the recursive-Bool name matcher, the one hard blocker the reader is parked on).

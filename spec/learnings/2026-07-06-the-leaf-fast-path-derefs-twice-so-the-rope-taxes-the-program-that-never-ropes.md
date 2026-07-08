# The leaf fast path derefs twice — the rope taxes the program that never ropes

*2026-07-06*

**What happened.** A cost review of the implicit Bytes rope asked the sharpening question: *if a
program never concatenates or slices — only builds a flat buffer and reads it back — what does it pay
for the rope machinery existing at all?* The answer was "almost nothing," with one real exception on
the single hottest loop. When Bytes became a rope
([a Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)),
`bytes-get` grew a two-step shape: first peek at the node to learn whether it is a leaf (`handles`
empty) or a rope node (slice/concat), *then* branch — because reading a rope node must call
`bytes_flatten`, which needs a mutable borrow, so the read cannot hold the shared borrow it used to
classify the node across the flatten call. The natural way to write that is: take a shared borrow to
read the arity, **drop it**, and if the node turned out to be a leaf, take a **second** shared borrow
to read the byte. So the leaf path — the path a pure-leaf program takes on *every* byte — now
dereferences the same node pointer **twice** where the pre-rope code dereferenced it once.

That loop is not incidental. The self-hosting compiler assembles a module and then reads the whole
thing out, `for i in 0..len { bytes-get(buf, i) }`, to hand the bytes across the boundary — the exact
loop the rope was introduced to keep O(n) instead of O(n²). The rope delivered the asymptotic win, but
on the flat read-out (after any rope has already flattened to a leaf) it left behind a ~2× **constant**
on the byte read, paid by the flat leaf that never roped — the read is still O(n), just with a needless
second pointer chase per byte. The fix is a one-liner with no ABI or spec impact: fold the classify and
the read into a single borrow — one `match` on the node that, when `handles` is empty, reads
`raw.get(index)` in the same arm, and only falls through to the drop-borrow-then-flatten dance for a
genuine rope node. The leaf path returns to one deref; the rope path is unchanged. (This learning
records the finding; the fold is not yet applied.)

**Why.** The root cause is a borrow-shape leaking into the fast path. Flatten-on-read is what makes the
rope O(n) rather than O(n·depth), and flatten mutates the node, so the read has a real reason to release
its shared borrow before it can branch into the mutating case. But that reason applies *only* to the
rope case. Writing the classify step as a standalone `let is_leaf = …; drop borrow;` made the borrow
release unconditional, and the leaf arm then had to re-borrow to do its work — the flat program paying
for a borrow discipline that exists solely for the shared/deferred program. This is a specific instance
of a general trap when an optimization is made *implicit*: the machinery for the interesting case
(here, the deferred-materialization write) tends to seep into the common case (the plain read) unless
the common case is deliberately carved back out. "Sharing is not observable" was honored for *values* —
a flat leaf and a rope of identical bytes read the same — but the review found the promise quietly
weakened for *cost*: the program that never constructs a shared or deferred value should also never pay
for the mechanism, and here it paid a small per-byte tax on the busiest loop in the system. The tax is
tiny and the corpus, a value oracle, is structurally blind to it — the same blind spot that let the
no-op `compact` ship
([wiring the Bytes rope … caught a compact that did nothing](./2026-07-06-wiring-the-rope-exercised-the-envelope-recipe-a-second-time-and-caught-a-no-op-compact.md)) —
so nothing failed; the cost is only visible by reading the read path and asking who pays.

**The requirement it drove.** None new — this sharpens the reading of an existing one.
`spec/capabilities/memory-and-resource-model.md` #Sharing Is Not Observable requires that whether
storage is shared or deferred never be *observable*, and its deferred-materialization clause licenses
the rope's flatten-on-read. This finding is the reminder that the requirement's spirit is a two-sided
promise: sharing must be invisible to a value's *meaning* (which the spec states and the corpus checks),
and — as an implementation obligation the spec cannot express as behavior — the mechanism enabling
sharing should not tax the value that never uses it, or the "transparent optimization" has an
untransparent floor. The concrete obligation lands on the runtime, not the spec: keep the leaf read
path a single dereference, so the rope stays a pure win for the roping program and a non-event for the
flat one. Recorded so the next time an *implicit* representation optimization (a CHAMP map, an RRB
relaxation, string interning) adds a classify-then-branch step to a hot accessor, the common-case
borrow is folded back in from the start rather than rediscovered by a cost review. Composes with
[a Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)
(the mechanism that introduced the second deref) and
[wiring the Bytes rope exercised the frozen-envelope recipe a second time](./2026-07-06-wiring-the-rope-exercised-the-envelope-recipe-a-second-time-and-caught-a-no-op-compact.md)
(the sibling finding that the value oracle cannot see resource- or cost-only regressions).

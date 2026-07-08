# A rejection family that looks like many checks is often one check at many positions — map the frontier by cluster to find the leverage

*2026-07-07*

**What happened.** ask-30 (the self-hosted compiler accepts ill-typed programs native rejects) has been falling
sub-family by sub-family as the compiler agent ports each rejection with a matching code: Bool over-reject fix →
bool-exhaustiveness CDZ0210 → out-of-range literal CDZ0201 → malformed-`let` + duplicate-field/key CDZ0201, agree
climbing 79 → 95 → 98 → 100 → 105. This cycle two more families landed (5 cases: malformed-`let` arity, duplicate
field/key), all already corpus-pinned, all verified — no gap for me to fill. The porting is outrunning my
gap-finding.

So instead of hunting the next individual bug, I mapped the whole remaining frontier: the 80 still-under-rejected
cases, bucketed by code and sub-clustered by message. The result was more useful than any single case would have
been. The 50 CDZ0201 under-rejects — half the entire frontier — sub-cluster into "comparison/ordering between
incompatible shapes" (10), "member access on a non-record" (5), "applying a non-function" (3), "tuple pattern
arity/shape mismatch" (4), "tuple access on a non-tuple" (2), "list/map elements don't share one type/shape" (8),
"over-applying a constructor" (2), "literal pattern type mismatch" (2)… and read together, **roughly 25 of the 50
are ONE underlying check — "an operand's shape doesn't fit the operation" — applied at many different syntactic
positions** (comparison operands, member-access targets, call heads, pattern scrutinees, collection elements). The
next cluster, 14 CDZ0301 "no silent numeric promotion," is largely the same provable-mismatch shape over
int/float operands. The capability codes (CDZ04xx, 5) are a genuinely separate routing concern; CDZ0210 (4) is
gated on ask-13's variant-count table.

**Why.** Two things.

First, the leverage insight: **a rejection family that presents as N distinct diagnostics is often one check
instantiated at N positions, and you can only see that by clustering the frontier, not by fixing cases one at a
time.** Case-by-case, "member access on a non-record," "applying a non-function," and "comparison between
different shapes" look like three separate features to build. Clustered, they are the same question — "does this
operand's shape fit what this position requires?" — asked at three syntactic sites, and they share the exact
`ck-of` / provable-mismatch machinery ask-53 already built for arith and comparison operands. So the port order
that minimizes work is not "hardest-first" or "corpus-order" but "biggest shared-mechanism cluster first": extend
the one shape-fits-position check to cover member-access, application, and pattern positions, and ~25 cases fall
together. The map is what reveals that the 50-case pile has a 25-case single lever in it.

Second, the loop's role shifts as the frontier matures. Early on, the highest-value loop output was finding the
next miscompile or decline. Once the sibling is porting rejection families faster than new gaps appear (and every
gap is already corpus-pinned), the highest-value output becomes the MAP — a structured, cluster-sized view of
what remains, so the port order is chosen by leverage rather than by whatever case surfaces next. A measurement
loop against a fast-moving implementation should notice when its comparative advantage moves from "detect" to
"characterize the remaining distribution," and switch — a frontier map delivered at the right moment saves more
work than another individually-found case. This is the "build a backlog" mandate at its most useful: not a list of
cases (the corpus already is that) but a clustering that exposes the shared mechanism under the surface variety.

**The requirement it drove.** No new corpus case — the 80 under-rejects are all already pinned (that is how the
byte gate counted them), and the 5 that landed this cycle were pre-pinned cases flipping under-reject → agree. The
output is the frontier map appended to ask-30 (the 80 remaining under-rejects by code and sub-cluster, with the
"~25 of 50 CDZ0201 are one shape-fits-position check" leverage read and the port-priority recommendation) and this
learning. WRONG=0 throughout — every under-reject is honest (compiles what native rejects), never a wrong value.
General lesson: **when an implementation is porting a rejection family faster than you can find gaps, stop hunting
individual cases and MAP the frontier by cluster — a family that looks like many checks is often one check at many
positions, and only the clustered view exposes the shared-mechanism lever that fixes a quarter of the pile at
once; a measurement loop's advantage shifts from "detect the next bug" to "characterize the remaining
distribution" as the frontier matures, and the map delivered then is worth more than another found case.**

# The reader's decode surface is complete — dispatch, iterate, and atom-decode are the three legs of a canonical-AST reader

*2026-07-07*

**What happened.** Working in the subset-growth mode the previous cycle identified
([[2026-07-07-self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset.md]]), the spike
rounded out the reader's **atom decode** and **variable-arity application** handling — completing the
surface a canonical-AST reader needs. `read-node` now dispatches by CBOR major type across the full
scalar surface: major 0 (unsigned int) → `NInt` directly, major 1 (negative int) → `NInt (-1 - arg)`
(CBOR's negint convention), major 7 (simple) → `NBool` (`0xF5`=true, `0xF4`=false). And `read-app`
decodes an application by head *and arity*: `if` (3 operands) → `NIf`, `not` (1 operand) → `NPrim
"not"`, any other head → a binary `NPrim` (2 operands). Verified: negint `0x29` → -10, bool `0xF5` → 1
/ `0xF4` → 0. So the reader now handles every leaf-atom form and the arity-varying node shapes the
compiler's own surface uses, not just binary applications over unsigned ints.

The durable observation is the **shape of a canonical-AST reader as three legs**, all now built and
gate-witnessed:
1. **Dispatch** — read a decoded scalar (a head index) and select an operation
   ([[2026-07-07-the-reader-is-wired-bytes-to-component-end-to-end.md]]).
2. **Iterate** — read an array length and loop by it (a module's def list, a call's arguments)
   ([[2026-07-07-the-whole-module-reader-is-wired-module-bytes-to-component.md]]).
3. **Atom-decode** — interpret each leaf scalar by its major type into the value it denotes (uint,
   negint, bool) — this cycle.

**Why.** These three legs are exhaustive over what a reader does — every byte of a canonical AST is
either a structural head the reader dispatches on, a count that drives iteration, or a leaf atom it
decodes to a value — so naming them as a closed set is a useful completeness check: a reader with all
three, over every CBOR major type its AST uses, can traverse any program in its surface. The reader
reaching this completeness by *accretion of small verified operations* (each major-type arm, each
arity case, added and pinned individually) is the same composition thesis the output side established
and the earlier reader work confirmed — there was no "reader algorithm" to get right, only a widening
set of decode arms each of which is a few lines over the shared `cbor-major`/`cbor-arg`/`skip-elems`
vocabulary. This is also the texture of subset-growth work now that the seed is unblocked: each cycle
widens the compiler's accepted surface by one construct (here, negint / bool literals and the
`if`/`not` arities), and the loop's job is to *pin the newly-accepted shape* rather than hunt a
miscompile — the coverage-measuring mode, not the defect-finding mode. The negint arm is worth one
specific note: CBOR encodes a negative integer `n` as the unsigned `-1 - n`, so a reader that read a
negint's argument as a plain uint would decode `9` where the source wrote `-10` — a silent literal
corruption, the kind of decode error that produces a valid component computing the wrong thing, which
is exactly why the atom-decode leg earns a known-answer corpus case rather than trust.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR atom decodes each
scalar major type to its value"* — pins the atom-decode leg: `dec` dispatches on major type and decodes
`0x29` (negint → -10), `0xF5` (bool → 1), and `0x0A` (uint → 10), summing to 1. It is deliberately a
known-answer over all three scalar majors at once, so a negint-as-uint slip (-10 read as 9) or a
bool-arg confusion changes the sum — the leaf-decode companion of the head-decode and length-iteration
cases. It **PASSES**, and with it the executable semantics now witnesses all three legs of a
canonical-AST reader (dispatch / iterate / atom-decode) over the `bytes → AST → typed-IR → component`
path. No new backlog item — this is subset-growth progress: the reader's decode surface is complete for
the scalar/application forms; the remaining subset frontier to self-inclusion is the compiler *emitting*
the richer constructs its own source uses (sum types, `match`, `String`, recursion), plus scale (TCO),
tracked as the standing self-hosting work rather than per-shape gaps.

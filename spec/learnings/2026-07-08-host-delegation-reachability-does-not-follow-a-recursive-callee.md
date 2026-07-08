# Host-delegation reachability does not follow a recursive callee

*2026-07-08*

**What happened.** Adversarial probing of the host-delegation surface found that an entrypoint's
`(host (E) …)` delegation is treated as NOT reaching an effect performed inside a recursive function it
calls. `(def (go n) (if (= n 0) unit (do (log.emit "x") (go (- n 1))))) (def (main) (host (log) (go 1)))`
is rejected CDZ0401 "`log.emit` is reached with neither an enclosing handler nor a host delegation" — a
FALSE rejection, since `main`'s `(host (log) …)` delegates `log` and reaches `log.emit` through `go`. The
recursion of the performing function is the sole trigger:
- `(host (log) (log.emit "x"))` (direct) → runs.
- `(host (log) (go))` for a NON-recursive `go` performing `log.emit`, and a two-level non-recursive chain
  → run.
- the intra-program-handler analog — a recursive `go` performing an effect discharged by an enclosing
  `handle` — runs (recursion-with-effects is realized; the corpus pins recursive-effect cases, all using
  `handle`).
- a recursive `go` that does NOT perform the effect (effect in `main` directly) → runs.
Only recursive-function-performs-host-delegated-effect is rejected, and regardless of where in the
recursive body the perform sits (base case or recursive step).

**Why it is a break.** capabilities-and-effects.md #An Entrypoint Delegates The Capabilities It Grants To
The Host: "The compiler MUST determine a program's required capabilities from the operations its
entrypoints actually REACH and delegate"; #The Authority An Entrypoint Reaches: "determined by the
operations reachable from its own body under its own delegations." Reachability follows the CALL GRAPH,
which includes recursive functions. So `log.emit`, reachable from `main`'s body (through `go`) under
`main`'s `(host (log) …)` delegation, IS granted, and the program must run. Rejecting it is a false
rejection of a valid, granted program.

**Root cause (likely) — the delegation-reachability walk does not traverse a recursive call edge.** The
analysis that decides whether a reached effect has a "home" (an enclosing handler or an entrypoint
delegation) walks the call graph from the entrypoint, but appears to stop at (or not recurse through) a
recursive function — so an effect performed only inside a recursive callee is seen as unreached-by-the-
delegation and classified "no home" (CDZ0401). The intra-program handler resolution walks the same
recursion correctly (the pinned recursive-effect cases run), so the gap is specific to the
host-delegation reachability path. The fix is to make the delegation-reachability walk follow every call
edge including recursive ones (with the usual visited-set to terminate), matching how the effect-row /
handler-resolution walk already handles recursion.

**The lesson (the recurring family — the master pattern).** A mechanism proven on one form (delegation
reachability through a non-recursive callee; handler resolution through a recursive callee) is not
carried to the sibling form (delegation reachability through a recursive callee). This is the master
pattern across the CALL-GRAPH-SHAPE dimension (non-recursive ↔ recursive) and the ROUTING-MECHANISM
dimension (intra-handler ↔ host-delegation): the two routing mechanisms must agree on reachability, and
both must follow recursion. The tell: the identical recursive `go` performing an effect runs when the
effect is discharged by an enclosing `handle` but is rejected when granted by a `host` delegation — the
routing mechanism, not the program's validity, decided the outcome. And it is a FALSE REJECTION, not a
miscompile — safe under decline-don't-miscompile, but a valid program the compiler must accept.

**Corpus case added.** `spec/semantics/04-capabilities.sexp` §"an entrypoint delegation reaches an effect
performed in a recursive callee" — `(host (log) (go 1))` for a recursive `go` performing `log.emit` MUST
run (output `unit`, one `log.emit` host call), the recursive-callee companion of the working
direct/non-recursive delegation cases. Gated `(needs effects)`, which the seed realizes, so the behavior
gate runs and catches it (expected output `unit`, observed a wrongly-rejected program). A generation that
does not yet follow a recursive call in delegation reachability must not reject a program the delegation
grants.

# A nested tuple pattern's arity is not checked — only the outermost

*2026-07-07*

**What happened.** Adversarial probing of pattern matching found a wrong-value miscompile at a
nested pattern position. `(match (tuple 1 (tuple 2 3)) ((tuple a (tuple b c d)) 9) (_ 0))` runs
to `0`. The arm pattern `(tuple a (tuple b c d))` has a nested three-element tuple pattern
`(tuple b c d)` at a position whose scrutinee element is the two-element tuple `(tuple 2 3)` — a
static shape mismatch that can never match. Instead of rejecting the arm, the compiler lets it
silently not-match and falls through to the `(_ 0)` wildcard. The identical mismatch at the
*top* level is correctly rejected: `(match (tuple 1 2) ((tuple a b c) a) (_ 0))` → CDZ0201.

**Why it is a break.** The corpus pins (02-binding-and-control.sexp §"a tuple pattern of the
wrong arity is a type error"): a tuple pattern of an arity the scrutinee cannot have is ill-typed
and "MUST reject … not silently fail." core-semantics.md #Patterns Compose requires the rule to
apply recursively: a tuple pattern "MUST admit any pattern in each of its binder positions … its
element MAY itself be … a tuple pattern, matched recursively to any depth." So a nested tuple
pattern's arity is checked against the corresponding nested scrutinee element exactly as the
top-level pattern's is. Silently failing the nested arm to a wildcard is the "silent non-match"
the flat cases forbid — a program that should be rejected runs and returns a value.

**Root cause — the arity check inspects only the outermost pattern.** In the seed
(`codegen.rs::check_type_rejections`, the tuple-scrutinee arm), the check resolves the scrutinee's
static shape, then `for arm in arms { if arm's pattern is (tuple …) && p.len()-1 != scrut_arity
{ reject } }`. It examines each arm's *outermost* pattern `p` and compares its element count to
the scrutinee arity — but never descends into `p`'s sub-patterns to check a nested tuple pattern
against the nested scrutinee element. So `(tuple a (tuple b c d))` vs `(tuple 1 (tuple 2 3))`:
the outer `(tuple a _)` has arity 2 = scrutinee arity 2, the check passes, and the nested
`(tuple b c d)` vs `(tuple 2 3)` is never seen. The fix is to walk the pattern and scrutinee
shape in lockstep, checking arity at every tuple position, not only the root.

**The lesson (a recurring shape this run).** This is the fifth adversarial break of the same
family: *a check that covers only part of its obligation.* Prior instances — an annotation
checked only the head constructor not the payload; a name resolved via the env only in value
position not head; a constructor arity guarded only the high end not the low; a tuple index
checked the operand kind not the index bound — and now a pattern arity checked only the outermost
tuple not the nested ones. The tell each time: a rule proven at the top / for the common shape,
never recursed / extended to the structurally identical inner case. When a type rule is stated
for a compound form, its enforcement must recurse wherever the form nests; a flat check over the
outermost node is not enforcement of a compositional rule.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a nested tuple pattern of
the wrong arity is a type error" — `(match (tuple 1 (tuple 2 3)) ((tuple a (tuple b c d)) 9) (_
0))` MUST reject CDZ0201, as the recursive companion to the flat wrong-arity cases. Native seed;
the behavior gate catches it (expected reject CDZ0201, observed a running component).

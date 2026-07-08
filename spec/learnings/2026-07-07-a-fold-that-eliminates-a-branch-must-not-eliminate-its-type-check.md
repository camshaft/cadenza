# A fold that eliminates a branch must not eliminate that branch's type-check — const-folding is value-preserving but not rejection-preserving

*2026-07-07*

**What happened.** A latent MISCOMPILE was found and fixed in the self-hosted compiler: `(if false (record (a 1))
7)` folded the constant condition `false` to the taken branch `7`, silently discarding the dead compound
then-branch — WITHOUT type-checking it. Native REJECTS this program (`conditional branches have different types`,
CDZ0201: a record then-branch and an Int64 else-branch don't unify), but the folding compiler accepted it and
emitted `7`. The fold optimized away the very branch that made the program ill-typed. Fixed by giving the check a
`KCompound`/`CKCompound` kind so a compound-vs-scalar branch pair is a provable mismatch → CDZ0201, checked
independently of the fold. I verified: native rejects both directions (`(if false (record (a 1)) 7)` and `(if true
7 (record (a 1)))`), the fixed compiler now agrees (rejects), and pinned the case as a corpus regression guard
(behavior gate +1).

**Why.** Constant-folding a conditional to its taken branch is a correctness-preserving transform for the
program's VALUE — `(if false A B)` and `B` compute the same result. But it is NOT correctness-preserving for the
program's REJECTION, because a conditional's well-formedness depends on BOTH branches type-checking and agreeing,
whether or not a branch is evaluated (the language rule: "an unevaluated branch cannot carry a deferred error").
So the fold `(if false A B) → B` preserves the value but DROPS the obligation to type-check `A` — and if `A` is
ill-typed, the fold has converted a program the compiler should REJECT into one it ACCEPTS. This is a
wrong-acceptance, the mirror of a wrong-value: the compiler emits running code for a program that has no defined
meaning. The general trap: **an optimization that eliminates a subterm also eliminates whatever CHECKS that
subterm would have triggered, and if those checks are load-bearing for the program's well-formedness, the
optimization is unsound even though it is value-preserving.** Folding, dead-code elimination, short-circuiting —
any transform that makes a subterm "not run" must still make it "get checked," because checking and running are
different obligations and only the latter is what the fold is licensed to skip.

The subtlety that made this instance hide: the SCALAR versions were already handled (`(if false 1 false)` →
CDZ0201 was pinned and worked), so "dead-branch type-checking" looked done. The COMPOUND branch slipped through
because folding a compound branch away is where the check is easiest to skip — the fold logic reaches for the
taken (scalar) branch and never forms the compound branch's kind at all, so there's nothing to compare. The
discriminator that exposes it is a compound-vs-scalar branch pair under a CONSTANT condition (so the fold fires)
where the dead branch is the COMPOUND one. That specific corner — const-condition × compound-dead-branch — is
what the corpus lacked and what now guards it. A test suite that covers "the dead-branch check works" with only
scalar branches is testing the check where it's hard to skip, not where it's easy to skip.

**The requirement it drove.** Corpus: "a conditional with a compound branch and a scalar branch is a type error
even when the compound branch is dead" — `(if false (record (a 1)) 7)` → CDZ0201 (behavior gate +1), the
compound-vs-scalar instance the scalar-vs-scalar dead-branch cases didn't exercise, pinning against the fold
skipping the compound branch's check. The output is this learning and the verified fix (native rejects both
directions; the fixed compiler agrees; 0 disagree holds). General lesson: **const-folding (and any transform that
eliminates a subterm) is value-preserving but NOT rejection-preserving — it drops the CHECKS the eliminated
subterm would have triggered, so a fold that removes an ill-typed dead branch silently accepts an ill-formed
program; type-check the whole form BEFORE/independently of folding, and test the check at the corner where the
fold is easiest to skip (here, a compound dead branch under a constant condition), not only where it's hard to
skip (scalar branches).**

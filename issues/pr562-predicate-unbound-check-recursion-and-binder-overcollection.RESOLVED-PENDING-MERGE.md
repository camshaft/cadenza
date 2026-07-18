# pr562 — compile.rs predicate unbound-name check: unguarded recursion + binder over-collection (2 Copilot)

Mirrored from GitHub PR #562 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/562 (6-MR publish batch)
File: `implementation/seed/crates/rcdzc/src/compile.rs` — `@requires`/`@ensures`/`@invariant`
predicate unbound-name checking.
Both VERIFIED against `git show trunk` — real code, not hallucinations.

## Comment 1 — id 3607501714 (compile.rs:913) — recursion reintroduces overflow risk
> `first_unbound_predicate_name` switched from an explicit stack to a recursive walk (`unbound_in`).
> This reintroduces a potential stack overflow for a deeply-nested predicate AST (the earlier comment
> explicitly called out avoiding overflow), and predicates are attacker-controlled input in the
> general compiler pipeline.

VERIFIED: `first_unbound_predicate_name` (913) calls `unbound_in` (950), which self-recurses on
`.`-member operand (953), match scrut/arms (960/971), let (991), and generic children (1001/1023)
with NO depth guard. Same class as the PR#556 lower.rs recursion issue (routed to v-inference):
severity hinges on whether the predicate AST can be deeply-nested enough to overflow (a nested-parens
predicate could). "Attacker-controlled" is overstated for a compiler, but deep-nesting → stack
overflow (a compiler crash, not a clean CDZ reject) is the real concern. Fix = depth cap → clean
reject, or restore the explicit stack the earlier code used.

## Comment 2 — id 3607501720 (compile.rs:947) — pattern_binder_names over-collects, masks unbound errors
> `pattern_binder_names` currently treats *every* `Name` leaf as a binder (including pattern heads
> like `list`/`tuple`, separators like `..`/`_`, and even qualified-name segments when they appear as
> atoms). That can incorrectly add non-binders to `bound`, which can mask genuine unbound-name errors
> in the predicate body (false negatives). There is already a well-scoped binder-leaf collector in
> `resolve::arm_pattern_binders` that skips `.` member forms and ignores `_`/`..`.

VERIFIED: `pattern_binder_names` (compile.rs:947) pushes any bare `db.ast.as_name(pat)` (only skips a
`(. …)` list-HEAD) — it does NOT skip `_` or `..`. By contrast `resolve::collect_arm_binder_leaves`
(resolve.rs, under `arm_pattern_binders` at 1235) explicitly `n != "_" && n != ".."` and handles
`.`-member whole-patterns. So the predicate checker over-collects `_`/`..`/head names into `bound`,
which can SILENCE a genuine unbound-name in a predicate body (false negative — a real bug slips the
@requires/@ensures/@invariant gate). Fix = reuse `resolve::arm_pattern_binders` (Copilot names it) or
mirror its `_`/`..`/member skips.

## Owner
Predicate name-binding for program-conditions/@invariant = v-inference territory (owns infer/unify/
resolve; already took the parallel PR#556 lower.rs recursion issue). Filing to PM to route.

---
ROUTED to v-inference (corpus-bugfix 2026-07-18, both grepped-real). Predicate unbound-name checking in
compile.rs: (1) :913 first_unbound_predicate_name -> :950 unbound_in self-recurses (member/match/let/children)
with NO depth guard -> deep predicate AST = stack overflow (crash not reject). Same class as PR#556 lower.rs
(v-inference DISMISSED, arena acyclic by construction) -> likely also dismiss-with-rationale IF predicate ASTs
are the same append-only arena nodes; confirm. (2) :929 pattern_binder_names OVER-collects: pushes any bare
as_name, skips only (. head), NOT _ or .. -> masks genuine unbound-name errors (false negative, real unbound
name slips the predicate gate). FIX (Copilot-named): use resolve::arm_pattern_binders (resolve.rs:1235, skips
_/../member) which compile.rs already calls at :4964. (1) = likely dismiss-if-acyclic; (2) = real reject-gap to fix.

---
RESOLVED-PENDING-MERGE (v-inference, 2026-07-18, MR 6bef46f34):
(1) unbound_in recursion "overflow" -> DISMISSED: same arena-acyclic invariant as PR#556 — recursion
    descends only into strictly-smaller child arena ids (append-only Arenas::push, quote uses same builder),
    so a predicate AST is a finite acyclic subtree; walk bottoms out. Predicate ASTs confirmed same arena
    nodes. Rationale comment added; depth cap would be cosmetic.
(2) pattern_binder_names over-collection -> FIXED: swapped the local walk (pushed every bare as_name, skipped
    only (. ) head) for resolve::arm_pattern_binders (canonical, skips _/../.-member, binds real leaves only).
    HONEST CAVEAT (v-inference verified): NO live false-negative witness — the over-collected tokens
    (_/../list-tuple heads/ctor names) are all unreferenceable or resolve as builtins/ctors, so the masking
    is NOT reachable in today's grammar. Lands as CORRECTNESS/CONSISTENCY hardening (removes a latent
    divergence a future pattern form could make reachable), not a demonstrable-bug fix. Test extended
    (list-rest arm: binds leaf+rest, stray name still CDZ0101). 2096/2096 pass. Retire on land.

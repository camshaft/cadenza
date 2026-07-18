# pr556 — lower.rs: two AST recursions without cycle detection (2 amazon-q comments)

Mirrored from GitHub PR #556 review comments (amazon-q-developer[bot]).
PR: https://github.com/camshaft/cadenza/pull/556 (13-MR publish batch)
File: `implementation/seed/crates/rcdzc/src/lower.rs`

VERIFIED against `git show trunk:...lower.rs`: unlike amazon-q's recent cdz-smith flags, these
functions REALLY EXIST and REALLY recurse — so NOT a hallucination. Worth the compiler owner
judging whether the arena AST can actually be cyclic (if it's acyclic by construction, dismiss;
if untrusted/constructed AST can cycle, a depth guard is warranted).

## Comment 1 — id 3607323125 (lower.rs:11356) — if_or_match_selects_lambda / arm_selects
> :stop_sign: Logic Error: The recursive call `if_or_match_selects_lambda(db, body)` in the lambda
> `arm_selects` can cause infinite recursion if there's a cyclic reference in the AST. The function
> should track visited nodes or have a depth limit to prevent stack overflow.

Confirmed present: `fn if_or_match_selects_lambda` at 11353; `arm_selects` closure at 11355 calls
`... || if_or_match_selects_lambda(db, body)` at 11356; recurses on then/else arms (11359/11378).

## Comment 2 — id 3607323128 (lower.rs:18532) — const_list_elems following LocalRef
> :stop_sign: Logic Error: The `const_list_elems` function follows `LocalRef` binders without
> checking for cycles. If there's a cyclic binding chain (A → B → A), this will cause infinite
> recursion and stack overflow. Add cycle detection or a recursion depth limit.

Confirmed present: `fn const_list_elems` at 18527; also called self-recursively at 18552.

## Triage
Both are real recursive AST walks. The soundness of the concern hinges on a Cadenza invariant:
is the arena AST guaranteed acyclic (StructIds only reference earlier-constructed nodes)? If yes,
these can't loop and it's a dismiss-with-rationale. If a LocalRef binding chain or match/if nesting
can form a cycle (esp. via metaprogramming/constructed AST), a depth/visited guard is warranted.
Compiler owner (rcdzc / compiler-ml) call — I can't assert the invariant myself.

---
ROUTED to v-inference (corpus-bugfix 2026-07-18): REAL code (github-liaison grepped trunk to confirm, NOT
an amazon-q hallucination). 2 recursive lower.rs AST walks without cycle detection: (1) if_or_match_selects_lambda
line 11353 (the fn fix-if-joined-capturing-lambdas just landed, 723e42d12) recurses on if/match arm bodies;
(2) const_list_elems line 18527 follows LocalRef binders + self-recurses. SEVERITY hinges on an invariant only
the owner can assert: is the arena AST guaranteed ACYCLIC (StructIds ref only earlier nodes, no LocalRef or
constructed-AST cycle)? YES -> dismiss-with-rationale (terminates by construction); NO -> depth guard
(decline-not-hang). Not a confirmed bug, not spawning. v-inference owns the arena invariant + lower.rs.

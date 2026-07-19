# pr625 — compiler-ml PORT: (1) sread sentinel doc mismatch [DOC] + (2) resolve-db NApp caller-env dynamic-scope [SEMANTICS]

Mirrored from GitHub PR #625 review comments (Copilot), via github-liaison 2026-07-19. Both grepped-real,
verified against `git show trunk` (b2fa31db5) by corpus-bugfix.

## #1 — sread.cdz:242 [DOC / code-shape — v-compiler-ml] — TRIVIAL
read-def-form's doc says `returns (nameId, bodyId, nextIndex, tree')` and `nameId = -1 sentinel = a
PARAMETERIZED def`. But read-def-body (sread.cdz:250-252) returns `(name-id(nm), 0-1, k, tree)` for the
parameterized case — the `-1` sits in the **bodyId** position, NOT nameId (nameId is ALWAYS `name-id(nm)`).
So the doc mislabels which field carries the sentinel. Doc-only fix: say "bodyId = -1 sentinel", not "nameId".

## #2 — resolve-db.cdz:51 [SEMANTICS — v-compiler-ml owns port src; v-inference may advise scoping] — LATENT
The NApp arm resolves the callee body with the CALLER's env:
    | Option.Some(Node.NApp(calleeId, _)) => resolve-node(tree, calleeId, env, col)
while its own comment (54-55) says "a helper's body sees no caller bindings — first-order." Code contradicts
intent = a latent DYNAMIC-SCOPE capture: a helper body name colliding with a caller `let` binding (NLet
inserts into `env`, resolve-db.cdz:44) would bind to the CALLER's node, not resolve free/to its own def.
Copilot fix: resolve the callee body under `Map.empty` (a fresh scope), matching lexical/first-order intent.
REACHABLE TODAY? Unclear — the NApp arm fires for slice-3b-ii (a nullary call as module root), and the
current subset is first-order/nullary; whether a helper body + a caller let-of-the-same-name is constructible
in today's grammar needs owner judgment. Even if not reachable yet, the code/comment contradiction is a
latent-bug trap as the subset grows.

## Routing (corpus-bugfix 2026-07-19)
Both in `implementation/compiler-ml/*` PORT source = v-compiler-ml (liaison-routing rule: compiler-ml source
is the PORT owner; v-inference owns only rcdzc infer/unify/resolve). ROUTED to v-compiler-ml, split as the
liaison asked: #1 trivial doc fix; #2 a scoping-semantics call (fix to Map.empty + judge reachability) — CC
v-inference as a scoping-semantics advisor (they own the rcdzc resolve twin, so they can rule whether the port
should match rcdzc's lexical resolve). VERIFIED loci on trunk b2fa31db5.

---
## v-inference ADVISORY RULING (2026-07-19) — #2 confirmed: port's Map.empty fix is FAITHFUL
v-inference (rcdzc resolve-twin owner) ruled: **rcdzc resolve is LEXICAL** — every name resolves by walking
PARENTS from its own occurrence (resolve.rs resolve_name:558 "nearest enclosing binder", :562 "Scope-FIRST is
what makes binding lexical", cited to core-semantics.md#binding-is-lexical). A callee body's names resolve
against the callee body's OWN lexical position; a helper body NEVER sees a caller NLet binding. The caller's
ARGUMENTS resolve in the caller's scope + splice in via beta-reduction (resolve_subtree:198-205), but that's
the arg subtree, not the body's free names. So the port comment "a helper's body sees no caller bindings —
first-order" is CORRECT, and `resolve-node(tree, calleeId, env, col)` passing the caller env is the latent
dynamic-scope defect. **Copilot's Map.empty (fresh scope) restores lexical/first-order semantics = the faithful
port match.** NO rcdzc change needed (already lexical) — purely a port-faithfulness fix for v-compiler-ml.
v-inference routed the ruling directly to v-compiler-ml too. SEMANTICS QUESTION RESOLVED; awaiting v-compiler-ml's
fix (doc #1 + Map.empty #2). corpus-bugfix tracks to close on land.

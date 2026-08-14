# GAP-6 — typed `git/*` effect family (v-agent-harness) — ❌ SUPERSEDED / REVERSED

> ❌❌ **SUPERSEDED (operator ruling, 2026-08-14) — the entire typed-kernel-git-family design below is
> REVERSED and was fully collapsed.** Operator directive, verbatim: "Why are we adding a kernel space git
> executor? That should be a userspace wrapper around shell so it can easily evolve without redeployments."
> The NEW direction: **git is USERSPACE** — a reducer-side wrapper that composes the ALREADY-EXISTING generic
> `shell` effect (GAP-2), NOT a typed kernel `git/*` family and NOT a host `GitExecutor`. WHY: a userspace
> shell-wrapper evolves by editing reducer code (no host/kernel redeploy) and bakes NO git vocabulary into the
> kernel/host (generic-compiler-aligned). What was built under this doc + then FULLY COLLAPSED: the kernel
> `git/*` family decl (`40ebf3e08`) + per-op Cedar authz (`e66ad882c`) + dispatch-wire (`f022038b2`) were
> REMOVED (C1 kernel removal + C2 reducer removal, landed on origin); v-ah-host removed the host `GitExecutor`
> + `live-git` feature. This mooted the GitExecutor flag-injection finding (`3ff858c10`), resolved-by-removal.
> REPLACEMENT PATH (in flight): git ops = reducer functions emitting the `shell` effect, whose structured
> `(shell-pipeline (stage (program …) (args …)))` payload a reducer builds via a prelude `Shell.pipeline`
> builder (v-inference-owned; encoder-gated on exposing `ast-encode` OR R2 `Value.encode`). The role-library
> git-publish step is DEFERRED until `Shell.pipeline` lands, then re-points. This doc is retained only as the
> historical record of the reversed typed-family design — DO NOT build from it.
>
> ~~STATUS: DESIGN, boundary CONFIRMED by v-ah-host (concierge relay, 2026-08-13) — mirrors fs/* + the 14 host
> executors + the schema-hash re-key. Phase-1a reify e2e LANDED on origin (d311398f8); the A/B framing is now
> RESOLVED to (A) built-in well-known (see Schema-hash-model fit). Build still DESIGN-ONLY / HELD: git/* is a
> host-executor family, so its BUILD touches the effect-family/schema surface + `cdz-kernel/src` near the
> phase-2 dispatched-frame flip (2b/2c) — do NOT start it until the phase-2 squash lands, to avoid colliding
> with v-effects' in-flight assembly. This doc is the design record (docs-only, no gated-crate impact).~~

## The problem (GAP-6)
A self-hosting agent replaces fleet-tooling, which drives git (`sync` = reset-onto-trunk + patch-id replay,
commit, etc.). Today an agent does git via the GAP-2 `shell` effect (`shell` running `git …`), gated by a
COARSE Cedar command allow-list (`permit(action=="shell") when resource.target like "git *"`). GAP-6 adds a
TYPED `git/*` effect family so git operations are FIRST-CLASS, PER-OP authz-gated (push to which repo/branch?
commit where?), and gate-able — the same safety upgrade `fs/*` gave over `shell` for file edits.

## Relationship to GAP-2 shell (composes; a typed refinement, NOT overlap)
- `shell` = GENERIC exec: any program, COARSE authz (one Cedar rule over the whole command string).
- `git/*` = TYPED per-op effects: each op is a distinct schema-hash identity + resource (repo/branch/path),
  so Cedar gates FINELY (`permit(action=="git/push") when resource.repo=="camshaft/cadenza" and
  resource.branch like "fleet/**"`). Exactly the `fs/*`-over-`shell` precedent (`effect.rs:210-218`: fs is
  "executor-routed through the normal authorize→executor path … Cedar `permit(action=="fs/write") when
  resource.target like "implementation/**"`, NOT a host allow-list").
- They COMPOSE: an agent may still use `shell` for arbitrary commands; `git/*` is the typed, tightly-gated
  path for the common git ops. Not overlapping — distinct effect families.

## The `git/*` family shape (mirrors `fs/*`)
A representative MINIMAL v0 op set (like `fs/*` started read/write/glob) — the ops the self-hosting fleet loop
needs. Each op: TARGET = the resource (repo/worktree/ref, a Cedar-gated resource), PAYLOAD = op args,
RESULT = the op outcome bytes.
- `git/status`  — working-tree state (target=worktree → status bytes).
- `git/diff`    — changes (target=worktree/ref → diff bytes).
- `git/add`     — stage paths (target=worktree, payload=pathspec → unit).
- `git/commit`  — record (target=worktree, payload=message → commit sha).
- `git/rev-parse` — resolve a ref (target=worktree, payload=ref → sha; e.g. HEAD for an MR ref).
- `git/checkout` — switch/create branch (target=worktree, payload=ref → unit).
- `git/fetch`   — sync remote refs (target=remote → unit).
- `git/push`    — publish (target=repo, payload=refspec → unit) — the most authz-sensitive op.
(DEFERRED: `gh`/PR ops — the FLEET's "MR" is a `fleet send`, not a gh PR, so a `gh/*` family is out of scope
for the self-hosting loop; add later if a general agent needs GitHub PRs. Merge/rebase deferred — the fleet's
`sync` is reset+patch-id replay, host-side, not a guest git op.)

## Kernel-seam vs host-executor SPLIT (the non-overlap question — mirrors GAP-4's D3)
Same split as `fs/*` (a host-executor family the KERNEL declares the contract for):
- **KERNEL (v-agent-harness, MINE):** the `git/*` family CONSTANTS + each op's SCHEMA (param/result shapes →
  schema-hash identity, exactly like the built-in/`fs/*` schemas in `ast_marshal`) + the value-form WIRE
  (encode/decode of target/payload/result). This is the effect CONTRACT — the reducer↔host wire shape — which
  is kernel-seam (as the kernel owns the codec even for host-served families).
  ROUTING: the kernel routes each perform by schema-hash to the registered executor (post-phase-2 the
  `by_schema_hash` dispatch), exactly like every other family — no `git` branch anywhere.
- **HOST (v-ah-host) — CONFIRMED theirs:** a thin `GitExecutor` (`git_exec.rs`) = a **`ShellExecutor` typed
  sibling**: CWE-78-safe DIRECT exec (program+args, NO `sh -c`), map the `EffectRequest` target/payload → git
  args, marshal to `EffectOutcome::Ok`, and CLASSIFY errors (network = RETRYABLE; merge-conflict / bad-ref /
  nonzero = PERMANENT), registered per-verb by its schema-hash, feature-gated (`live-git`). Builds host-side
  when my decl/routing lands (same build-order as the phase-3 executor re-key). THIN MECHANISM — decides nothing.
- **Cedar (policy, wasm):** per-op authz predicates over resource=repo/branch/path — the authz-granularity
  value-add, NOT a host allow-list (operator standing order: policy in wasm, host is thin mechanism).
- ✅ BOUNDARY CONFIRMED (v-ah-host, mirrors fs/*): KERNEL owns family/schema-hash + perform/routing + wire
  CONTRACT; HOST owns the `GitExecutor` impl. Same shape as fs/* (I declared family+schema+wire; they served
  the executor) and as the 14 executors + schema-hash re-key.

### v-ah-host's 3 design flags (folded in — all consistent with standing orders)
1. **GENERIC-COMPILER (no hard-coded capability):** `git/*` MUST be a generically-DECLARED effect family via
   the SAME decl→schema-hash mechanism as any family (http/shell/blob) — NEVER a kernel/compiler hard-coded
   `git` special-case. git is just an instance; rcdzc/kernel stay git-AGNOSTIC (they hash whatever schema is
   declared, with no `git` branch). This holds under the RESOLVED framing (A) below — a kernel WELL-KNOWN
   built-in family still uses the generic schema mechanism (its schemas are baked like fs/emit/kv, hashed by
   the same machinery), it is NOT a compiler special-case; rcdzc stays git-agnostic.
2. **POLICY vs SCHEMA:** the typed family adds SCHEMA + authz-GRANULARITY (a grant can permit `git/clone` but
   deny `git/push` — per-VERB) — it does NOT add host policy. WHICH repos/remotes/refs a session may touch
   stays CEDAR on the resolved TARGET (same posture as shell's WHICH-commands). The host `GitExecutor` decides
   nothing.
3. **DRY with shell:** `GitExecutor` = `ShellExecutor` SPECIALIZED to git with typed verbs — SHARE the
   CWE-78-safe spawn helper (do NOT duplicate), riding the DONE GAP-2 shell exec. (Host-side, but pinned here.)

## Schema-hash-model fit (no hardcoded capability) — RESOLVED to (A) built-in well-known
Under the schema-hash identity model, a `git/*` op's IDENTITY is the schema-hash of its declared op-sig. The
compiler hard-codes NOTHING about git (the GENERIC-COMPILER rule holds); the built-in/userspace split lives in
the KERNEL's family vocabulary + the host executor registry, NOT in rcdzc.

🔑 RESOLVED to (A) — `git/*` is a KERNEL WELL-KNOWN family (like `fs/*`/`emit`/`kv`/`http`), served by a host
`GitExecutor`, with each op carrying an intrinsic `builtin_effect_schema_hash` that `EffectRequest::new_with_
family` populates from the family (effect.rs). NOT framing (B) userspace-declared-and-reified. WHY (the decisive
rule, learned from the schema-hash phase-2 emit case): an effect that has a HOST EXECUTOR is BUILT-IN — it keeps
its well-known family on the INPUT wire and its schema-hash is intrinsic to its construction; it must NOT be
redeclared as a userspace `effect Git` that REIFIES to `effect/Git`, because a reified `effect/<name>` identity
does NOT match an executor keyed on the built-in family (exactly the collision that broke a userspace-`effect
Emit` migration: EmitExecutor keys on family `emit`, so `effect/Emit` would not route). rcdzc reify is for
GENUINELY-userspace, executor-LESS effects (observe-and-drop); an executor-backed capability like git is
built-in. So (B) is DISPROVEN for git — it has a `GitExecutor`. The executor is host-side; authz is Cedar.

TWO-WIRE placement (mirrors the phase-2 model): a reducer performs a built-in `git/*` op on the INPUT wire
(family-string, kernel derives the intrinsic schema-hash at parse via `new_with_family`) — this is the
phase-3 input-typed-receive surface; the DISPATCHED-frame identity is the intrinsic `git/*` schema-hash. No
reify, no userspace `effect Git` decl in the reducer.

## Increment plan (post-flip; [K]=kernel/mine, [H]=host/v-ah-host)
1. **[K]** the `git/*` family constants + op SCHEMAS (param/result shapes) + schema-hash identities +
   value-form wire (encode/decode target/payload/result). Behavior-neutral (no executor yet). Mirrors fs/*.
2. **[H]** the thin `GitExecutor` (`git_exec.rs`, `live-git` feature) — injection-safe direct-exec,
   supervision-classified outcomes; `LiveExecutorSet` wiring. Mirrors fs_exec/shell.
3. **Cedar** per-op git authz predicates (resource=repo/branch/path); a granted-capability shape for git ops.
4. A reference reducer performing built-in `git/*` ops (via the well-known family on the input wire, like a
   reducer emitting built-in `emit`/`kv` — NOT a userspace reify) as an e2e, proving the fleet-loop git subset
   (status→add→commit→rev-parse) routes to the real `GitExecutor` by the intrinsic `git/*` schema-hash.

## Not doing / deferred
- `gh`/PR ops + merge/rebase: the fleet's MR is a `fleet send`, its sync is host-side reset+patch-id — out of
  scope for the self-hosting loop v0.
- The build: HELD until phase-1a lands (a typed git effect RIDES the schema-hash routing; and this touches the
  effect-family/schema surface near the phase-2 flip). The 2a rider preempts the instant phase-1a is on origin.

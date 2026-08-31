# Design — cost-tiered optimization levels for rcdzc (a Core-IR pass manager)

**Author:** v-core-opt (backend-independent optimization vertical). **Audience:** whoever adds an
optimization pass to rcdzc, and v-cdz-tooling (owns how a level is *requested*).
**Status:** **DESIGN — nothing landed yet (2026-07-15).** This states the mechanism and the taxonomy
so passes are tiered *by construction* as they accumulate, before there are 15 untiered ones. Line
numbers are landmarks at this commit, not promises they won't drift. **Taxonomy DECIDED by the
operator (2026-07-15): O0/O1/O2/O3 (Rust-style), default O1** (see §6) — the design below already
matches this.

---

## 1. TL;DR — the win, the seam, the one rule

**The win (operator directive, 2026-07-15).** Some optimizations are expensive. A scripting / dev
iteration should compile FAST (skip expensive passes); a long-lived / release program should spend
more time up front. Cadenza has **no opt-level surface today** — no `-O` flag, no build profile, and
the existing passes run unconditionally. This is greenfield: design the level mechanism as a
first-class part of the pass framework now.

**The seam.** rcdzc's pipeline is target-neutral above `select::select_module` and one backend below
it (see `DESIGN-backend-retargeting.md`). Optimizations split two ways today:
- **Fold-tier, in `lower.rs`** — constant folding, copy-propagation, admin-redex elimination, DCE of
  unused bindings, algebraic identities (`fold_arith`, `should_keep_binding`). These run as `core_of`
  is demanded; they are cheap and largely *canonicalization* (they also make the IR well-formed).
- **Backend-tier, in `backend/wasm/select.rs`** — LICM (`licm_invariant` ~3014), dominator CSE
  (`collect_cse_candidate_groups` ~2562), accumulator introduction (`accum.rs`), slot reuse. These are
  the expensive whole-function analyses — and (the v-core-opt gap) they live in the WASM backend, so
  the Rust backend inherits none of them.

**The one rule (correctness bar).** Every level MUST produce **observably-identical behavior** — only
speed/size differ, never semantics. A higher `-O` that changes a result is a miscompile. This is
gate-enforced (§5): a program computes the same value at every level, on every backend.

---

## 2. The taxonomy — a small enum, Rust's `-O` as the mental model

```rust
/// The requested optimization level. Higher = more compile time spent, same observable behavior.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum OptLevel {
    /// -O0 — canonicalization only. The MINIMUM to emit a correct, well-formed artifact:
    /// const-fold, admin-redex elimination, trivial DCE. Fast-by-construction; the dev/scripting path.
    O0,
    /// -O1 — cheap local cleanups on top of O0: copy-propagation, algebraic identities, local CSE,
    /// unreachable-arm removal. Still per-node / per-region, no whole-function dataflow.
    #[default]
    O1,
    /// -O2 — whole-function analyses: LICM, global (dominator) CSE, accumulator introduction,
    /// non-trivial inlining. The default for a RELEASE build.
    O2,
    /// -O3 — aggressive / speculative: whole-program inlining, cross-function specialization, passes
    /// whose cost is superlinear or whose payoff is workload-dependent.
    O3,
}
```

Rationale for the split points:
- **O0 is not "no passes"** — it is exactly the canonicalizations that make the IR *correct* (ANF
  admin-redex elimination, const-fold of what the type system already reduced). Skipping these would
  mis-emit, not just under-optimize. So O0 = "the cheapest CORRECT emit."
- **O1 vs O2 is the cheap/expensive line** — O1 stays local (bounded per-node/per-region work); O2 is
  where a pass may walk the whole function (LICM's invariance analysis, dominator CSE). This is the
  line the operator cares about: dev stays on the cheap side by default.
- **Default = O1**, not O0 or O2 — a sensible fast-ish default: the cheap cleanups are nearly free and
  meaningfully improve the common case, but no whole-function analysis runs unless asked. (Open product
  call, §6 — the operator may prefer O0-default for max dev speed, or a dev/release two-point split.)

The enum is `Ord` so a pass declares a *minimum* level and the manager runs it iff `requested >= min`.

---

## 3. The pass manager — tier-by-construction

Each pass declares its tier; the manager runs the passes whose `min_level <= requested`. Adding a pass
without a tier is a compile error (the trait method is required), so the fast path stays fast as passes
accumulate — the whole point of doing this now.

```rust
pub trait CorePass {
    /// The LOWEST OptLevel at which this pass runs. O0 = always-on canonicalization.
    fn min_level(&self) -> OptLevel;
    fn name(&self) -> &'static str;
    /// Transform the Core column in place (or produce a rewritten column). MUST be
    /// behavior-preserving at every level (§5).
    fn run(&self, db: &mut Db);
}

pub struct PassManager { passes: Vec<Box<dyn CorePass>>, level: OptLevel }
impl PassManager {
    pub fn for_level(level: OptLevel) -> Self { /* register all, in canonical order */ }
    pub fn run_all(&self, db: &mut Db) {
        for p in &self.passes {
            if self.level >= p.min_level() { p.run(db); }
        }
    }
}
```

**Where it plugs in.** The manager runs on the **Core column, above the backend split** — that is the
v-core-opt territory and the reason a Core pass benefits both backends. Concretely: after `lower` has
filled the core column and before `select_module` (wasm) / the Rust emitter consume it. The existing
fold-tier work in `lower.rs` is *already* O0/O1-shaped (it runs as `core_of` is demanded); the
migration (§4) is to (a) give it an explicit tier, and (b) LIFT the backend-tier analyses out of
`backend/wasm/` into O2 Core passes so Rust inherits them.

**Non-goal for the manager:** it does not re-implement the passes, only *sequences and gates* them. A
pass is a `CorePass`; the manager is the tier policy.

---

## 4. Migration path (increment backlog — one gated slice per tick)

The framework lands incrementally; passes move under it one at a time, each with a gate proving
behavior is unchanged at every level.

1. **Land `OptLevel` + `PassManager` skeleton** with the existing fold-tier declared O0/O1. Thread a
   default level through `compile()` (no surface yet — hardcode default). Gate: full corpus unchanged.
2. **Wire the level to `compile()`'s options** (still no CLI surface — an internal parameter). Gate: a
   sweep case that runs a representative program at O0/O1/O2/O3 and asserts the SAME value at each.
3. **Lift LICM to a Core O2 pass** — coordinate with v-wasm-opt (it owns the wasm-backend LICM today).
   Agree what lifts vs stays wasm-specific. Verify the Rust backend now inherits it (`gate --target rust`).
4. **Lift dominator CSE / accumulator-intro** similarly (later slices, each coordinated).
5. **Expose the level at the surface** — coordinate with v-cdz-tooling (§6): a `cdz compile --opt-level`
   flag and/or a `Project.cdz` manifest `dev`/`release` profile. v-cdz-tooling owns the CLI/manifest;
   v-core-opt owns the level→passes mapping the flag selects.

Ordering rule: never yank a pass out from under v-wasm-opt — `note` first, agree the migration, then
lift. Some wasm CSE/LICM may be genuinely wasm-specific and STAY in the backend; only the
backend-agnostic core of a pass lifts.

---

## 5. Correctness — the level-equivalence gate (a NEW kind of gate coverage)

The invariant "every level is observably identical" is not expressible by a single-run corpus case, so
v-core-opt adds a gate mode for it:

- **Per-case, multi-level:** the gate compiles a representative subset (or all) corpus programs at each
  `OptLevel` and asserts the recorded output/trap is reproduced at EVERY level, on BOTH backends. A
  level that changes a result is a miscompile — a hard fail, not a todo.
- Wire it into `cargo xtask check` so it runs for the whole fleet (structurally prevents a peer from
  landing a pass that mis-optimizes at a higher level).
- Cheapest first cut: reuse the existing corpus + a `--opt-level` sweep flag on `xtask gate` (analogous
  to `--target rust`). Then a single `xtask gate --opt-sweep` runs O0..O3 and diffs.

Until the sweep exists, each lifted pass lands with a corpus case that would compute a WRONG value if
the pass mis-fired, run at the level that enables the pass.

---

## 6. Product decisions — DECIDED (operator, via concierge, 2026-07-15)

- **How many levels + names — DECIDED: O0/O1/O2/O3** (Rust-style, 4 granular levels, familiar). Tiering
  as in §2: O0 = canonicalization only (max dev speed); O1 = + cheap local cleanups; O2 = + whole-function
  passes (inlining, global CSE, LICM); O3 = + aggressive / whole-program.
- **The default — DECIDED: O1** (cheap canonicalizations + cheap cleanups ON, no whole-function/
  whole-program analysis — good dev speed with meaningful wins).
- **The request surface — coordinate with v-cdz-tooling** (its territory): a `cdz compile -O<n>` /
  `--opt-level` flag AND a `Project.cdz` dev/release profile that maps to a level (e.g. dev→O1,
  release→O2 or O3 — the exact mapping decided jointly with v-cdz-tooling). v-cdz-tooling owns the
  flag + manifest parsing; v-core-opt owns the `OptLevel` enum + level→passes mapping the surface selects.

Coordination: v-core-opt ↔ v-cdz-tooling on the surface; v-core-opt ↔ v-wasm-opt on which passes lift
to Core vs stay wasm-specific (v-wasm-opt confirmed 2026-07-15: its Lir/slot-level CSE, select-ification,
br_table, LICM, accum-intro, slot reuse, guard elision are wasm-Lir-specific and STAY; the common-ctor
build-once hoist/sink family is already backend-independent in `lower.rs`).

## 7. The dev/release → OptLevel mapping — CANONICAL (v-core-opt, 2026-07-15)

v-cdz-tooling asked for the authoritative profile→level mapping for `cdz build` + a `Project.cdz`
profile field. This section is that authority; `cdz build` and `rcdzc` follow it so they agree.

- **`cdz build` default (no flag, no manifest field) → `OptLevel::default()` = `O1`.** The dev/scripting
  common case: cheap cleanups on, no whole-function analysis. (Matches `compile`'s default wrapper.)
- **`cdz build --release` → `O2`.** Release spends the whole-function passes (inlining, global CSE,
  LICM) — the "release-default" the design has always named. NOT `O3`: `O3`'s aggressive/whole-program
  passes are opt-IN via an explicit `--opt-level O3`, not implied by `--release` (so `--release` stays a
  predictable, well-tested tier; a project wanting maximum spends `--opt-level O3` deliberately).
- **A `dev` alias → `O1`** (= the default). Provided so `--release`/`dev` read as a symmetric pair and a
  manifest can name `dev` explicitly; `dev` is just the name for the default tier, it adds no new level.

**Canonical alias table** (the two named profiles map onto the O-levels; the O-levels remain the ground
truth the `PassManager` gates on):

| profile / alias | OptLevel | meaning                                                        |
|-----------------|----------|----------------------------------------------------------------|
| `dev` (default) | `O1`     | cheap local cleanups; fast iteration                           |
| `release`       | `O2`     | + whole-function passes (inlining, global CSE, LICM)           |
| (explicit only) | `O0`     | canonicalization only — max dev speed, via `--opt-level O0`    |
| (explicit only) | `O3`     | + aggressive/whole-program — via `--opt-level O3`, never implied|

**Manifest field name — flat `opt-level`, not a `[profile.release]` block.** `Project.cdz` is flat
`def`s (name/entry/tests), not TOML, so a flat well-known field `def opt-level = "O2"` fits the manifest
shape and reuses the SAME `rcdzc::OptLevel::FromStr` the `--opt-level` flag parses (one spelling, one
parser, one mapping). A cargo-style `[profile.*]` block would import TOML sectioning the manifest does
not otherwise have. If per-profile granularity is later wanted, `def opt-level-release = "O3"` extends
flatly without a block. **Precedence:** `--opt-level` flag > manifest `opt-level` field > (`--release` ⇒
`O2`) > `OptLevel::default()` (`O1`). The flag always wins; `--release` and a manifest field that
disagree is a coordination detail for v-cdz-tooling (suggest: `--release` sets the baseline, an explicit
`--opt-level` still overrides).

**Behavior bar unchanged:** every level is observably identical (only speed/size differ). `cdz build
--release` must produce the same OBSERVABLE result as a default build for every program — the
level-equivalence guarantee the `every_opt_level_emits_byte_identical_wasm` test already pins, and a
future `--opt-sweep` gate would extend corpus-wide.

## 8. Current state — the framework is READY BUT EMPTY, and that is the honest finding (v-core-opt, 2026-07-16)

Everything except the passes themselves is landed and wired end-to-end:
- `OptLevel` (O0..O3, default O1) + `PassManager` (tier-gated) + `CorePass` trait — `opt.rs`.
- `compile_with_opt(inputs, targets, level)` runs the manager at the post-load Core seam.
- The full surface: `cdz build --release` (O2), `--opt-level <O0..O3>`, `Project.cdz def opt-level`,
  resolved by §7's precedence — landed by v-cdz-tooling.
- Level-equivalence guard: `every_opt_level_emits_byte_identical_wasm` unit test.

**But the `PassManager` registers ZERO passes, so no level changes emitted output today** — and an
audit (v-core-opt, ticks 17–21) found this is the CORRECT state, not incomplete work:
- The cheap backend-independent optimizations the design assigned to O0/O1 (constant folding, copy
  propagation, admin-redex elimination, algebraic identities, non-recursive-call inlining +
  monomorphization, `@inline-*` policy, the fold family) are ALREADY done eagerly in `lower.rs` as the
  demand-driven core column is built. They are not separable into level-gated passes without rearchitecting
  lowering to be non-lazy — and they should always run (they are canonicalization + the cheapest correct
  emit), so gating them off at O0 would only ever mis-emit or under-optimize with no dev-speed win worth
  the rearchitecture.
- The expensive whole-function optimizations the design assigned to O2 (LICM, dominator/global CSE,
  accumulator introduction) live in the WASM backend (`select.rs`) and are wasm-Lir/slot-specific
  (confirmed with v-wasm-opt). They do NOT lift to backend-independent Core passes, because the RUST
  backend delegates optimization to `rustc` — wasm needs its own only because it has no downstream
  optimizer. So there is no backend-agnostic O2 transform to register.

**Consequence:** the tiered-opt framework is valuable as READY infrastructure — the surface, the enum,
the precedence, the seam, and the level-equivalence guard all exist, so the moment a genuinely
backend-independent, level-worthy transform is identified it drops in as one `CorePass` registration
with a gate. But forcing a speculative pass now would add cost without a real optimization behind it.
The open question (escalated to the operator via the concierge): is "ready but empty" the intended
resting state, or is there a specific backend-independent O2 transform the operator wants built (e.g. a
Core-level global value-numbering that BOTH backends keep, accepting the redundancy with rustc's)? Until
that is answered, v-core-opt holds the framework here and continues hardening the fold family's gate
coverage (which is what actually protects the emitted-code quality both backends share).

## 9. OPERATOR MANDATE (via concierge, 2026-07-16): ADD MORE OPS, HIGHER IN THE PIPELINE + proof-guided elision

§8's open question is ANSWERED. The operator's direction (relayed by concierge):
1. **NOT "ready but empty" as a resting state — keep adding optimizations, and favor HIGHER-IN-PIPELINE
   ones.** Verbatim steer: *"the more optimizations we can do higher in the pipeline the better"* — i.e.
   Core-IR / pre-backend transforms that benefit BOTH backends are preferred over late backend-specific
   ones. This is precisely v-core-opt's charter, so it is the mandate now (not marginal corpus pins).
2. **Proof-guided elision (a v-core-opt × v-verification seam).** The verification vertical is building
   pre/post-condition machinery (operator just greenlit it). The directive: *"if we can prove an integer
   never overflows, elide the overflow checks entirely."* So a checked/guarded operation's check is the
   DEFAULT, and a discharged proof (from the verification layer) REMOVES it. Checked-arith is therefore not
   merely a runtime feature — it is the canonical proof-elidable check. **Do NOT build the full
   `Core::CheckedArith` node speculatively yet** — coordinate with v-verification as their pre/post design
   firms up — but SHAPE any checked/bounds/guard op so a proof obligation can be attached and discharged.

### 9a. The core-override/rewrite seam — ✅ LANDED
`CorePass::run(&mut Db)` transforms the core column in place, but `core_of` (`lower::core_of`) is a
DEMAND-DRIVEN MEMOIZED query, not a mutable store — so a pass needs a core-override layer that `core_of`
consults. **This seam is now IMPLEMENTED** (this section previously described it as the not-yet-done first
slice; it is done — do NOT re-implement):
- `Db::core_override: FxHashMap<StructId, Core>` (`db.rs`) holds pass-installed overrides, keyed by the same
  `StructId` space `core_of` uses.
- `Db::install_core_override(id, core)` / `Db::has_core_overrides()` are the write + probe API a pass calls.
- `core_of` (`lower.rs`, top of the fn) returns the override if `!db.core_override.is_empty()` and one is
  present for `id`, else computes as before — so an un-overridden node is byte-identical to pre-seam.
- `PassManager::run` (`opt.rs`, driven at `compile.rs`) populates the overrides by running each enabled pass.
Remaining design constraints to respect when adding passes: the override must be visited-consistently by the
poison/escape walks (compile.rs VISITED-set), and installing an override must NOT break incremental
re-lowering (overrides clear on input change).

### 9b. The first REAL pass — ✅ LANDED: global CSE (O2). Next-pass candidates below.
The first registered `CorePass` is **`GlobalCsePass` (O2)** in `opt.rs`: whole-function global common-
subexpression elimination on the Core column (the backend-independent lift of the wasm backend's Lir-slot
CSE), installing `Core::Let` overrides so BOTH backends compute a repeated trap-free subexpression once.
Its three soundness guards (SCALAR-ONLY, FRONTIER, TRAP-FREE-OR-FRONTIER) are replicated verbatim from
v-wasm-opt's `select.rs` analysis.
- **⚠ MVP LIMITATION (the load-bearing next follow-up).** `PassManager::run` currently fires BEFORE lazy
  lowering, so a pass-time `core_of` is the FIRST demand of a node — and a node whose correct lowering needs
  a context established at lower time (lambda-lift / handler-lift / contract-desugar / `?`-try / pattern
  binder) would lower WITHOUT it and MEMO-POISON `db.core` (→ "reference has no local slot" at emit). So
  `GlobalCsePass` is gated to `body_is_pure_scalar` bodies. The proper fix is **TIMING — run passes over an
  already-lowered+lifted column (a `force-lower-all` before the `PassManager`)**; that Option-A follow-up
  lets CSE (and every future pass) cover capturing/effectful bodies too. This is the highest-value next
  OptLevel slice (expands the shipped pass's reach; backend-independent; no cross-vertical coordination).
- **Other next-pass candidates** (once the timing fix lands, or for pure-scalar-safe shapes now): a Core-tier
  **redundant-`if`/select canonicalization** (identical-branch collapse over a trap-free cond, double-negation
  unwind, `(= b true)`→b). Precondition to check first: confirm `lower.rs` does NOT already fold these (the
  corpus pins witness the OUTCOME is correct, not that a Core pass vs the backend does it). If lower already
  folds them, pick a transform it does not. The proof-guided-elision (9.2) is the eventual flagship.

### 9c. Near-term plan (one gated slice per tick, under the §5 level-equivalence gate)
- ~~**Slice 1:** the `core_of` override seam + `PassManager::run` populating it.~~ ✅ **DONE** (§9a).
- ~~**Slice 2:** register the first REAL pass.~~ ✅ **DONE** — `GlobalCsePass` (O2), MVP pure-scalar (§9b).
- **Slice 3 (NEXT, highest-value):** the **pass-timing fix** — force-lower+lift the column BEFORE the
  `PassManager` runs, so a pass-time `core_of` is not a first-demand (removes the `body_is_pure_scalar`
  gate; lets CSE + every future pass cover capturing/effectful bodies). Gate: byte-identical emit on both
  backends for the pure-scalar bodies already covered + newly-covered bodies stay behavior-identical
  (`--opt-sweep` + `gate --check`). Backend-independent, no cross-vertical coordination.
- **Slice 4+:** more registered passes (§9b redundant-`if`/boolean canonicalization pending the `lower.rs`
  precondition check; then lift dominator-CSE / accumulator-intro from `backend/wasm/`, each `note`-
  coordinated with v-wasm-opt per §4's ordering rule). Each pass lands with its `min_level` + a
  level-equivalence gate case.
- **Ongoing design:** coordinate the proof-guided-elision seam (9.2) with v-verification; when their
  pre/post-condition query exists, add the checked-op-with-discharged-proof → unchecked-op pass.
- **Standing coverage gap (flagged to v-nix):** `--opt-sweep` (the level-equivalence check) is manual-only —
  not in the nix gate battery — so the tiered-opt invariant is unprotected fleet-wide. Proposed adding it to
  the hourly-advisory run (4× cost, shardable). Until wired, run it manually when touching a pass.
This section supersedes §8's "hold" — the framework is FILLED and operational (seam + one real pass) per the
operator mandate; Slice 3 (the timing fix) is the next unit.

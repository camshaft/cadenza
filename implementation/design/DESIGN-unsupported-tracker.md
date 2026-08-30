# Unsupported-error tracker — an auto-generated sexpr registry of every rcdzc decline, annotated with what each is blocked on

> **Operator spark (verbatim, 2026-08-30 via Slack):** *"i want an automated way of tracking all of
> the unsupported errors in the rcdzc compiler. and i want a way to say what it's blocked on getting
> implemented. i want the tracking to all be in sexpr."*

Four forking decisions were pinned with the operator at design kickoff (2026-08-30) — they set the shape
of everything below:

1. **Scope** = *all declines* (`Reject::decline` + `Reject::unsupported`, ~698 sites), NOT the coded
   rejects. Declines are precisely the errors that can be "blocked on getting implemented."
2. **Sync mechanism** = *symbolic decline IDs* — make the decline catalog a first-class, enumerable
   thing in the compiler; the registry is generated from it. (Chosen over a source-text scrape and over
   a runtime-enumeration harness — the operator wants the robust in-sync story, worth the refactor.)
3. **Blocked-on** = *structured fields* (owner / needs / ref / status), not free prose.
4. **sexpr vs binary-AST** = the sexpr registry is a *source-surface* artifact (like the corpus `.sexp`
   and `data/wasm-abi.sexp`); binary-AST remains the wire format if any tool ever exchanges it. sexpr is
   the surface, binary-AST is the exchange — the standing "binary-AST is THE data-exchange format"
   mandate is honored, not bent.

## 0. What already exists (the honest baseline — measure before building)

The "no" abstraction is `Reject` in `implementation/seed/crates/rcdzc/src/diag.rs` (struct at
diag.rs:688–703), carrying `code: Option<Code>`, `message: String`, `at: Option<StructId>` (the AST node
the "no" is about), and an optional structural `fix`. Three constructors:

- `Reject::decline(msg)` — diag.rs:725 — **uncoded** decline (`code: None`). **530 sites.**
- `Reject::unsupported(msg)` — diag.rs:742 — a decline carrying the umbrella code
  `Code::UnsupportedConstruct` = **CDZ0900** (diag.rs:387/:443). **168 sites.**
- `Reject::coded(code, msg)` — diag.rs:709 — a real **rejection** ("the program is wrong"). **446 sites.**

`is_decline()` (diag.rs:780) is true for both decline flavors (codeless + CDZ0900) and false for coded
rejects and `CDZ0999` (RecursionBound — a resource wall, not a not-yet-built gap). **Declines span 32
source files.** The `Code` enum (diag.rs:39–388) + its `code()` string table (diag.rs:402–444) is the
authoritative error-code registry; codes are wording-independent by spec (a re-word never moves a code).

**How declines are tracked TODAY — and why it is inadequate:**
- **v-deferral-declines** enumerates declines with *ad-hoc multiline regex scans*
  (`rg -U --multiline-dotall --count-matches 'decline\(…(later increment|not yet…)'`). Those scans have
  repeatedly miscounted across ticks (71 → 82 → 99 → 110 for the same tree) because a `format!` message,
  a shared-const decline, or an internal `)` defeats the regex. **There is no canonical count.**
- **What each decline is "blocked on"** is real, valuable knowledge — but it lives *hand-maintained and
  scattered* across the fleet's memory logs (owner routing, "blocked on the runtime-Char rep", "blocked
  on #3228 world-result-type", "design-gated pending operator ruling"). It is not a queryable artifact,
  it rots, and it is invisible to anyone outside the memory graph.
- **The corpus** pins *individual* declines a program triggers as `(declines CDZ0900 …)` expectations
  (cdz-corpus / cdz-corpus-grade). That is per-example regression protection, NOT a catalog of the
  compiler's decline *surface*.

**The precedent to reuse (the crux):** the repo already has the exact "sexpr source artifact + generator
+ `--check` drift gate" pattern:
- `data/wasm-abi.sexp` — a checked-in, repo-root, language-independent sexpr table (operator seq-173:
  kept *outside* the Rust tree on purpose), shape `(do (opcode NAME n) …)`.
- `xtask/src/codegen.rs` — `emit_or_check(out, source, check, oracle, summary)` (codegen.rs:176): in
  `--check` mode it regenerates in-memory, byte-compares to the committed file, and on drift prints
  *"xtask codegen --check: <file> is OUT OF DATE with <oracle>. … Fix: run cargo xtask codegen and
  commit …"* then fails. Wired as a hard gate in `xtask check`. `xtask-codegen-wasm-abi`'s `--oracle-check`
  (main.rs:945–996) is the same idea with add/drop/wrong-value drift reporting.

So the tracker is not a new mechanism class — it is a *third instance* of an established one, with the
decline catalog as the oracle.

**⚠ Convergent mandate (must not build twice):** the operator's seq-286-BROAD directive to
**v-deferral-declines** — *"assign a stable trackable CDZ referent to ALL ~529 codeless declines + route
coding to owning lanes"* — is the **same mechanism** this design specifies. A `DeclineId` *is* the
"stable trackable referent"; the `(blocked-on (owner …))` field *is* "route coding to owning lanes." So
this design is the concrete framework that seq-286-broad plugs into — `DeclineId` + `data/unsupported.sexp`
should BE how v-deferral-declines discharges seq-286-broad, co-owned, not a second parallel effort. This
is the load-bearing coordination call (§5, §7).

## 1. The gap — what the operator's ask is really for

Three things, none of which exists today as one artifact:
1. **Completeness by construction** — "*all* of the unsupported errors," guaranteed, not scraped. You
   should be unable to add a decline to the compiler without it appearing in the tracker.
2. **A blocked-on arc per entry** — machine-readable "what has to land before this stops declining, and
   who owns it," promoted from scattered memory prose to a tracked field.
3. **sexpr as the surface** — the whole artifact human-readable/reviewable in sexpr, kept in sync
   automatically.

## 2. The core design (RECOMMENDED shape)

Two coupled pieces: a **first-class decline catalog in the compiler** (the oracle) and a **generated
sexpr registry** (the surface, carrying the human-authored blocked-on).

### 2.1 `DeclineId` — the enumerable catalog (the oracle)

Introduce a stable symbolic identifier for each distinct decline **reason**, in a new submodule
`rcdzc/src/diag/declines.rs`:

```rust
/// The stable, enumerable catalog of every construct rcdzc declines to compile.
/// A `DeclineId` names a REASON (a capability the compiler does not realize), not a call site —
/// sites emitting the same reason share an id. Minting a variant is the only way to emit a decline,
/// so `DeclineId::ALL` is a COMPLETE, by-construction list of the compiler's decline surface.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DeclineId {
    CharRuntimeScalarAt,
    WasmHostBoundaryCompoundArg,
    TailResumptiveFoldNonMatchForm,
    CrossModuleImport,
    // … one per distinct decline reason
}

impl DeclineId {
    /// The complete catalog — the generator iterates this; completeness is structural.
    pub const ALL: &'static [DeclineId] = &[/* every variant */];
    /// Stable kebab-case key used in the sexpr registry (never changes once minted).
    pub fn key(self) -> &'static str { … }
    /// The umbrella code this decline carries today (CDZ0900), or None while still codeless.
    pub fn code(self) -> Option<Code> { … }
    /// A canonical one-line reason (wording-independent of the runtime `format!` message).
    pub fn reason(self) -> &'static str { … }
}
```

The decline constructors gain an id-carrying form; the runtime `message` keeps carrying the *specifics*
(the offending type, the arity, …) while the `DeclineId` is the stable catalog key:

```rust
impl Reject {
    /// A decline naming its catalog id. The umbrella code (if any) comes from `id.code()`.
    pub fn declined(id: DeclineId, message: impl Into<String>) -> Reject { … }
}
```

`Reject` gains an `id: Option<DeclineId>` field. During migration, bare `decline()`/`unsupported()`
still exist (id `None`); the end state deprecates them so *every* decline carries an id (see §3).

**Why per-reason, not per-site:** the operator tracks *what needs implementing* — a capability, not a
line number. ~698 sites collapse to an estimated ~150–250 reasons; a reason is the unit that gets
"unblocked." (Chosen default — see §6.)

### 2.2 `data/unsupported.sexp` — the generated registry (the surface)

A repo-root, language-independent sexpr artifact (sibling of `data/wasm-abi.sexp`). One form per catalog
id. The generator writes the **compiler-derived** fields (`code`, `reason`, `sites`) and *preserves* the
**human-authored** `(blocked-on …)` block across regenerations:

```
(do
  (unsupported char-runtime-scalar-at
    (code CDZ0900)
    (reason "String.scalar-at over a runtime index")
    (sites 1)
    (blocked-on
      (status permanent)
      (owner v-corpus-declines)
      (needs "operator ruled char has no runtime scalar-at; use String.at")
      (ref pr 5848)))
  (unsupported wasm-host-boundary-compound-arg
    (code CDZ0900)
    (reason "a compound value crossing the host call boundary as an argument")
    (sites 9)
    (blocked-on
      (status in-flight)
      (owner v-rust-backend)
      (needs "the WIT arg/param boundary shape-coverage matrix")
      (ref doc "backend/wasm/WIT-BOUNDARY-SHAPE-COVERAGE.md")))
  (unsupported cross-module-import
    (code CDZ0900)
    (reason "cross-module import resolution")
    (sites 1)
    (blocked-on
      (status design-gated)
      (owner v-module-system)
      (needs "an operator design decision on the module surface"))))
```

`(status …)` is a small enum: **`blocked`** (a named dependency must land first), **`in-flight`** (owner
actively building), **`permanent`** (a by-design decline that will never be implemented — kept so the
tracker *distinguishes "won't" from "not yet"*), **`design-gated`** (needs an operator ruling), and
**`unowned`** (freshly-minted, not yet triaged — the backlog the generator forces you to clear).

## 3. Increments (top-to-bottom, the way a vertical lands them)

1. **Catalog infrastructure (compiler-only, no behavior change).** Add `DeclineId` (seeded with ~6–10
   ids for the sites the vertical migrates first), `Reject::declined()`, the `id` field, and
   `DeclineId::ALL`/`key`/`code`/`reason`. Unit test pins the catalog is enumerable and every id's
   `key`/`code` is stable. Messages unchanged → zero corpus/gate movement.
2. **Registry + generator + gate.** Add `data/unsupported.sexp`; add an `xtask codegen` target (extend
   `xtask/src/codegen.rs` via `emit_or_check`, or a new `xtask-codegen-unsupported` crate mirroring
   `xtask-codegen-wasm-abi`) that iterates `DeclineId::ALL`, emits/merges the registry preserving
   `(blocked-on …)`, and in `--check` mode reds on: a new id with no entry, an entry for a dead id, or a
   drifted `code`/`reason`. Wire `--check` into `xtask check`. Seed blocked-on for the Inc-1 ids.
3. **Site migration in batches (coordinated with v-deferral-declines).** v-deferral-declines already
   touches every decline site for the seq-280 professional-wording + seq-286 CDZ0900-coding pass — the
   `DeclineId` is folded into *that same edit* (mint the id, switch `decline(msg)` →
   `declined(id, msg)`, add the registry entry with a triaged blocked-on). One cluster per batch (wasm
   host-boundary, lower/* runtime ops, resolve.rs effects, …). Gate stays green throughout.
4. **Close the door.** Once all ~698 sites carry an id, deprecate bare `Reject::decline`/`unsupported`
   (keep them only for the truly-transient) and add a lint/gate so a *new* decline without a
   `DeclineId` cannot land. At that point the registry is provably complete.

A migration progress metric ("sites still lacking an id" → 0) reuses v-deferral-declines' drive-to-0
tracking, so the two efforts share one dashboard rather than competing.

## 4. Soundness & the gate (the correctness bar — non-negotiable)

- **Completeness is structural, not scraped.** `DeclineId::ALL` is the single source of truth; the
  generator iterates it. You cannot emit a decline without an id (post-Inc-4), and you cannot add an id
  without the `--check` gate demanding a registry entry. No regex, no undercount.
- **The gate = `cargo xtask codegen --check`**, byte-identity on the *generated* fields, blocked-on
  *preserved* (never clobbered by regen). It reds on new-untracked / dead-entry / code-or-reason-drift,
  with the standard "run `cargo xtask codegen` and commit" fix line. It joins the existing
  `xtask check` codegen family (`wasm_abi.rs`, `runtime_abi.rs`, contract schemas).
- **No runtime-hash entanglement.** `diag.rs` and the catalog are compiler-side; they are *not* inside
  `REQUIRED_RUNTIME_HASH`, so this is not a flag-day and needs no store rebuild.
- **Blocked-on is advisory, not compiled.** A stale/incomplete blocked-on never breaks a build — only a
  *missing entry for a live id* (or a dead entry) reds the gate. This keeps the human field honest
  without making it load-bearing for compilation.

## 5. Territory & coordination (who owns what — avoid collision)

- **`rcdzc/src/diag.rs` + new `diag/declines.rs`** (the `DeclineId` enum + `declined()` + `id` field):
  **v-deferral-declines** is the natural OWNER of the build, not merely a coordinating peer — their
  seq-286-broad mandate ("a stable trackable CDZ referent for all ~529 codeless declines + route coding
  to owning lanes") is discharged BY building this. The `DeclineId` catalog is that referent; the
  per-site id-migration rides *inside* their existing wording/coding sweep — one pass, not two. Also
  overlaps **v-corpus-harness** (owns the `Code` registry / C1 code assertions). Recommendation to the
  PM/operator: hand the build to v-deferral-declines (extend its charter) rather than spin a disjoint
  vertical that would re-sweep the same 698 sites.
- **`data/unsupported.sexp`** (new): owned by the vertical that builds this; blocked-on entries are
  authored by whoever triages a decline (often the owning lane, relayed).
- **`xtask` codegen**: extend `xtask/src/codegen.rs` or add `xtask-codegen-unsupported`; the gate wires
  into `xtask check`.
- **v-corpus-declines** owns the corpus `(declines CDZ0900 …)` per-example pins — *distinct* from this
  surface catalog; the two reference the same CDZ0900 code but serve different jobs (per-example
  regression vs. compiler-wide surface). Reconcile terminology, don't merge.
- **v-inference** is the reject-vs-decline semantic authority — it adjudicates any site whose scope
  (decline vs. permanent coded reject) is ambiguous at migration time.

## 6. Decisions (resolved at kickoff + chosen defaults for the sub-forks)

- **[operator-pinned]** Scope = all declines; sync = symbolic ids; blocked-on = structured; sexpr =
  source surface. (§ top.)
- **[chosen default]** **Id granularity = per distinct reason**, not per call site (§2.1). If the
  operator wants per-site resolution instead, `sites` becomes a list of file:span anchors and the id
  count rises toward ~698 — flagged for confirmation at handoff.
- **[chosen default]** **Blocked-on lives in the sexpr, not in Rust** — keeps routing churn out of the
  compiler and satisfies "all in sexpr"; the compiler only owns `code`/`reason`.
- **[chosen default]** **Registry at `data/unsupported.sexp`** (repo root, language-independent, per
  seq-173 — same home as `data/wasm-abi.sexp`).
- **[chosen default]** The registry records per-id **`(code …)`** (CDZ0900 or absent-while-codeless), so
  it doubles as v-deferral-declines' coding-pass progress view.

## 7. Stakeholder review — PENDING

To be signed off before/at build handoff:
- **v-deferral-declines (PRIMARY — likely build owner)** — reconcile this design against their
  seq-286-broad mandate; confirm `DeclineId` + `data/unsupported.sexp` IS their "stable trackable CDZ
  referent" deliverable, so it is built once inside their per-site wording/coding pass. This sign-off
  gates the whole handoff — if they instead build a different referent, this design must adapt to it.
- **v-corpus-harness** — `Code`-registry authority; confirm the catalog does not conflict with the
  CDZ0900 umbrella ruling and that `reason`/`code` split is sound.
- **v-corpus-declines** — confirm the surface catalog vs. corpus `(declines)` pins terminology split.
- **v-inference** — standing adjudicator for ambiguous decline-vs-reject scope calls during migration.

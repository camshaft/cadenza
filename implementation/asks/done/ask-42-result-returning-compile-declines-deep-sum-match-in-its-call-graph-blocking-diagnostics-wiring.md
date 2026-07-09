## 42. 🔴 A `Result`-returning `compile` declines on the DEEP sum-matches in its call graph — blocking the diagnostics-channel wiring (ask-40's payoff)

**Finding.** ask-40 landed the diagnostics ABI: `(def (compile b) …)` returning `Result<Bytes,
list<diagnostic>>` is lifted as `compile: list<u8> → result<list<u8>, list<diagnostic>>`, verified for
*unconditional* bodies (`(Ok <bytes>)` → `Ok`, `(Err (list (record (code …) (message …))))` → `Diagnostics`).
But wiring it into the REAL `compile.cdz` — so an ill-typed/malformed program returns `(Err <CDZ0201>)`
instead of trapping — **declines the whole compiler**:

```
declined: runtime match with a non-literal pattern
```

The change is minimal and idiomatic (no contortion):
```
(def (compile b) (compile-result (resolve-module (read-module b))))
(def (compile-result funcs)
  (if (any-func-rejects? funcs (build-ktab funcs))       ; recursive FList walk; per-func well-typed?/has-kerror?
      (Err (reject-diagnostic))                          ; (list (record (code "CDZ0201") (message …)))
      (Ok (compile-program funcs))))                     ; compile-program : FList → Bytes (a helper)
```
`compile-program` is a helper returning `Bytes` (the Ok arm is the known-good "opaque call result" shape, NOT
an inline literal), and `reject-diagnostic` returns a `list<record>`. The SAME `compile-program` /
`well-typed?` / `has-kerror?` / `any-func-rejects?` functions compile FINE when `compile` returns bare
`Bytes` (the current shipping seam) — so this is specifically the **Result-shaping analysis of the compile
call graph being more restrictive than the bare-`Bytes` path.**

**Bisection (2026-07-07, against the real compiler.cdz):**
- `compile = (Ok (compile-program (resolve-module (read-module b))))` — no rejection check — **COMPILES** (`Ok
  89 bytes`). So `(Ok <helper→Bytes>)` and the Result-wrapping itself are fine.
- Condition `false` literal (Err arm present but const-folded away) — **COMPILES**. So the `Err`-arm shape and
  `reject-diagnostic` are fine.
- Condition `(has-kerror? (func-body (flist-head funcs)))` alone — **COMPILES**.
- Condition `(not (well-typed? (func-body (flist-head funcs)) (build-ktab funcs)))` alone — **COMPILES**.
- Condition `(func-rejects? (flist-head funcs) (build-ktab funcs))` (= `(or (not (well-typed? …))
  (has-kerror? …))` on ONE func) — **DECLINES** "runtime match with a non-literal pattern".
- Condition via `any-func-rejects?` (the recursive FList walk) — **DECLINES** (same).

So the trigger appears once the rejection check both (a) is reached through the `FList`/`Func` unpacking of the
resolved program and (b) drives the two deep `Core` sum-matchers (`well-typed?` and `has-kerror?`, which match
`Core` with nested `(tuple a b)` payloads) — in the Result-shaped `compile`. Simpler standalone reproductions
(a recursive nested-tuple sum-match in the condition + an `Ok(helper)` arm) do NOT trigger it, so it is the
combination at the real compiler's scale/shape, not any single construct.

**Where it comes from (seed).** `gen_match_arms` (codegen.rs ~4890) declines a runtime `match` arm whose
pattern is not an int/bool literal or a name/`_`/`else` catch-all: `_ => decline("runtime match with a
non-literal pattern")`. In the bare-`Bytes` path these constructor-pattern matches are lowered by the seed's
runtime sum-match machinery (heap sums) and compile fine; under the Result-shaping analysis of `compile`, some
scrutinee's kind resolves to a non-heap kind on this path, so the constructor arm falls to the scalar
`gen_match_arms` and declines. The Result-shape analysis needs to treat these scrutinees as the heap sums they
are, exactly as the bare-`Bytes` path does.

**Repro (the real artifact — one-line change).** In `implementation/compiler/compiler.cdz`, change the
`compile` body from `(compile-bytes b)` to `(compile-result (resolve-module (read-module b)))` (the
`compile-result` / `any-func-rejects?` / `func-rejects?` / `has-kerror?` / `reject-diagnostic` defs are already
present, DORMANT, in the file). Then `cadenza-seed compile-run <compiler.cdz> <any-well-typed.cdz>` →
`declined: runtime match with a non-literal pattern` instead of `Ok (N bytes)`.

**Why it matters.** This is the last hop for ask-30 + ask-40: the compiler now REJECTS ill-typed/malformed
programs (they trap), and the ABI to report a coded `CDZ0201` exists — but the compiler can't USE the ABI
because turning `compile` into a `Result` makes the seed decline its own sum-matches. Until fixed, the ~30
`native=rejected` cases stay `decline` (honest trap) instead of reaching `agree` (coded diagnostic matching
native). No workaround is acceptable (restructuring the matchers to dodge the analysis would be contorting the
compiler around the gap).

**Acceptance signal.** With the one-line `compile` change above, `compile-run <compiler.cdz>` on a well-typed
program returns `Ok (N bytes)` and on `(+ 1 true)` / `(if true 1 false)` / `(+ 1)` returns `Diagnostics:
[("CDZ0201", …)]`; `component-check` then scores the ~30 rejection cases `agree` (code matches native) rather
than `decline`. Current state: `compile` reverted to bare `Bytes` (self-hosts, 0 hard/0 error); the diagnostic
detectors are in place and unit-consistent (each compiles individually), waiting on this analysis fix.
Related: ask-40 (the ABI, done), ask-30 (the type-checker whose rejections this reports), ask-13 (the general
sum/list-pattern surface — a broader instance of the same runtime-match machinery).

**UPDATE 2026-07-07 (the kinded-artifact interface — ask-41 — would SIDESTEP this bug).** The operator moved
the interface to `compile: list<artifact> → {artifacts, diagnostics}` (Amendment 0.8.0, ask-41). That return is
a SINGLE record type on BOTH the success and rejection paths — unlike `Result`, whose `Ok`/`Err` arms are
DIFFERENT variant types. This bug is rooted in the seed's Result-shape analysis handling the `if`/`match` that
chooses between an `Ok`-arm and an `Err`-arm (different shapes) in the compile call graph. With a record return,
`(if <deep-sum-match> (mkdiag-record) (mkartifact-record))` has both branches at the SAME `compile-output` shape
— no variant conflict. VERIFIED (seed 13:51): that exact conditional-with-deep-sum-match shape, returning the
record on both arms, **compiles VALID and does NOT decline** (the host reads `Ok (0 bytes)` only because the
artifact ABI is not yet DECODED — ask-41 unrealized — but the seed does not choke on the conditional). So
**realizing ask-41 (the artifact envelope) is the clean path that ALSO closes ask-42's diagnostics wiring** —
the compiler would return `{artifacts: [component], diagnostics: []}` on success and `{artifacts: [],
diagnostics: [CDZ0201]}` on rejection, both the same record type, dodging the Result-branch shape-analysis
entirely. Recommend fixing via ask-41 rather than patching the Result-branch analysis.

**UPDATE 2026-07-07 (after the sibling's tail-position Result-detection fix landed, seed 13:34).** The
*decline* is GONE — the Result-wired `compile` now BUILDS and self-hosts (`(Ok (compile-program funcs))` →
`Ok (89 bytes)` for a well-typed program). But the rejection path now MIS-ROUTES: an ill-typed / malformed
program returns **`Ok (88 bytes)`** (the `unreachable`-stub component) instead of `Diagnostics`. I.e. the
condition `(any-func-rejects? funcs (build-ktab funcs))` evaluates to **false at run time** even for `(+ 1)`
(which has a clear `KError` node in its resolved tree that `has-kerror?` must catch) and `(+ 1 true)` (which
`well-typed?` must reject) — yet `compile-program`'s OWN internal `typecheck-funcs` (same `well-typed?`) still
turns the body into the KError stub (hence 88 bytes). So `any-func-rejects?` and `typecheck-funcs` — using the
IDENTICAL `well-typed?` — DISAGREE at run time: one sees no rejection, the other does. The `has-kerror?` /
`well-typed?` logic is correct in ISOLATION (a standalone `run()` of the same recursive nested-tuple sum-match
returns the right bool). So the seed is mis-lowering `any-func-rejects?`'s recursive FList-walk-of-Core-matches
**specifically when it sits in the condition of a Result-lifted `compile`** — the walk returns the wrong
(false) answer rather than declining. This is the same root (Result-shaping changes how the deep sum-matches
lower) now surfaced as a WRONG VALUE (Ok-instead-of-Err) rather than a decline — arguably worse. Reverted to
bare-`Bytes` again (self-hosts, 27 agree / 0 hard / 0 error). The one-line repro is unchanged; the seed-side
fix must make the deep sum-matches in `compile`'s condition evaluate identically whether `compile` returns
`Bytes` or `Result`.

**🔴 LOOP-VERIFIED STILL OPEN 2026-07-07 (Run 83) — persists on seed 14:10.** Independently re-probed the
one-line repro on the CURRENT seed: applied `(compile b) = (compile-result (resolve-module (read-module b)))` to
a copy of compiler.cdz and ran `compile-run <copy> <well-typed>` → `declined: runtime match with a non-literal
pattern`. So the scrutinee-kind divergence (a heap-sum scrutinee mis-inferred as scalar under Result-shaping →
the constructor arm falls to the scalar `gen_match_arms` and declines) is NOT fixed by the recent rebuilds; it
remains the last blocker for ask-30/ask-40 → agree. The SHIPPING bare-`Bytes` compiler.cdz is unaffected
(WRONG sweep = 0, gate green) — the miscompile is latent, only in the dormant Result-wired path.

---

**🔬 ROOT CAUSE TRACED + APPROACH DECIDED 2026-07-07 (seed).** Reproduced (flip `compile` to
`(compile-result (resolve-module (read-module b)))`): declines "runtime match with a non-literal pattern".
Traced the exact cascade:
1. `func-body`'s arm `((Func.Fn (tuple np body)) body)` returns `body` = slot 1 of `(Tuple Int64 Core)`
   = a `Core` (Heap). But inference does NOT seed a match arm's pattern binders with their declared slot
   kinds, so inferring the bare-name body `body` (not in the var set) defaults `func-body`'s RETURN to
   Int64 (verified: `func-body ret=Int64` in BOTH the bare and Result paths — a latent mis-inference).
2. A caller `(well-typed? (func-body f) ktab)` then passes an Int64 arg where the param is Heap →
   kind mismatch → `gen_call` INLINES `well-typed?` with its param aliased to the Int64 arg node.
3. Inside the inline, `well-typed?`'s own `Core` constructor-`match` scrutinee is now scalar → falls to
   the SCALAR `gen_match_arms` → declines "runtime match with a non-literal pattern".
Only bites at self-host scale (the FList/Func unpacking feeding the deep matchers through the
Result-shaped entry); a minimal "extract a compound slot, then ctor-match it" plain program compiles fine.

**Tried + REVERTED:** the direct inference fix (seed arm binders with declared slot kinds before
inferring the arm body) works but re-walks arm subtrees inside the kind-inference fixpoint → **compile-cost
blowup** (bare compiler.cdz went sub-second → >60s — the exponential-in-inference class). Reverted; a
blowup-free version is possible but fiddly and, more importantly, superseded:

**DECISION — pursue via ask-41 (the kinded-artifact interface), not a result<> point-patch.** As the
sibling verified and this trace confirms, the mis-shape is provoked by the `result<Ok, Err>` entry's
DIFFERENTLY-TYPED arms (`list<u8>` vs `list<diagnostic>`). The `{artifacts, diagnostics}` record
(Amendment 0.8.0) is ONE type on both success and rejection, so the deep sum-match that chooses between
them has same-shaped branches and lowers as an ordinary heap consumer — the same trigger body COMPILES
under the record return. So realizing ask-41 (envelope + wrapper + selection) closes ask-42 by
construction. This ask stays open as: EITHER realize ask-41, OR (independently) fix the arm-binder
slot-kind inference WITHOUT re-walking the fixpoint. Learning:
`spec/learnings/2026-07-07-a-result-typed-entry-can-mis-shape-a-deep-sum-match-in-its-call-graph.md`.

**Still open on seed 14:34 (Run 86 re-probe).** Another seed rebuild (+17KB, native effects work) did not touch
ask-42 — the Result-wired copy still `declined: runtime match with a non-literal pattern`. Stable refreshed
(gate 570, WRONG=0); the diagnostics blocker persists. The kinded-artifact interface (ask-41) remains the path
around it.

**Still open on seed 14:52 (Run 87).** Another rebuild (+5870 compiler.cdz, envelope files edited — ask-41
artifact-ABI in flight) did not fix ask-42; Result-wired copy still declines. Per SEED-GAPS the diagnostics
COLLECTION side works; the artifact-record OUT-channel (`{artifacts, diagnostics}` reads `Ok (0 bytes)`,
undecoded) is the sole remaining wiring — ask-41 "closes this by construction."

---

## ✅ RESOLVED-BY-CONSTRUCTION 2026-07-07 (conformance loop) — via ask-41

ask-41 (the full symmetric kinded-artifact ABI) is LANDED, which — as this ask itself predicted — closes
the diagnostics-wiring blocker BY CONSTRUCTION: `{artifacts, diagnostics}` is ONE record type on both the
success and rejection branches, so the choosing `if`/`match` has same-shaped branches and the deep `Core`
sum-match is an ordinary heap consumer (no Result-shape reconciliation, so the arm-binder slot-kind
mis-inference never triggers). Verified: `(def (compile inputs) (if <deep-sum-match> (mkdiag-record)
(mkcomponent-record)))` compiles VALID and the host now DECODES the record (picks the component artifact /
reports diagnostics) instead of reading `Ok (0 bytes)`.

**What this means for compiler.cdz:** switch the `compile` return from `result<>` to the `{artifacts,
diagnostics}` record and the ~30 ask-30 rejections reach `agree` — see the ask-41 handoff banner atop
SEED-GAPS for the exact record shape and severity convention.

**Residual (NOT this ask):** the underlying arm-binder slot-kind inference bug is real but only bites the
`result<>` shaping; it is captured in `spec/learnings/2026-07-07-a-result-typed-entry-can-mis-shape-a-deep-sum-match-in-its-call-graph.md`
and a faithful fix (seed arm binders with declared slot kinds WITHOUT re-walking the inference fixpoint —
the naive fix blew compile cost up 60×) remains a future seed nicety, no longer on the self-hosting critical
path. Closing this ask as resolved via ask-41. Learning: `kinded-artifact-abi-and-cabi-realloc-arg-order`.

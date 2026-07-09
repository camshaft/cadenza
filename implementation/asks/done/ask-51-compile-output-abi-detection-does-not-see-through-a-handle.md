## 51. 🔴 The `compile-output` ABI detection doesn't look through a `handle` — blocks EFFECT-based diagnostics (the operator's direction)

**Finding.** ask-41 landed the `compile: list<artifact> → compile-output{artifacts, diagnostics}` ABI, chosen
from the body's static return shape via a tail-position walk (per the handoff banner: through
`if`/`match`/`let`/`do`/1-level-helper). But the walk does NOT look through a `handle` — so when the
`compile-output` record is produced INSIDE a `Diag` effect handler (the natural shape for effect-based
diagnostics: `handle` the whole check+emit+collect+build-record), the seed falls back to the bytes ABI and the
record is read as raw bytes (`Ok (0 bytes)`), not decoded as `{artifacts, diagnostics}`.

**Boundary, isolated (2026-07-07, seed 16:31, `compile-run`):**

| `compile` body shape | ABI detected |
|---|---|
| record DIRECTLY: `(record (artifacts …)(diagnostics …))` | ✅ artifact ABI (`Ok (1 byte)`) |
| record via `let` tail: `(let ((x 1)) (record …))` | ✅ artifact ABI (`Ok (1 byte)`) |
| record via `handle` tail: `(handle … (record …))` | 🔴 NOT detected → falls to bytes ABI (`Ok (0 bytes)`) |

So the tail-position walk covers `let`/`do`/`if`/`match`/helper but stops at `handle`. The record inside the
handle compiles and runs fine (ask-49 landed the compound-returning recursive-effectful handle) — it's purely
the ABI *detection* that doesn't recurse into the handle's body to find the `(record (artifacts …) …)`.

**Minimal repro (falls back to bytes ABI):**
```
(module m
  (effect D (op e (-> Int64 Unit)))
  (def (compile inputs)
    (handle (list) ((D.e (v) s (resume unit s)))
      (record (artifacts (list)) (diagnostics (list))))))
```
→ `Ok (0 bytes)` (bytes ABI). Move the record OUT of the handle → `Ok (1 byte)` (artifact ABI).

**Why it matters.** This is THE last hop for effect-based diagnostics, the operator's explicit direction ("use
effects... emit diagnostics via an effect"). The `Diag` effect + recursive `check-*` pass are built and proven
in compiler.cdz; ask-46 (handle at compile entry) and ask-49 (compound-returning handle on the run/emit path)
both landed, so the handler itself works. The ONLY remaining blocker to wiring `compile` as
```
(handle (list) ((Diag.emit …)(Diag.collect …))
  (do (check-funcs …) (record (artifacts (list (component-artifact bytes)))
                              (diagnostics (Diag.collect unit)))))
```
is that the seed doesn't recognize the `compile-output` record as the return shape when it sits inside the
handle. The handoff banner notes a workaround — "collect diagnostics with a plain recursive return-a-list pass
(no handler) and still return the record, sidestepping ask-46/effects entirely" — but that ABANDONS the effects
direction the operator asked for (the diagnostics would be threaded by explicit accumulator args, not the `Diag`
effect). So the faithful fix is to make the ABI detection look through `handle` (its body's tail), exactly as it
already looks through `let`/`do`/`if`/`match`.

**Acceptance signal.** The minimal repro above detects the artifact ABI (`Ok (1 byte)` / decodes as
`{artifacts:[], diagnostics:[]}`), and compiler.cdz's `compile` can be the `Diag`-handler-wrapped
`compile-output` record — self-hosting, gate-green, with the collected diagnostics surfaced. Then the ~30
ask-30 ill-typed rejections reach `agree` via the effect-based pipeline.

**Status.** 🔴 Seed — extend the `compile-output`/`Result` ABI tail-position detection to recurse through a
`handle`'s body (the last construct in the walk it doesn't cover). Related: ask-41 (the artifact ABI + the
tail-walk this extends), ask-46/ask-49 (the recursive-effectful handle lowering, both landed — this is detection,
not lowering), ask-45 (the `Diag` collection), ask-30 (the rejections this surfaces). Current state:
compiler.cdz stays bare-`Bytes` (self-hosts, 27 agree / 0 hard / 0 error); `Diag` decl + `check-*` pass retained.

---

**✅ CONFIRMED FIXED IN SOURCE — awaiting a STABLE refresh (loop, 2026-07-07 ~16:45; CONFIDENCE: HIGH).**

The fix is ALREADY in the seed source — the `handle`-tail recursion the ask asks for is present in BOTH
tail-walks, each carrying an explicit ask-51 comment:
- `compile_body_is_artifacts` — `codegen.rs:1517`: `Some("handle") if items.len() == 4 => self.compile_body_is_artifacts(&items[3], seen),`
- `compile_body_is_result` — `codegen.rs:1457`: `Some("handle") if items.len() == 4 => self.compile_body_is_result(&items[3], seen),`

**Why the ask was still reproducing:** the finding was probed against the STABLE snapshot
(`implementation/stable/cadenza-seed`, mtime **16:38:26**), but the source fix + a fresh
`target/release/cadenza-seed` landed at **16:40** — so stable is ~2 min STALE relative to the fix. This is the
recurring "verify on the artifact you built, not the stale snapshot" pattern.

**Independent re-probe — the boundary FLIPS on the freshly-built seed (`implementation/seed/target/release/cadenza-seed`, 16:40, with the matching fresh `crates/cdz-runtime/.../cdz_runtime.wasm`):**

| `compile` body shape | STALE stable (16:38) | FRESH build (16:40) |
|---|---|---|
| record DIRECTLY | `Diagnostics: []` (artifact ABI ✅) | `Diagnostics: []` ✅ |
| record via `let` tail | `Diagnostics: []` ✅ | `Diagnostics: []` ✅ |
| record via `handle` tail (the repro) | 🔴 `Ok (0 bytes)`, 3103-B **bytes** wrapper | ✅ `Diagnostics: []`, 3917-B **artifact** wrapper |

And the FULL effect-based diagnostics shape (emit+collect inside the handler, `(diagnostics (w 2))`) now surfaces
end-to-end on the fresh build → **`compile → Diagnostics: [("CDZ0201","bad"),("CDZ0201","bad")]`** (4182-B
component). That is exactly this ask's acceptance target. (Note: the CLI's `--emit-component`+`component-check`
byte gate and the four-gate suite were NOT re-run by the loop — the compiler agent should confirm those before
publishing.)

**What remains (for the compiler agent):** refresh `implementation/stable/` from the 16:40 build once the four
gates are green + re-stamp `SHA256SUMS`. Then compiler.cdz can activate the `Diag`-handler-wrapped
`compile-output` record (the dormant handler documented in its `compile` docstring) and the ~30 ask-30 rejections
reach `agree` via the effect-based pipeline. **Moved to `pending-validation/`** — the fix is verified against the
running fresh artifact; `done/` awaits the stable publish + gate confirmation + a compiler.cdz activation re-probe.

**✅ LOOP-CORROBORATED 2026-07-07 (Run 99) — boundary flip reproduces exactly; ⚠️ the current stable STILL lacks
the fix.** Independently ran the repro on both toolchains: STABLE (mtime **16:38**, SHA256SUMS OK) → `handle`-tail
`compile-output` record gives `compile → Ok (0 bytes): []` (bytes-ABI fallback, fix ABSENT); LIVE
(`target/release/cadenza-seed`, **16:40**) → same repro gives `compile → Diagnostics: [(CDZ0201,bad),(CDZ0201,bad)]`
(artifact ABI, fix PRESENT). `cmp` confirms the two binaries differ. So the stable snapshot published at 16:38
predates the 16:40 fix by ~2 min and does NOT contain ask-51 — a stable refresh from a ≥16:40 build is still
needed (the 16:38 refresh fell in the gap between the ask-49 fix, which IS in stable, and the ask-51 fix, which is
not). ask-49 confirmed present in stable 16:38 (`ran → Value("b\"\\x03\"")`). Behavior gate 571/0 on stable;
compiler.cdz still bare-Bytes 16:07 so the byte gate is unchanged 65/124/386 (WRONG=0) — the payoff waits on the
handler activation. Learning: `2026-07-07-a-snapshot-can-capture-a-partial-landing-when-fixes-land-minutes-apart`.

# A fix verified on one entry doesn't move a gate driven through another — the compile/run ABI fork has two hops, not one

*2026-07-07*

**What happened.** ask-46 landed: a recursive-effectful `handle` now lowers under the `compile` entry, so the
diagnostics handler can be installed at `compile`. I verified it independently on the refreshed stable seed with
the handoff's own target shape via `compile-run`:

```
(def (compile inputs)
  (record (artifacts (list))
    (diagnostics (handle (list)
      ((D.emit (v) s (resume unit (List.push s (record (code "CDZ0201") (message "bad") (severity 0)))))
       (D.collect (u) s (resume s s))) (w 2)))))
```

→ `compile → Diagnostics: [("CDZ0201","bad"),("CDZ0201","bad")]`. Genuinely fixed. So I expected the byte gate to
finally move — the ~30 ask-30 type-rejections should start carrying `CDZ0201` and reaching `agree`. It did not:
byte gate unchanged at 65 agree / 124 disagree / 386 decline, compiler.cdz still emits the identical 42151-byte
component. The diagnostics handler is NOT yet active in `compile`.

The reason is a second lowering hop that ask-46 didn't cover, filed as **ask-49** and independently reproduced:
the differential GATE drives compiler.cdz through `emit` → `run()`, not through `compile-run`. On that run-entry
path, a recursive-effectful `handle` whose **result value is a runtime compound** (a `list`/`Bytes`/record — as
the diagnostics handle's is) still declines: `recursive effectful function returning a compound / under host
delegation not yet emitted`. The same handle returning a **scalar** works on the run entry (ask-45); the same
compound-returning handle works on the **compile** entry (ask-46). So the capability the compiler needs — install
a diagnostics handler whose collected result is a compound list — is present on one entry and absent on the
other, and the gate exercises the absent one.

**Why.** This is the ask-46 learning's thesis — *a new entry ABI forks a self-hosting compiler's lowering
coverage* — demonstrated a second time and sharpened into a rule about VERIFICATION, not just implementation.
When lowering coverage forks by entry, **the entry you verify a fix on and the entry your gate drives the
artifact through can be different entries**, and then a real, correctly-verified fix moves no gate number. I
verified ask-46 the natural way (`compile-run`, the compile entry — the entry ask-46 is *about*), got a true
green, and it told me nothing about whether the gate would move, because the gate runs the compiler as a `run()`
component. The fix was real; my inference "fix landed ⇒ gate should move" was the error. The two are only linked
if the capability is present on *the gate's* entry, and here it wasn't (ask-49). The discipline this adds to the
loop: **when a fix lands, verify it on the entry the GATE uses, not (only) the entry the fix is described in — a
green on the wrong entry is a true result about the wrong question.** The general shape: for any artifact with
more than one entry ABI, a capability claim is scoped to an entry; "feature X works" must name the entry, and a
downstream consumer only benefits if X works on *its* entry. The self-hosting endgame here needs the
compound-returning effectful handle on BOTH the compile entry (ask-46, done — how the compiler self-hosts) and
the run entry (ask-49, open — how the gate runs the compiler), and only landing both lets the ~30 rejections
reach `agree`.

**The requirement it drove.** No corpus case — this is a run-vs-compile entry ABI lowering gap (a
`compiler.cdz`/seed self-hosting concern), not a value-behavior the `(output (: v T))` oracle expresses; the
effect/handler value semantics are pinned separately. The output is the loop-verification recorded on ask-46
(landed, confirmed on the compile entry via `compile-run`), the independent reproduction recorded on ask-49 (the
run-entry compound-return twin, still open — the actual last hop before the gate moves), and this learning
capturing the cross-cutting rule. General lesson: **when a multi-entry artifact's lowering coverage forks, scope
every capability claim to an entry and verify a fix on the entry your gate actually drives — a correctly-verified
fix on a different entry is a true green that moves no gate, and mistaking it for progress hides the real
remaining hop.**

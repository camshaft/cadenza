## 72. 🔴 (seed) A value definition rejects a leading `(doc …)` — "value def without a single value expression"

**Status: BLOCKING the merged `cdzc.cdz` build** (the last blocker now that ask-71 landed). A FUNCTION
definition accepts a leading `(doc …)` — `(def (f) (doc "…") body)` compiles — but a VALUE definition does
NOT: `(def name (doc "…") value)` declines **"value def without a single value expression"** (the seed's
value-def arm requires exactly `[def, name, value]`, no doc slot — codegen.rs `Def::Value` case,
`items.len() == 3`). A definition is "a value, function, type" (glossary); the doc affordance must not depend
on which form.

**Minimal reproducers (stable seed, `emit`; the asymmetry is the point):**

```
; DECLINES "value def without a single value expression":
(module m
  (def answer (doc "the answer") 42)
  (def (main) answer))

; COMPILES (the function-def control — a leading doc is already accepted here):
(module m
  (def (f) (doc "hi") 42)
  (def (main) (f)))
```

Corpus repro added: `spec/semantics/11-modules.sexp` "a value definition may carry a leading doc, like a
function definition" (→ 42), currently `todo [value def without a single value expression]`.

**Why it's load-bearing NOW.** The rewritten compiler `cdzc.cdz` (built by `implementation/compiler/Makefile`
from `cdzc/*.cdz`) includes the xtask-generated opcode table `cdzc/05-op.cdz` = `(def op (doc "WebAssembly
opcode bytes …") (record …))` — a documented top-level value-def. With ask-71 fixed (top-level value-defs
now bind), THIS `(doc …)` is the sole remaining reason the merged `cdzc.cdz` declines. It is NOT worked
around: stripping the doc from op.cdz's generator, or dropping op.cdz from the merge, would be a contortion
— the generated table legitimately carries a doc, exactly as every other generated def does.

**What the seed needs.** In the value-def parse (the `(def name …)` arm), accept an optional leading
`(doc …)` form between the name and the value — the same doc-skip the function-def arm already does — so a
value def is `(def name [doc]? value)`. The doc is documentation, not part of the value.

**Priority.** 🔴 HIGH — with ask-71 done, this is the single blocker between the merged `cdzc.cdz` and a
clean compile. Verified independently that everything else compiles: the xtask-generated frame value-defs
(`cdzc/40-frame.cdz`) + the full `resolve → … → wrap-component` pipeline produce the byte-identical 89-byte
scalar component; only `op.cdz`'s doc'd value-def declines. Related: ask-71 (top-level value-defs — fixed),
ask-58 (modules-as-records).

**Acceptance signal.** The first reproducer `emit`s a valid component (→ 42), and the merged `cdzc.cdz`
compiles end-to-end (with `op.cdz` in the merge).

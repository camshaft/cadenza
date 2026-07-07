# The byte-level gate's decline discriminator exposed the real self-hosting frontier: the compiler has no type-checker

*2026-07-07*

**What happened.** The prior cycle handed the compiler agent a fix (ask-29): `component-check`'s "496 disagree"
was mostly honest declines (bare-`unreachable` stubs) miscounted as disagreements, so the gate needed a decline
discriminator — classify a component whose entry is a bare `unreachable` as `decline`, not `disagree`. The
discriminator **landed** (seed rebuild). Re-running the byte gate:

```
before:  58 agree, 496 disagree,            204 skip
after:   58 agree, 152 disagree, 344 decline, 204 skip
```

The 344 declines split off cleanly — and the remaining **152 disagreements are now the honest frontier**, no
longer buried. Categorizing them:

- **117** `native=ok / component=ok`, byte-different — the fold-vs-overflow-helper `soft` set (SUCCESS cases,
  values correct: native emits overflow-checked arithmetic helpers, `compiler.cdz` const-folds). Expected.
- **33** `native=rejected / component=ok` — **`compiler.cdz` COMPILES ill-typed programs native REJECTS.**
  Verified directly: `(if true 1 false)` → native `declined: conditional branches have different types`,
  `compiler.cdz` → `Ok (89 bytes)`; `(+ 1 true)` → native `declined: mismatched types`, `compiler.cdz` → `Ok`.
- **2** `native=declined`; **0** where `compiler.cdz` is *more* strict (no false rejections).

The 33 span three diagnostic families: 19 CDZ0201 (conditional branch/condition type errors), 11 CDZ0301
(no-implicit-promotion operand errors), 3 CDZ0210 (non-exhaustive match). The finding: **the self-hosted
compiler has no type-checker.** It reads → resolves → folds → lowers → emits, but runs no type-rejection pass —
it never diagnoses a type error, it just compiles the ill-typed program to a valid-but-should-not-exist
component.

**Why.** This is a reject-don't-miscompile violation at the *whole-program* level — the strongest form, because
the program is *accepted* (emitted as a valid component) rather than declined. And it was **invisible until the
discriminator landed**: buried in "496 disagree" it read as generic reader-coverage noise; with declines split
off, the 33 stand out as a distinct, categorizable class with three named diagnostic families. This is the third
consecutive cycle where a measurement fix immediately paid for itself — the discriminator was scoped to stop
overcounting declines, and its *actual* value was surfacing the next real frontier the overcount had hidden.
That is the general shape worth keeping: **a gate's discriminator does not just make the number honest, it makes
the residue legible** — once the honest-decline noise is subtracted, what remains is the actual work, and here
the residue named itself (type-checking) the moment it was isolated. It also closes the arc of the loop's
recurring theme across three gates (value / trap / byte): each needed the decline-vs-result discriminator, and
the byte gate's — the last and strongest — is the one that revealed the compiler's biggest remaining gap,
because byte-identity is the only gate strict enough to notice that "emitted a valid component" and "should have
rejected" are different outcomes (a value gate sees a plausible value; a trap gate sees nothing; only the byte
gate, comparing against native's *rejection*, sees the compile-that-should-not-be).

**The requirement it drove.** No new corpus case — every one of the 33 is *already* a corpus rejection case
native realizes; the gate measures the gap directly against them, which is the point of the byte-level gate. The
durable outputs: this learning; ask-30 (the missing type-checker, filed high-priority in `asks/open/`), which
names the two coupled sub-gaps — (1) a type-checking pass in `compiler.cdz` (the machinery is half-there: the
return-kind fixpoint `build-ktab`/`kind-of` *computes* kinds but does not *reject* on mismatch), and (2) the
diagnostics ABI (the `compile` export must return `result<list<u8>, list<diagnostic>>` and construct coded
diagnostics, since the compiler's only failure channel today is a trap — so even a type-checker would first move
these 33 from `disagree` to `decline`, and only to `agree` once it can emit the matching `CDZ####`); and the
report to the compiler agent via the `📡 FROM THE CONFORMANCE LOOP` channel. General lesson: **the strictest
gate you can afford is worth the discriminator it demands — byte-identity against a reference that itself rejects
ill-typed programs is the only differential that can catch a missing type-checker, because every weaker gate
accepts the same programs the buggy compiler does.**

---

**Follow-up (2026-07-07, next cycle) — enumerating the 33 refined "missing type-checker" into TWO passes.** A
quiet maintenance cycle (a −1374-byte `compiler.cdz` refactor, byte gate steady at 58/152/344, byte-identical set
verified un-regressed) was spent enumerating the exact 33 `native=rejected / component=ok` cases. They are not
all type errors — the CDZ0201 group splits three ways, and the split changes the shape of the fix:
- **~20 genuine type errors** needing a type-inference/rejection pass: mismatched `if` branches (int/bool,
  int/float), non-Bool `if` condition, non-boolean connective operand, mismatched operator operands, ordering
  int-vs-bool / int-vs-string, a non-list quasiquote splice, all 11 CDZ0301 int-vs-float no-promotion cases, and
  the 3 CDZ0210 non-exhaustive matches.
- **~10 arity / malformed-form errors** needing only a WELL-FORMEDNESS check at read/resolve, NOT type
  inference: a bare keyword (`if`/`=`/`+`/ordering), an operator with the wrong operand count (equality on one
  operand, arithmetic on one, conditional with a missing or extra branch), a binding form with no body.

So the "missing type-checker" is really **two passes of different cost**: a cheap structural arity/well-formedness
check (belongs in the reader/resolver, catches ~10) and a type-inference rejection pass (needs kinds across
branches/operands/match, catches ~20). The refinement matters because the arity check is low-effort and could
land first, moving a third of the 33 without the full inference machinery. General lesson, small but real: **once
a gate isolates a class of failures, enumerate the actual members before scoping the fix — "33 type errors" was
really "≈20 type errors + ≈10 arity errors," two passes not one, and the cheaper half is separable.** The count
was right; the *shape* was wrong until enumerated, the same over-generalization trap as the gap-3n "parity"
misread, now avoided by listing the cases instead of sampling them.

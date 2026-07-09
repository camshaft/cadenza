## 23. 🟢 The self-hosted reader miscompiles unsupported constructs instead of declining — RESOLVED (compiler side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (compiler side).** All three facets of the atom-decode leak now route to `KError` →
`unreachable` (a clean decline), verified by reading `read-node` and by the harness (`hard` 3→0, those cases
moved to `decline`; `01-literals` error 1→0): (1) major-7 `read-node` accepts ONLY the two bool encodings
(info 20/21) and sends float/null/other → the unknown-marker → `KError` (compiler.cdz:664); (2) the unbound
name-reference — a name whose prelude index isn't a parameter — declines instead of emitting `NLocal -1` /
`local.get -1` (compiler.cdz:649–653); (3) any other major (bytes/text/map) → the same marker
(compiler.cdz:665). The remaining reader-side decline-don't-miscompile work — a distinct `error` bucket where
the emission is *invalid* rather than a clean trap — is item **25** (entry selection), NOT this atom-decode
family. Original finding kept below.

**Finding.** `compiler.cdz`'s reader **never declines** an unsupported construct — it emits a
valid-but-WRONG component. Verified: a CBOR float `0xfb` (major 7, info 27) hits `read-node`'s major-7
branch, which assumes a boolean (`arg == 21`?), so `arg 27 ≠ 21` → `NBool 0` → the program returns
`false`. Strings / records / tuples / bytes-ops / host calls have no reader node, so they fall through
to `NInt` / an `NPrim`-of-`"?"` stub. The harness's `0 mine-declines` is this: the compiler always
emits something. This is a reject-don't-miscompile violation *inside the Cadenza-authored compiler*.

**Why it touches the seed/compiler.** Decline-don't-miscompile is a core discipline the spec mandates
for every generation; the compiler's *reader* leaks it on the atom-decode path. It is *unsafe* coverage
— a silent wrong-but-valid component passes a naive "did it build?" check, and when the compiler
eventually compiles its own source a miscompiled construct yields a subtly-wrong compiler, not a clean
failure. The reader already declines an unrecognized *operator head* correctly (`PUnknown → KError →
unreachable`); the atom/literal decode must do the same.

**Status.** ⚪ `compiler.cdz` work (the reader), mirrored by SEED-GAPS. **Corpus:** pinned the
discriminating seed-level fact — `10-bytes.sexp` *"a CBOR simple value that is not a known boolean is
classified as not-a-boolean"* (a major-7 decoder must check the value IS `0xF4`/`0xF5`, not merely
`≠ 0xF5`, so a float/null head is not read as false; three-way classify → -90). Fix: route the reader's
unrecognized major-7 (and any unhandled atom kind / node shape) to `KError`, not a defaulted
`NBool`/`NInt`. **Acceptance signal:** the harness's `mine-declines` count rises from 0 to the number of
unsupported constructs as the reader learns to decline them (and DISAGREE falls correspondingly).
Learning: `spec/learnings/2026-07-07-the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining.md`.

**Update (2026-07-07) — a THIRD facet: the unbound name-reference.** `read-node`'s tag branch (major 6)
resolves a name to `(Node.NLocal (ienv-pos env idx 0))`, where `ienv-pos` returns **-1** for a name not
in the parameter/let environment — used directly as a local slot index with no bounds check. So an
*unbound* name-reference decodes to `NLocal -1` → `KLocal -1` → an invalid `local.get` (uleb of -1 is a
huge index; a validation error or a wrong local — a miscompile either way), rather than a decline. This
is the same violation class as the float→false and string→stub facets (a fall-through to a wrong node),
so the fix is the same: when `ienv-pos` returns -1 (unbound), route to `KError`, not `NLocal -1`. Adds
the name-reference to the list of reader paths that must decline rather than miscompile.

---

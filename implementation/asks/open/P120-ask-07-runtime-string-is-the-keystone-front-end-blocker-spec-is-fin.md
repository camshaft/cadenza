## 7. ⚪ Runtime `String` is the keystone front-end blocker (spec is fine; seed + realized-set work)

**Finding.** Name dispatch (comparing a head against `"def"`, `"+"`, …) and the reader's symbol table
both need runtime `String`; all `String.*` is const-fold-only today. Not a spec *gap* (the string
capability is specified), but the operator should know it is the gate to a true `bytes → bytes` front
end, and that the built-in `Ast`/`quote` is a **dead end for self-hosting** (`quote` won't flow
through a function call; `Ast.*` ctors are unusable at runtime) — so the compiler decodes the CBOR
input into its **own user-declared `Node` sum**, which recurses through calls fine.

**Status.** 🟢 **DONE (2026-07-07) — the keystone landed.** Runtime `String` now works in the seed: all
four Tier-0 probes compile and run (string fn parameter → `String.byte-len` = 5; runtime `=` dispatch →
1; string returned across a call → `"hello"`; string sum-payload bound by a `match`). The compiler's
front rung now resolves a head by NAME (`head-prim` maps `"+"` → a `Prim` code; no string survives into
Core; unknown head → `PUnknown` → decline). Never a spec *gap* (the string capability was always
specified) — it was seed + realized-set work, now complete. Pinned by the runtime-string cases in
`13-strings.sexp` (all green) plus the new multi-way head-dispatch case. Learning:
`spec/learnings/2026-07-07-runtime-strings-unblock-the-name-based-front-rung.md`. **Remaining front-end
critical path:** decoding arbitrary-arity forms (needs the nested-payload binder — backlog #1) and the
CBOR reader / symbol table.

---

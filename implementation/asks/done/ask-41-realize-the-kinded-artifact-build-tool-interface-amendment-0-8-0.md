## 41. 🟡 Realize the kinded-artifact build-tool interface (Amendment 0.8.0) — `compile: list<artifact> → {artifacts, diagnostics}`

**Spec landed (2026-07-07, Amendment 0.8.0).** The frozen `build-tool-interface.md` derivation entry was
generalized from the two-arm `result<list<u8>, list<diagnostic>>` to a **kinded-artifact interface**:
```
compile: func(inputs: list<artifact>) -> compile-output
record compile-output { artifacts: list<artifact>, diagnostics: list<diagnostic> }
record artifact       { kind: string, bytes: list<u8> }
record diagnostic     { severity, code, message }        // severity: error | warning | …
```
The derived component is ONE output artifact (by `kind`); DWARF / source map / manifest are other
artifacts of the same shape. Inputs are the same list (the AST is the `ast`/`source` artifact); a build
cache or a multi-file program's extra source units / dependencies are more input artifacts. Success =
a component artifact present + no error-severity diagnostic; a warning is a non-error diagnostic returned
ALONGSIDE a produced component. Rationale + migration path:
`spec/learnings/2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result.md`.

**Why (operator, this cycle).** Three limits of `result<bytes, diags>`, each a real need: (1) no
warnings alongside a produced module (result is either/or); (2) no sidecar byte outputs — DWARF, source
map, manifest; (3) no more than one input — multi-file imports, an incremental build cache, a derived
dependency. The artifact-list shape subsumes all three and is additively extensible (new kinds don't
change the arity).

**Migration path (in force now).** The REALIZED seed interface stays `compile: list<u8> →
result<list<u8>, list<diagnostic>>` — the degenerate single-input / single-output case (input = one `ast`
artifact; success = the `component` artifact with no error diagnostics; failure = the diagnostics list).
This is landed and gate-green ([[diagnostics-abi-result-envelope]],
[[diagnostics-abi-branch-detection-and-artifacts-direction]]) and unblocks the compiler agent's faithful
rejection TODAY. So this ask is NOT blocking self-hosting — it is the follow-on that realizes the fuller
interface when DWARF / cache / multi-file imports become concrete work.

**Realization scope (seed + spec, when prioritized):**
1. **xtask envelope** — a `build_compile_artifacts_reference` lifting `compile: func(list<artifact>) →
   compile-output`, with the `artifact` + `diagnostic` (severity) + `compile-output` record types
   (each non-primitive record EXPORTED as a named type — the wasmparser `all_valtypes_named` gotcha from
   [[diagnostics-abi-result-envelope]]). New HEAD/TAIL consts.
2. **Wrapper** — read the input `list<artifact>` into runtime artifacts; call the user body (returns a
   runtime `compile-output` record of `{artifacts, diagnostics}`); marshal both lists to the canonical
   retptr layout. Reuses the string/list/record marshaling helpers (`emit_marshal_string_into`,
   `emit_bytes_copy_loop`) already built for the result envelope.
3. **Selection** — detect a `compile-output`-returning body (a record with `artifacts`/`diagnostics`
   fields) via the tail-walk (`compile_body_is_result`'s generalization).
4. **Corpus / gates** — a `(def (compile inputs) …)` returning `{artifacts, diagnostics}`; verify
   Ok-with-warnings, multi-artifact output, multi-artifact input.
5. **diagnostics-schema** — `diagnostic` gains a `severity` field (error | warning | …); reconcile with
   `spec/contracts/diagnostics-schema` / options.

**Acceptance signal.** A `compile` body that returns a produced component artifact + a warning diagnostic
round-trips through the host as both (component present, warning reported); a multi-input `list<artifact>`
(source + cache) is accepted; an unrecognized input kind is a diagnostic, not a silent drop.

**Status.** 🟡 **Seed + spec, deprioritized** — the `result<>` degenerate case is realized and unblocks
the immediate self-hosting need; this generalizes it when multi-output (DWARF) / multi-input (cache,
imports) become real. Related: ask-40 (the `result<>` interface this generalizes), Amendment 0.8.0.

**⬆️ PRIORITY UP 2026-07-07 — realizing this ALSO closes ask-42 (the diagnostics-wiring blocker), so it is on
the self-hosting critical path, not just a follow-on.** ask-42: wiring compiler.cdz's rejection path to the
`result<bytes, diags>` ABI mis-lowers — a Result-returning `compile` whose call graph chooses between an
`Ok`-arm and an `Err`-arm via a deep sum-match returns the wrong value (the seed's Result-shape analysis can't
reconcile the differently-typed branches). The kinded-artifact interface AVOIDS this by construction:
`{artifacts, diagnostics}` is ONE record type on BOTH success and rejection, so the choosing `if`/`match` has
same-shape branches. VERIFIED (seed 13:51): `(def (compile inputs) (if <deep-sum-match> (mkdiag-record)
(mkartifact-record)))` — the exact ask-42 trigger shape — COMPILES VALID and does NOT decline under the record
return (host reads `Ok (0 bytes)` only because the artifact ABI isn't decoded yet). So realizing the seed side
(envelope + wrapper + selection, §Realization scope 1–3) is the clean way to let compiler.cdz report `CDZ0201`
diagnostics and move the ~30 ask-30 rejections `decline → agree`. Seed status: NOT yet realized — a `(def
(compile inputs) (record (artifacts …) (diagnostics …)))` body compiles VALID but the host decodes it as the
plain `list<u8>` path (`Ok (0 bytes)`), and there is no `build_compile_artifacts` envelope in `crates/`.

---

## ✅ DONE 2026-07-07 (conformance loop) — FULL SYMMETRIC ABI realized + RE-PROBED

The full symmetric kinded-artifact ABI is landed and verified end-to-end (not just the degenerate case).
`compile: list<artifact> → compile-output{artifacts, diagnostics}` — input `list<artifact>` unmarshalled,
`compile-output` record marshalled out, host decodes it (picks the `component`-kind artifact; error-severity
denies it; warnings ride alongside).

**Re-probed shapes (all via `compile-run` + 3 new `tests/compile_probes.rs` probes):**
- warning (severity 1) + component artifact → the component bytes (`Ok`), warning does not deny;
- error (severity 0) + component artifact → `Diagnostics` (component denied);
- multi-artifact output (`dwarf` sidecar + component) → component selected BY KIND;
- input `list<artifact>` (AST as one `{bytes, kind:"ast"}` artifact) marshalled without trapping.

**Root bug fixed en route:** the shared `cabi_realloc`'s alignment arg is canonical param index 2, but the
seed read index 1 — every NESTED wasmtime-driven input allocation collapsed to address 0 (invisible on the
single-allocation bytes ABI, fatal for the nested `list<artifact>` input). Canonical order now everywhere.

**Gates:** BEHAVIOR 570/0, IGNITION byte-identical, COMPONENT-CHECK 575 agree/0 disagree, cargo test green.
Spec: added "Every Diagnostic Carries A Severity" to `spec/capabilities/diagnostics.md`. 📦 STABLE binary
refreshed. Learning: `kinded-artifact-abi-and-cabi-realloc-arg-order`. Handoff banner atop SEED-GAPS tells
compiler.cdz exactly how to wire its `{artifacts, diagnostics}` return (closes ask-42/40/30 by construction).

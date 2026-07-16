# rcdzc feature-gap: the String-crossing matrix blocks any runnable model-call `(String -> String)` shape

**Filed by:** v-agent-harness, 2026-07-16. **Kind:** honest feature-limitations (deliberate declines,
NOT soundness bugs — each rejects at the binding to avoid emitting an invalid component). **Owner seam:**
v-peer-linking / cross-component-interop (the boundary ABI). **Supersedes/extends:**
`issues/host-op-cannot-return-string-or-compound-result.md` (that is ONE cell of the matrix below).

## What I probed (built `cdz`/`cdz-run` @ trunk `e1506bd7c`, ran e2e)

A Bedrock/model call is fundamentally `(String prompt -> String completion)`. I probed every way a
`String` can cross the component boundary. **Only one crossing works today**; the model-call shape needs
several that don't:

| # | crossing | works? | diagnostic / reason |
|---|----------|--------|---------------------|
| 1 | peer op **RESULT** = String | ✅ **YES** (ran e2e; consumed via `String.byte-len` → 11) | `extern_abi_val_type` → U32 runtime handle |
| 2 | peer op **ARG** = String/Bytes | 🔴 NO | `diag.rs::STRING_ARG_ACROSS_PEER_MESSAGE` — inbound rope handle not yet emitted; arg would lower as a component `string` needing a `mem` canon-opt the peer envelope lacks → invalid component |
| 3 | entrypoint **RESULT** escapes String/compound (a peer-bound op reached under it) | 🔴 NO | `select.rs:~8544` — resource-escape emit paths (`emit_runtime_resource`/`emit_recursive_sum_resource`) carry the runtime import but not the peer extern envelope |
| 4 | host op **ARG** = String (non-const) | 🔴 NO | "a host call with a non-constant string argument is not yet emitted" |
| 5 | host op **ARG** = String (const literal) | ✅ yes | the const-fold path (`HostParam::Str`); the earlier "string param works" was THIS |
| 6 | host op **RESULT** = String/compound | 🔴 NO | `abi_val_type` → None (my prior issue) |
| 7 | entrypoint **PARAM** = String | 🔴 NO | "type `String` has no component boundary representation (only aliased int widths cross)" |

## The conclusion that matters

**A `String` crosses NO boundary in a runnable model-call shape today — only a peer RESULT works.** So:
- **Route B (Bedrock as a Cadenza peer):** declines on the PROMPT ARGUMENT (#2) and on returning the
  completion from `main` (#3). Blocked without #2.
- **Route A (Bedrock as a host op):** declines on the RESULT (#6) and a non-const prompt ARG (#4).
  Blocked without #6 (+#4).
- **Neither route gives `String -> String` without ABI work.** This is ONE coherent gap area (a rope
  crossing the boundary, in both directions, host AND peer, plus the entrypoint edges) — v-peer-linking's
  territory.

## Critical-path for the agent harness (the smallest unblock)

**Cell #2 (peer String ARGUMENT) is the critical path.** With #2 built, Route B works: prompt crosses IN
as a peer arg, completion comes back as a peer RESULT (#1, already works) and is consumed IN-PROGRAM
(parsed into tool-calls) rather than returned from `main` — which sidesteps #3. So the harness does NOT
need #3 immediately if it consumes the completion internally.

Fix sketch for #2 (from `diag.rs` doc): wire the inbound-rope-handle emit so a peer op's String/Bytes
arg crosses as a runtime handle (the mirror of the working RESULT path #1), instead of lowering as a
component `string` that needs a `mem` canon-option the peer envelope doesn't supply.

## Repros

At `/tmp/inc1a/` this session (and reproducible):
- `#1 works`: peer `(do (def (converse (: seed Int64)) "model-reply") (export converse))` +
  `--component-name cadenza:bedrock/api`; consumer `(String.byte-len (host (Bedrock) (Bedrock.converse
  seed)))` bound to it → runs to 11.
- `#2 declines`: consumer `(-> String String)` `converse` → CDZ0201 String-arg-across-peer.
- `#3 declines`: consumer returning the peer's String from `main` → resource-escape-no-peer-import.
- `#4 declines`: host op `(-> String Int64)` with a non-const arg.
- `#7 declines`: bare `(def (main (: p String)) …)` entrypoint.

## Cross-refs

- Design: `implementation/design/DESIGN-agent-harness.md` §2 (corrected this session to reflect the
  matrix — §2 previously implied the peer String path was symmetric; it is NOT — only the RESULT).
- Prior cell: `issues/host-op-cannot-return-string-or-compound-result.md` (#6).
- Diagnostics: `rcdzc/src/diag.rs::STRING_ARG_ACROSS_PEER_MESSAGE` (#2);
  `rcdzc/src/backend/wasm/select.rs:~8544` (#3); `backend/wasm/host.rs::abi_val_type` (#6).

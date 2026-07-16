# PR review comment — mirrored from GitHub PR #407 (Copilot inline)

- **PR:** #407 (MERGED)
- **File:** `guide/src/playground/share.ts:12`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591164520
- **Link:** https://github.com/camshaft/cadenza/pull/407#discussion_r3591164520

## Comment (verbatim)
> This uses a default import from `lz-string`, but the guide's TS configs don't enable `esModuleInterop` / `allowSyntheticDefaultImports` (and `verbatimModuleSyntax` is on). If `lz-string` has no real default export, `tsc -b` will fail even though Node may run it. A namespace import keeps runtime compatibility with Node's strict ESM loader while also type-checking against named-export typings.

## Liaison triage — CONFIRMED tension against trunk (verify tsc -b)
This change (the very commit `guide: share.ts default-import lz-string (CJS-safe)`) has an EXPLICIT
comment defending the default import for RUNTIME: "lz-string is a CommonJS module: a NAMED import fails
under a strict ESM loader... The default import gets the whole module.exports object, which works under
both Vite and node." Copilot's counter is a TYPECHECK concern: `guide/tsconfig.app.json` sets
`verbatimModuleSyntax: true` and there's NO `esModuleInterop`/`allowSyntheticDefaultImports` (confirmed
by grep), so `tsc -b` may reject `import LZString from "lz-string"` if lz-string has no real default
export. Genuine runtime-vs-typecheck tension. RESOLUTION the reviewer suggests: a namespace import
(`import * as LZString from "lz-string"`) satisfies BOTH — runtime (gets module.exports) and typecheck
(against named-export typings). Guide territory (v-guide). ACTION: verify whether `tsc -b` / the guide
build actually passes today; if it does, dismiss; if it fails (or CI's typecheck is lenient and would
break on a stricter run), switch to a namespace import. Fix on `trunk` if needed. Quote + link in queue
file.

<!-- WONTFIX 2026-07-16 (v-guide-infra): FALSE POSITIVE — the default import is CORRECT under moduleResolution:bundler; a namespace import would break the node runtime. Concierge backlog filed with the rationale. -->

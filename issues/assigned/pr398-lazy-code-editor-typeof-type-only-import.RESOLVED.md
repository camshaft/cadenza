# PR review comment — mirrored from GitHub PR #398 (Copilot inline)

- **PR:** #398 "fleet: twenty-fourth batch (Types-as-values recovery, memory-safety, open-sums, LSP, broad corpus)" (MERGED)
- **File:** `guide/src/editor/LazyCodeEditor.tsx:16`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590785473
- **Link:** https://github.com/camshaft/cadenza/pull/398#discussion_r3590785473

## Comment (verbatim)
> `CodeEditorType` is imported with `import type`, so it's a type-only symbol; `ComponentProps<typeof CodeEditorType>` will fail because `typeof` in a type query requires a value symbol. Use a type query on a dynamic import of the module (so CodeEditor stays lazily loaded) and drop the type-only component import.

## Liaison triage — CONFIRMED against trunk
Confirmed: `import type { CodeEditor as CodeEditorType } from "./CodeEditor.tsx";` then
`type CodeEditorProps = ComponentProps<typeof CodeEditorType>;`. `typeof X` in a type query needs `X`
to be a VALUE binding, but `import type` erases it to a type-only symbol — a TS error (and if it compiles
under a loose config, it's still incorrect). Fix: `ComponentProps<typeof import("./CodeEditor.tsx")["CodeEditor"]>`
(a type query on the dynamic import, keeping CodeEditor lazy) and drop the `import type` line. Guide
territory (v-guide). Fix on `trunk`. Quote + link in queue file.

<!-- RESOLVED 2026-07-16 (trunk@b706d3b76, v-guide-infra): LANDED + verified by file content on trunk. -->

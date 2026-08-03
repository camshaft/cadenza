# PR #1155 review comment — guide/src/content/chapters/ExampleApps.tsx (v-guide-editor)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1155
(PR: "cand: v-guide-editor — (oldest-first)").

## Units example numeric form inconsistent with Units chapter + UI (Copilot, ExampleApps.tsx:82) — doc nit
> The units example here uses `<C>5 meter</C>`, but the Units chapter consistently illustrates
> quantity rendering with a decimal (e.g. `<C>5.0 meter</C>`). To keep documentation consistent with
> the referenced chapter and the UI's displayed format, use the same numeric form here.

Minor cross-chapter consistency: use `5.0 meter` to match the Units chapter and the UI's actual
displayed format.

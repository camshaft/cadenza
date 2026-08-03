# PR#1023 — guide arc.test.ts: tighten SECTION_ORDER/inPillar from `string` to the Pillar union (v-guide-editor)

One Copilot review comment on the guide TS test. `guide/src/content/arc.test.ts` → v-guide-editor.
Gate = Node22(mise) + guide TS via `tsc -b` (NOT `--noEmit`; app config sets noUnusedLocals).

## Comment (verbatim) — arc.test.ts:26 (id 3696321766)

- "`SECTION_ORDER` and `inPillar` are typed with plain `string`, which loses type-safety around valid
  pillars and forces a lot of `Record<string, …>` indexing. Since `pillarOf(c)` already defines the
  pillar domain, you can tighten types with `ReturnType<typeof pillarOf>` to catch typos at compile time."

## Liaison verification (confirmed on trunk 81e0f587b)

arc.test.ts:25 `const SECTION_ORDER: Record<string, string[]>` and :39 `const inPillar = (pillar:
string): Chapter[] => …`. `pillarOf` (chapters.ts:37) returns the union `type Pillar = "language" |
"platform"` (chapters.ts:14). So Copilot's tightening is valid: keying `SECTION_ORDER` and typing
`inPillar`'s param by `Pillar` (or `ReturnType<typeof pillarOf>`) would make a mistyped pillar key a
compile error instead of a silent runtime miss. Note `Pillar` is already exported from chapters.ts, so
`import { …, type Pillar }` is cleaner than the `ReturnType<>` form — v-guide's call which spelling.
Minor / type-hygiene; the current code is CORRECT (just loosely typed) — this is polish, not a bug.

Owner: **v-guide-editor** (`guide/src/content/arc.test.ts`). Optional type-tightening — key `SECTION_ORDER`
and type `inPillar`'s param by the `Pillar` union (import it from chapters.ts) so an invalid pillar string
fails at compile time. Not a correctness bug; v-guide's discretion whether the polish is worth it.

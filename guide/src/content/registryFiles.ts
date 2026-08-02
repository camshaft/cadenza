/// Shared registry parse for the content tests: `slug → chapter TSX filename`, read out of `chapters.ts`'s
/// lazy imports. Every narrative test (arc / opener / tenets / links / forwardRefs / pillarBridge /
/// chapters / exercises) needs this same slug→file map to open a chapter's source and scan its prose, and
/// each used to carry its own verbatim copy of the regex + loop. That duplication drifted (the regex was
/// bounded in one file but not the others after a ReDoS review) — the same copy-paste-drift class as
/// NON_TEACHING_SECTIONS. This is the single source of truth; all of them import it.
///
/// The gap between a `slug:` and its `import(...)` is BOUNDED ({0,500}?, not an unbounded lazy `[\s\S]*?`):
/// a slug with no following import would otherwise backtrack the whole remaining file per exec
/// (catastrophic on a malformed registry — amazon-q flagged this on PR #1166). Every real entry's gap is
/// well under the bound (the longest, platform-safety's blurb, is 316 chars), so 500 is behaviour-preserving
/// with comfortable margin.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url)); // src/content

/// slug → its chapter TSX filename (e.g. `"welcome" → "Welcome.tsx"`), parsed from `chapters.ts`'s
/// `slug: "…" … import("./chapters/File.tsx")` entries. A fresh Map per call (tests may mutate/inspect).
export function fileForSlug(): Map<string, string> {
  const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");
  const re = /slug:\s*"([^"]+)"[\s\S]{0,500}?import\("\.\/chapters\/([^"]+)"\)/g;
  const m = new Map<string, string>();
  for (let x = re.exec(registrySrc); x; x = re.exec(registrySrc)) m.set(x[1], x[2]);
  return m;
}

/// Internal-link integrity for the guide's prose. Cross-chapter links (<Ch to="/effects">, the
/// <Link>s in WhatsNext / Playground / HomePage / Layout) are the narrative's connective tissue —
/// they carry the reader along the arc. A chapter rename, a removed slug, or a typo'd href turns one
/// into a dead route that silently 404s at runtime; nothing else in the suite catches it. This test
/// pins the invariant "every internal link points at a real route" so a future content edit that
/// drifts a slug fails here instead of shipping a broken cross-reference. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");
const componentsDir = join(here, "..", "components");
const mainTsx = join(here, "..", "main.tsx");

/// The set of routes a `to="/…"` may legitimately point at: the site root, every top-level app route
/// declared in main.tsx (path: "/x"), and one per chapter slug (served by the `/:slug` catch-all).
/// Deriving both halves from source (not a hard-coded list) means the test tracks the real router.
function validRoutes(): Set<string> {
  const routes = new Set<string>(["/"]);
  const mainSrc = readFileSync(mainTsx, "utf8");
  // `path: "/playground"` etc. Skip the dynamic `/:slug` param route — chapter slugs cover it below.
  for (const m of mainSrc.matchAll(/path:\s*"(\/[^"]*)"/g)) {
    if (!m[1].includes(":")) routes.add(m[1]);
  }
  for (const c of CHAPTERS) routes.add(`/${c.slug}`);
  return routes;
}

/// Every `to="/…"` string literal in a source file, with the line number for a legible failure. The
/// interpolated form `to={…}` (used inside the <Ch>/<Link> wrapper *definitions*) is intentionally
/// not matched — those forward a literal passed at the call site, which this scan sees separately.
function internalLinks(src: string): { href: string; line: number }[] {
  const out: { href: string; line: number }[] = [];
  const lines = src.split("\n");
  for (let i = 0; i < lines.length; i++) {
    for (const m of lines[i].matchAll(/to="(\/[a-z0-9/-]*)"/g)) {
      out.push({ href: m[1], line: i + 1 });
    }
  }
  return out;
}

function tsxFilesIn(dir: string): string[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".tsx"))
    .map((f) => join(dir, f));
}

test("every internal to=\"/…\" link points at a real route (chapter slug or app route)", () => {
  const routes = validRoutes();
  const files = [...tsxFilesIn(chaptersDir), ...tsxFilesIn(componentsDir)];
  const dead: string[] = [];
  for (const file of files) {
    const src = readFileSync(file, "utf8");
    for (const { href, line } of internalLinks(src)) {
      if (!routes.has(href)) {
        dead.push(`${file.replace(here, "src/content")}:${line} → ${href}`);
      }
    }
  }
  assert.equal(
    dead.length,
    0,
    `dead internal link(s) — the target route does not exist:\n  ${dead.join("\n  ")}`,
  );
});

test("the link scan actually found links (guards against a broken regex silently passing)", () => {
  // A no-op scan would pass the integrity test vacuously. Assert we see a healthy number of links so
  // a future refactor that breaks the extraction (or moves the files) trips this instead of hiding.
  const files = [...tsxFilesIn(chaptersDir), ...tsxFilesIn(componentsDir)];
  const total = files.reduce((n, f) => n + internalLinks(readFileSync(f, "utf8")).length, 0);
  assert.ok(total >= 20, `expected many internal links across content, found ${total}`);
});

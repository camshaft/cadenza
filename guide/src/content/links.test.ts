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
import { fileForSlug } from "./registryFiles.ts";

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

/// The standalone APP routes (playground / calculator / cad / notebook …) — every top-level
/// `path: "/x"` in main.tsx that is NOT the site root, the `/:slug` param route, or a chapter slug.
/// These are the showcase apps the guide's "Example applications" gallery is meant to gather; deriving
/// them from the router (not a hard-coded list) means adding a new app route automatically extends the
/// gallery-coverage invariant below.
function appRoutes(): string[] {
  const chapterRoutes = new Set(CHAPTERS.map((c) => `/${c.slug}`));
  const mainSrc = readFileSync(mainTsx, "utf8");
  const out: string[] = [];
  for (const m of mainSrc.matchAll(/path:\s*"(\/[^"]*)"/g)) {
    const p = m[1];
    if (p === "/" || p.includes(":") || chapterRoutes.has(p)) continue;
    if (!out.includes(p)) out.push(p);
  }
  return out;
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

// A cross-chapter link is a "see X" promise: it should carry the reader to a DIFFERENT chapter. A chapter
// whose prose links to its OWN slug (`Effects.tsx` containing `<Ch to="/effects">`) passes the "real route"
// check above — the route exists — but it reads to the reader as a cross-reference that goes nowhere new, a
// dead-end pointer at the page they're already on. It's the kind of drift a copy-paste of a link block, or a
// chapter rename that lands a link on its own new slug, introduces silently. The "real route" test can't
// catch it (a self-slug IS a real route); this pins that a chapter's chapter-links always point elsewhere.
test("no chapter's prose links to its OWN slug (a self-link is a dead-end cross-reference)", () => {
  const selfLinks: string[] = [];
  for (const [slug, file] of fileForSlug()) {
    // A chapter-slug link back to `/${slug}` is a self-reference; an app-route link (/cad) never is.
    for (const { href, line } of internalLinks(readFileSync(join(chaptersDir, file), "utf8"))) {
      if (href === `/${slug}`) selfLinks.push(`${file}:${line} → /${slug} (its own chapter)`);
    }
  }
  assert.equal(
    selfLinks.length,
    0,
    `self-link(s) — a chapter cross-links to its own slug, pointing the reader nowhere new:\n  ${selfLinks.join("\n  ")}`,
  );
});

test("the link scan actually found links (guards against a broken regex silently passing)", () => {
  // A no-op scan would pass the integrity test vacuously. Assert we see a healthy number of links so
  // a future refactor that breaks the extraction (or moves the files) trips this instead of hiding.
  const files = [...tsxFilesIn(chaptersDir), ...tsxFilesIn(componentsDir)];
  const total = files.reduce((n, f) => n + internalLinks(readFileSync(f, "utf8")).length, 0);
  assert.ok(total >= 20, `expected many internal links across content, found ${total}`);
});

// The "Example applications" gallery chapter is the guide's one narrative gateway to the showcase apps —
// its whole job is to send the reader into every standalone app (playground, calculator, CAD, notebook).
// If a new app route is added to the router but not linked here (or an app link is dropped from the
// chapter), the gallery silently omits an app: it ships in the router, reachable only by typing the URL,
// with no narrative path in. Nothing else in the suite would catch that. Pin it: the gallery chapter must
// link every app route the router declares. (The chapter is identified by its registered slug, so a
// rename tracks automatically; deriving the app set from main.tsx means a new app extends this by itself.)
test("the Example-applications gallery links every standalone app route the router declares", () => {
  const gallery = CHAPTERS.find((c) => c.section === "Example applications");
  assert.ok(gallery, "no chapter in the 'Example applications' section — the gallery is missing");
  const file = fileForSlug().get(gallery!.slug);
  assert.ok(file, `no TSX import found for the gallery chapter slug ${gallery!.slug}`);
  const linked = new Set(internalLinks(readFileSync(join(chaptersDir, file!), "utf8")).map((l) => l.href));
  const missing = appRoutes().filter((r) => !linked.has(r));
  assert.equal(
    missing.length,
    0,
    `the Example-applications gallery (${file}) does not link these app route(s): ${missing.join(", ")} — every showcase app needs a narrative path in`,
  );
});

test("appRoutes finds the showcase apps (guards against a broken router scan)", () => {
  // A broken scan would make the gallery-coverage test pass vacuously (0 missing of 0). Assert we see the
  // known showcase apps so a regex/refactor break trips here instead of hiding.
  const routes = appRoutes();
  for (const r of ["/playground", "/calculator", "/cad", "/notebook"]) {
    assert.ok(routes.includes(r), `expected app route ${r} among the router's standalone routes; found ${routes.join(", ")}`);
  }
});

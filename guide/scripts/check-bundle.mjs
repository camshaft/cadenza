/// Bundle-weight guard for the guide's FIRST PAINT. The entry chunk (index-*.js) is what every visitor
/// downloads before anything renders, so a heavy dep leaking into it is a real, silent regression. This
/// asserts two invariants over a fresh `dist/`:
///   (1) the entry chunk stays under a size ceiling — a tripwire for ANY heavy dep accidentally landing in
///       first paint (a static import of three.js / katex / manifold / a big lib);
///   (2) KaTeX specifically stays OUT of the entry — it's lazy-loaded by <Math> (dynamic import), so its
///       code must live in a SEPARATE async chunk, never the entry. Chapters render on eager routes, so a
///       static katex import would bloat first paint for every reader (the reason <Math> lazy-loads it).
/// Run AFTER `npm run build` (reads dist/). Fails loud with the offending sizes.

import { readdirSync, statSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const assets = join(here, "..", "dist", "assets");

// The entry chunk (Vite names it `index-<hash>.js`) — the code every visitor loads before first paint.
// Ceiling chosen with headroom over today's ~90KB, low enough to catch a heavy dep (three.js ~600KB,
// katex ~270KB) leaking in. Bump deliberately (with a note) if the guide's core legitimately grows.
const ENTRY_MAX_BYTES = 150 * 1024;

let files;
try {
  files = readdirSync(assets);
} catch {
  console.error("\n✗ check:bundle FAILED — dist/assets not found. Run `npm run build` first.");
  process.exit(1);
}

const failures = [];

const entry = files.find((f) => /^index-.*\.js$/.test(f));
if (!entry) {
  failures.push("no entry chunk (index-*.js) in dist/assets — did the build succeed?");
} else {
  const bytes = statSync(join(assets, entry)).size;
  if (bytes > ENTRY_MAX_BYTES) {
    failures.push(`entry chunk ${entry} is ${(bytes / 1024).toFixed(1)}KB — over the ${(ENTRY_MAX_BYTES / 1024).toFixed(0)}KB first-paint ceiling (a heavy dep may have leaked into first paint)`);
  } else {
    console.log(`  ✓ entry chunk ${entry} is ${(bytes / 1024).toFixed(1)}KB (under the ${(ENTRY_MAX_BYTES / 1024).toFixed(0)}KB first-paint ceiling)`);
  }
  // KaTeX must NOT be in the entry chunk — <Math> lazy-loads it, so it belongs in an async chunk only. A
  // reliable signature: KaTeX ships its version as `version:"0.16..."` and the distinctive `\\KaTeX`
  // command string + `ParseError` class. If the entry carries KaTeX's parser, it was statically imported.
  const entrySrc = readFileSync(join(assets, entry), "utf8");
  const katexInEntry = entrySrc.includes("\\KaTeX") || /KaTeX parse error/.test(entrySrc);
  if (katexInEntry) {
    failures.push(`the entry chunk ${entry} contains KaTeX code — it must be lazy-loaded (dynamic import in <Math>), NOT statically imported into first paint`);
  } else {
    console.log(`  ✓ KaTeX is not in the entry chunk (lazy-loaded by <Math> into its own async chunk when a page renders math)`);
  }
}

if (failures.length) {
  console.error("\n✗ check:bundle FAILED — first-paint bundle regressed:\n" + failures.map((f) => "  ✗ " + f).join("\n"));
  process.exit(1);
}
console.log("\n✓ check:bundle: the first-paint entry chunk is within budget + heavy deps (KaTeX) stay lazy.");

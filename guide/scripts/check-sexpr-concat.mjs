/// Guide .sexp PROSE string-concat footgun lint (v-guide-editor's I5 ask). When prose is wrapped across
/// several adjacent string atoms — e.g. (p "…the " "platform…") — a DROPPED trailing space silently joins
/// two words ("theplatform"). The check:codegen DOM-fidelity gate CANNOT catch this: the join produces
/// valid-but-wrong TEXT, and both spellings render as valid DOM. This lint catches it at the source: any two
/// CONSECUTIVE string atoms in a list where the first ends in a letter/digit and the second starts with a
/// letter (the classic missing-space-at-the-wrap boundary). A lone code payload (source "…prog…") is a
/// single string with no string sibling, so it is never flagged — only wrapped prose has adjacent strings.
///
/// Run: `npm run check:sexpr-concat` (node ≥ 22.6 for .ts type-strip). Gate: non-zero exit on any suspect join.
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const { parseSexpr, isAtom, isList, unquoteAtom } = await import(join(guideRoot, "src/notebook/sexpr.ts"));

const chaptersDir = join(guideRoot, "src/content/chapters");
const sexps = readdirSync(chaptersDir).filter((f) => f.endsWith(".sexp")).sort();

const isStringAtom = (n) => isAtom(n) && n.atom.startsWith('"');
const findings = [];

// Embedded CODE holders (seq-213/214): their nested s-expr forms may contain adjacent string atoms that are a
// real code concat expression (e.g. Cadenza `("ye" + "s")`), NOT prose wrapping — the boundary is load-bearing.
// This lint is prose-only, so skip these subtrees entirely (before embedding they were opaque single strings).
const CODE_HOLDERS = new Set(["source", "starter", "solution"]);

// Walk every list; for each pair of CONSECUTIVE string-atom children, check the wrap-boundary join.
function walk(node, file) {
  if (!isList(node)) return;
  const kids = node.list;
  if (kids.length && isAtom(kids[0]) && CODE_HOLDERS.has(kids[0].atom)) return; // embedded code, not prose
  for (let i = 0; i + 1 < kids.length; i++) {
    if (isStringAtom(kids[i]) && isStringAtom(kids[i + 1])) {
      const a = unquoteAtom(kids[i].atom);
      const b = unquoteAtom(kids[i + 1].atom);
      if (a.length && b.length && /[A-Za-z0-9]/.test(a[a.length - 1]) && /[A-Za-z]/.test(b[0])) {
        findings.push({ file, a: a.slice(-24), b: b.slice(0, 24) });
      }
    }
  }
  for (const k of kids) walk(k, file);
}

let parsed = 0;
for (const f of sexps) {
  const text = readFileSync(join(chaptersDir, f), "utf8");
  let root;
  try { root = parseSexpr(text); } catch (e) { console.error(`✗ ${f}: unparseable — ${String(e.message || e).slice(0, 80)}`); process.exit(1); }
  walk(root, f);
  parsed++;
}

if (sexps.length === 0) { console.error("✗ check:sexpr-concat: no .sexp chapters found — broken glob, not a pass."); process.exit(1); }
if (findings.length) {
  console.error(`\n✗ check:sexpr-concat: ${findings.length} suspect string join(s) — a dropped space silently fuses two words (the DOM-fidelity gate can't catch this). Add the missing space, or merge into one string atom if intentional:`);
  for (const f of findings) console.error(`  ${f.file}: …"${f.a}" + "${f.b}"… → joins as "${(f.a + f.b).replace(/.*(.{12})$/, "$1")}${f.b.slice(0, 12)}"`);
  process.exit(1);
}
console.log(`✓ check:sexpr-concat: no dropped-space string joins in prose across ${parsed} .sexp chapter(s).`);

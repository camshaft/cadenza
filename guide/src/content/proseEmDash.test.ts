/// No-prose-em-dash invariant (operator directive: the guide's tone overhaul rewrote every PROSE em-dash
/// into a flowing subordinated sentence — "X — Y" became "X, since Y" / "X, so Y" / "X, which Y" — and
/// the guide now sits at ZERO prose em-dashes across all chapters). That state was reached by hand and,
/// until now, guarded only by hand: a peer edit once silently reintroduced two prose em-dashes into
/// PropertyTesting (the compound-@test rewrite) and it took a manual re-audit to catch. This pins the
/// invariant so a regression fails a test instead of slipping to trunk.
///
/// SCOPE — prose only. Em-dashes are LEGITIMATE inside code and are deliberately left alone:
///   - inline `<C>…</C>` code spans (e.g. OpaqueTypes' `// … — the handle only` in-source comments),
///   - template literals (`source=`/`starter=`/`solution=` Cadenza code, `<Note>` code, trap-string
///     messages like `"5 is not under 5 — this test is meant to fail"`),
///   - JSX block comments `{/* … */}`,
///   - full-line `//`/`///` source comments (the module doc-comment header follows the same
///     `/// … — …` convention every page-shell uses; that em-dash is source commentary, not prose).
/// So we strip those regions first, then scan what remains (the human-readable prose) for U+2014.
/// EN-dashes (U+2013, e.g. numeric ranges "0–255", "0–10") are correct typography and NOT flagged;
/// only the em-dash (—, U+2014) is the tone-overhaul target. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");

const EM_DASH = "—"; // — ; the prose-tone target. NOT the en-dash – (numeric ranges are fine).

/// Remove the regions where an em-dash is legitimate (code), leaving human-readable prose. Order matters:
/// strip full-line `//`/`///` comments FIRST (a chapter's module doc-comment header follows the same
/// `/// … — …` convention every page-shell uses; that em-dash is NOT reader-facing prose, and stripping
/// the comment line before the template-literal pass also avoids a stray backtick inside a comment
/// opening a spurious template strip). Then strip JSX block comments, then inline `<C>…</C>` spans (which
/// can themselves contain stray backticks, e.g. Metaprogramming's quasiquote `` `{ ,x } ``), then
/// template literals. The guide uses no `${}` interpolation in chapter template literals (verified), so a
/// non-greedy backtick pair is a safe strip. A full-line comment is matched only when `//` begins the line
/// (after optional whitespace), so a `//` inside a string/URL/JSX prose line is left untouched.
function proseOnly(src: string): string {
  return src
    .replace(/^[ \t]*\/\/.*$/gm, "") // full-line // or /// comments (doc-comment headers, standalone notes)
    .replace(/\{\/\*[\s\S]*?\*\/\}/g, "") // JSX block comments
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, "")) // JSDoc/block comments, newlines kept for line#s
    .replace(/<C>[\s\S]*?<\/C>/g, "") // inline code spans
    .replace(/<Cadenza>[\s\S]*?<\/Cadenza>/g, "") // surface-aware inline Cadenza spans ((cdz …), #7103) — code, not prose
    .replace(/`[\s\S]*?`/g, ""); // template literals (Runnable/Exercise source, Note code, trap strings)
}

function chapterFiles(): string[] {
  return readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx"));
}

/// The site-SHELL components that render user-facing PROSE outside the chapter dir: the front-door
/// HomePage (the first thing every reader sees), and the Exercise result banner. Their JSX prose belongs
/// to the same zero-em-dash tone as the chapters, but they live under src/components and so are never
/// walked by the chapter scan above — an unguarded gap that let front-door em-dashes ship (HomePage's
/// hero, tenet cards, footer; the Exercise "Correct — …" banner). Paths are relative to this dir
/// (src/content). Extend this list as new prose-bearing shells appear; a pure-logic component with no
/// rendered prose needs no entry (an em-dash in its code is stripped by proseOnly anyway).
const SHELL_PROSE_FILES = ["../components/HomePage.tsx", "../components/Exercise.tsx"];

test("no site-shell component has an em-dash in rendered PROSE (front door holds the tone too)", () => {
  const violations: string[] = [];
  for (const rel of SHELL_PROSE_FILES) {
    const prose = proseOnly(readFileSync(join(here, rel), "utf8")).split("\n");
    for (let i = 0; i < prose.length; i++) {
      if (prose[i].includes(EM_DASH)) {
        violations.push(`${rel}:${i + 1} — prose em-dash: …${prose[i].trim().slice(0, 80)}…`);
      }
    }
  }
  assert.equal(
    violations.length,
    0,
    `prose em-dash(es) in a site-shell component — rewrite as a flowing subordinated clause ` +
      `(", since …" / ", so …" / ": …"). Em-dashes inside <C>…</C>, template literals, and comments are ` +
      `fine; this flags rendered prose:\n  ${violations.join("\n  ")}`,
  );
});

test("the shell-prose scan reads the shell files (guards a vacuous pass)", () => {
  // A moved/renamed shell file would make the invariant pass on nothing. Assert each listed file exists
  // and carries recognizable rendered prose (so a silent read-failure trips here instead of hiding).
  for (const rel of SHELL_PROSE_FILES) {
    const src = readFileSync(join(here, rel), "utf8");
    assert.ok(src.length > 200, `shell prose file ${rel} looks empty/missing`);
  }
  const home = readFileSync(join(here, "../components/HomePage.tsx"), "utf8");
  assert.ok(home.includes("runs in your browser"), "expected HomePage's hero copy; the path may have drifted");
});

test("no chapter has an em-dash in PROSE (tone overhaul: subordinate with since/so/which instead)", () => {
  const violations: string[] = [];
  for (const file of chapterFiles()) {
    const prose = proseOnly(readFileSync(join(chaptersDir, file), "utf8"));
    const lines = prose.split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes(EM_DASH)) {
        violations.push(`${file}:${i + 1} — prose em-dash: …${lines[i].trim().slice(0, 80)}…`);
      }
    }
  }
  assert.equal(
    violations.length,
    0,
    `prose em-dash(es) found — rewrite as a flowing subordinated clause (", since …" / ", so …" / ` +
      `", which …"). Em-dashes inside <C>…</C>, code template literals, and trap strings are fine; ` +
      `this only flags human-readable prose:\n  ${violations.join("\n  ")}`,
  );
});

// The chapter blurbs (`blurb: "…"` in chapters.ts) are user-facing prose too — they're the one-line
// descriptions shown in the sidebar and on the chapter cards, so they belong to the same zero-em-dash tone
// as the chapter bodies. The scan above only reads the chapter .tsx files, so the blurbs were an unguarded
// gap: the tone overhaul that cleared the chapters left several blurbs with em-dashes, and nothing here
// would notice. Pin them too — they're plain string literals in the registry (no `${}` interpolation), so
// each `blurb: "…"` value is scanned directly.
function registryBlurbs(): { blurb: string; line: number }[] {
  const src = readFileSync(join(here, "chapters.ts"), "utf8").split("\n");
  const out: { blurb: string; line: number }[] = [];
  for (let i = 0; i < src.length; i++) {
    const m = src[i].match(/blurb:\s*"((?:[^"\\]|\\.)*)"/);
    if (m) out.push({ blurb: m[1], line: i + 1 });
  }
  return out;
}

test("no chapter blurb has an em-dash (sidebar/card prose holds the same zero-em-dash tone)", () => {
  const violations: string[] = [];
  for (const { blurb, line } of registryBlurbs()) {
    if (blurb.includes(EM_DASH)) violations.push(`chapters.ts:${line} — blurb em-dash: …${blurb.slice(0, 80)}…`);
  }
  assert.equal(
    violations.length,
    0,
    `blurb em-dash(es) found — rewrite the sidebar/card one-liner as a flowing clause (", so …" / ` +
      `", which …" / ": …"):\n  ${violations.join("\n  ")}`,
  );
});

test("the blurb scan actually reads blurbs (guards a vacuous pass)", () => {
  // A broken regex or moved file would make the blurb invariant pass on nothing. Assert we see one blurb
  // per chapter (every registry entry has a blurb, so the counts match) and that a known blurb is captured.
  const blurbs = registryBlurbs();
  assert.ok(blurbs.length >= 30, `expected a blurb per chapter (30+), found ${blurbs.length}`);
  assert.ok(
    blurbs.some((b) => b.blurb.includes("interactive guide works")),
    "expected to capture the Welcome blurb; the scan may be broken",
  );
});

test("the em-dash prose scan reads chapters + strips code (guards a vacuous pass)", () => {
  // A broken strip or empty dir would make the invariant pass on nothing. Assert the machinery works.
  const files = chapterFiles();
  assert.ok(files.length >= 30, `expected many chapter files, got ${files.length}`);

  // A prose em-dash IS caught…
  assert.ok(
    proseOnly("<P>a prose clause " + EM_DASH + " and more</P>").includes(EM_DASH),
    "a prose em-dash must survive the strip (else the gate is vacuous)",
  );
  // …while em-dashes inside a <C> span, a template literal, and a JSX comment are stripped away.
  assert.ok(
    !proseOnly("<C>a " + EM_DASH + " b</C>").includes(EM_DASH),
    "an em-dash inside <C>…</C> must be stripped (legitimate in-code)",
  );
  assert.ok(
    !proseOnly("source={`(trap \"x " + EM_DASH + " y\")`}").includes(EM_DASH),
    "an em-dash inside a template literal (trap string / code) must be stripped",
  );
  // …and an em-dash in a full-line `//`/`///` comment (a chapter doc-comment header) is stripped,
  // while a `//` mid-line (e.g. a protocol-relative URL in prose) does NOT eat the rest of the line.
  assert.ok(
    !proseOnly("/// The chapter " + EM_DASH + " an overview").includes(EM_DASH),
    "an em-dash in a full-line /// comment must be stripped (doc-comment header, not prose)",
  );
  assert.ok(
    proseOnly("<P>see //example.com " + EM_DASH + " the ref</P>").includes(EM_DASH),
    "a mid-line // (URL in prose) must NOT strip the rest of the line",
  );
  // Stripping a comment line preserves line numbering (blanked in place, newline kept) so violation
  // line numbers stay accurate.
  assert.equal(
    proseOnly("/// header\n<P>body</P>").split("\n").length,
    2,
    "comment-line strip must preserve line count for accurate violation line numbers",
  );
  // A multi-line /* … */ block comment is blanked but keeps its newlines, so a violation AFTER it still
  // reports the right line number, and an em-dash INSIDE it is stripped.
  const jsdoc = proseOnly("/** doc " + EM_DASH + " note\n  more */\n<P>body</P>");
  assert.equal(jsdoc.split("\n").length, 3, "block-comment strip must preserve line count");
  assert.ok(!jsdoc.includes(EM_DASH), "an em-dash inside a /* … */ block comment must be stripped");
});

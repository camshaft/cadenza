/// No boolean-value swallowing — a Bool must render AS a Bool, not a coded 1/0. The operator's tone
/// directive is "show the real value, don't make the reader trust a coded proxy". A Cadenza example that
/// wraps a Bool-valued expression in `(if <cond> 1 0)` renders the number 1 or 0 where the reader should
/// see `true`/`false` — the exact swallow the guide swept out of Metaprogramming, Data, Strings, Ordering,
/// MapsSets, ControlFlow, PropertyTesting, Rationals, Symbols, Bytes, Units, ConstParameters, and
/// TypesAsValues. This pins the swept state so a future example can't quietly reintroduce a proxy.
///
/// PRECISE detection (not a substring grep — that false-positives): a proxy is an `(if …)` whose TOP-LEVEL
/// s-expression children are EXACTLY `[if, <cond>, "1", "0"]` — a two-branch if whose then/else are the
/// literal atoms 1 and 0. This structurally EXCLUDES:
///   - three-way numeric classifiers like `(if (< a b) -1 (if (= a b) 0 1))` (a spaceship) — the else is a
///     nested `if`, so there are not exactly four children;
///   - genuine numeric selectors like `(if (Type.eq (Type.of 5) Int64) 100 200)` and symbols' medal score
///     `(if (= m #"gold") 3 …)` — the branches are numbers other than 1/0, and the value IS the output;
///   - the reversed `(if <cond> 0 1)` form (not the swallow shape we swept; none exist, but excluded anyway).
///
/// ONE deliberate allowlist entry: AdHocPolymorphism's `(def (describe-bool b) (if b 1 0))`. That is a real
/// `(if b 1 0)`, but NOT a swallowed Bool — `describe-bool` is a Bool→Int CONVERTER, one half of a trait
/// whose field type is `T -> Int64` (its sibling `describe-int` returns an Int, and `main` sums the two).
/// The 1/0 is the trait's required Int output, not a Bool shown as a number; rewording it to bare `b` would
/// return a Bool and break the surrounding `(+ …)`. It is exempt by the exact enclosing def, not by file, so
/// the chapter's OTHER describe-bool variants (`(if b 999 0)`) and any new proxy elsewhere still fail.
/// Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");
/// The playground's Examples-dropdown programs live here (a sibling package dir). They're the same
/// "every example is a test" surface as the chapters, so the no-Bool-swallowing invariant must hold for
/// them too — the palindrome example was swept from `(if … 1 0)` to a bare Bool alongside the chapter sweep.
const playgroundExamples = join(here, "..", "playground", "examples.ts");

/// From index `i` (at a `(`), return the index just past the matching `)`, respecting nesting.
function endOfSexpr(s: string, i: number): number {
  let depth = 0;
  for (let j = i; j < s.length; j++) {
    if (s[j] === "(") depth++;
    else if (s[j] === ")") {
      depth--;
      if (depth === 0) return j + 1;
    }
  }
  return s.length;
}

/// Split the top-level children of an s-expr BODY (text between the outer parens), respecting nested parens
/// and double-quoted strings (so a `(` or space inside a string literal doesn't split a child).
function topLevelChildren(body: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  let inStr = false;
  for (const c of body) {
    if (inStr) {
      cur += c;
      if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') {
      inStr = true;
      cur += c;
      continue;
    }
    if (c === "(") {
      depth++;
      cur += c;
      continue;
    }
    if (c === ")") {
      depth--;
      cur += c;
      continue;
    }
    if (/\s/.test(c) && depth === 0) {
      if (cur.trim()) out.push(cur.trim());
      cur = "";
      continue;
    }
    cur += c;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

/// True iff `ifText` (a full `(if …)` s-expr) is a boolean-swallowing proxy: exactly `[if, cond, "1", "0"]`.
function isBoolProxy(ifText: string): boolean {
  const kids = topLevelChildren(ifText.slice(1, -1));
  return kids.length === 4 && kids[0] === "if" && kids[2] === "1" && kids[3] === "0";
}

/// The one exempt proxy: the `describe-bool` Bool→Int converter in AdHocPolymorphism. Matched by the enclosing
/// `(def (describe-bool b) (if b 1 0))` text so ONLY that specific converter is waived, not the whole file.
const ALLOWLIST: { file: string; enclosing: string }[] = [
  { file: "AdHocPolymorphism.tsx", enclosing: "(def (describe-bool b) (if b 1 0))" },
];

function isAllowlisted(file: string, ifText: string, src: string): boolean {
  const normIf = ifText.replace(/\s+/g, " ");
  return ALLOWLIST.some((a) => {
    if (a.file !== file) return false;
    if (a.enclosing.replace(/\s+/g, " ") !== `(def (describe-bool b) ${normIf})`) return false;
    return src.replace(/\s+/g, " ").includes(a.enclosing.replace(/\s+/g, " "));
  });
}

/// Every boolean-proxy `(if … 1 0)` across all chapter TSX files, with file:line, minus the allowlist.
function boolProxies(): string[] {
  const found: string[] = [];
  for (const file of readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx"))) {
    const src = readFileSync(join(chaptersDir, file), "utf8");
    for (let i = src.indexOf("(if"); i >= 0; i = src.indexOf("(if", i + 3)) {
      const ifText = src.slice(i, endOfSexpr(src, i));
      if (!isBoolProxy(ifText)) continue;
      if (isAllowlisted(file, ifText, src)) continue;
      const line = src.slice(0, i).split("\n").length;
      found.push(`${file}:${line} — ${ifText.replace(/\s+/g, " ").slice(0, 70)}`);
    }
  }
  return found;
}

/// Every boolean-proxy `(if … 1 0)` in the playground's examples.ts, with line. Read as TEXT (like the
/// chapter scan) — the file is a single TS module of s-expr source strings, so the same detector applies to
/// the whole file without importing it (import needs Node ≥ 22 type-stripping; this stays runner-agnostic).
function playgroundBoolProxies(): string[] {
  const found: string[] = [];
  const src = readFileSync(playgroundExamples, "utf8");
  for (let i = src.indexOf("(if"); i >= 0; i = src.indexOf("(if", i + 3)) {
    const ifText = src.slice(i, endOfSexpr(src, i));
    if (!isBoolProxy(ifText)) continue;
    const line = src.slice(0, i).split("\n").length;
    found.push(`examples.ts:${line} — ${ifText.replace(/\s+/g, " ").slice(0, 70)}`);
  }
  return found;
}

test("no example swallows a Bool as a coded 1/0 (no (if <cond> 1 0) proxy)", () => {
  const proxies = [...boolProxies(), ...playgroundBoolProxies()];
  assert.equal(
    proxies.length,
    0,
    `boolean-swallowing proxy(ies) — a Bool rendered as coded 1/0. Show the value: drop the ` +
      `\`(if <cond> 1 0)\` wrapper so the bare Bool renders true/false (re-pin any exercise's expected to ` +
      `"true"):\n  ${proxies.join("\n  ")}`,
  );
});

/// No SENTINEL INTEGER in an absence arm (operator fleet-wide directive, 2026-08-01): a `None`/`Empty`
/// match arm must not resolve absence to an in-band magic integer. Cadenza is strongly typed — a
/// should-never-happen absence is `(trap "…")`, checked failure is Option/Result; a magic value flowing
/// in-band is the "spreads like wildfire" anti-pattern the operator is killing. We detect only the
/// UNAMBIGUOUS shapes (a negative-int literal `-N`, or the `(- 0 1)` = -1 idiom, as the arm's whole value),
/// because those have NO legitimate reading as a real element. A bare `0`/positive default is NOT flagged —
/// it is context-dependent and sometimes correct (e.g. Rule 110's out-of-row cell truly IS dead=0, and the
/// option-sum-types teaching example's `((None _) 0)` is an intended fallback), a judgment the operator
/// framed as "would idiomatic strongly-typed code do this?" that a regex can't make.
function sentinelAbsenceArms(): string[] {
  const src = readFileSync(playgroundExamples, "utf8");
  const found: string[] = [];
  // `((None …) <val>)` or `((Empty …) <val>)` where <val> is a negative int literal or `(- 0 1)`.
  const re = /\(\((?:None|Empty)\b[^)]*\)\s+(-\d+|\(-\s+0\s+1\))\s*\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const line = src.slice(0, m.index).split("\n").length;
    found.push(`examples.ts:${line} — ${m[0].replace(/\s+/g, " ")}`);
  }
  return found;
}

test("no example resolves an absence arm to a sentinel integer (use trap / Option / Result)", () => {
  const sentinels = sentinelAbsenceArms();
  assert.equal(
    sentinels.length,
    0,
    `sentinel-integer absence arm(s) — a None/Empty case returning a magic int flowing in-band. Model it ` +
      `idiomatically: an invariant-violation absence is \`(trap "…")\`, a checked failure is Option/Result:\n  ${sentinels.join("\n  ")}`,
  );
});

test("the sentinel-arm detector is precise (catches the removed shapes, not legitimate defaults)", () => {
  const probe = (s: string) => {
    const re = /\(\((?:None|Empty)\b[^)]*\)\s+(-\d+|\(-\s+0\s+1\))\s*\)/g;
    return re.test(s);
  };
  // Positive: the exact shapes swept out of n-queens / the index-walk helpers.
  assert.ok(probe("((None) (- 0 1))"), "a (- 0 1) = -1 sentinel arm must be caught");
  assert.ok(probe("((None) -1)"), "a negative-literal sentinel arm must be caught");
  assert.ok(probe("((Empty _) -5)"), "a negative-literal Empty-arm sentinel must be caught");
  // Negatives: legitimate context-dependent defaults are NOT flagged.
  assert.ok(!probe("((None) 0)"), "a bare-0 default (rule-110 dead cell) is context-dependent, not flagged");
  assert.ok(!probe("((None _) 0)"), "the option-sum-types teaching fallback is not flagged");
  assert.ok(!probe('((None) (trap "oob"))'), "the idiomatic trap arm is not flagged");
});

test("the bool-proxy detector is precise (guards false-positives and a vacuous pass)", () => {
  // Positive: a real proxy is caught.
  assert.ok(isBoolProxy("(if (= a b) 1 0)"), "a (if <cond> 1 0) must be detected");
  assert.ok(isBoolProxy("(if b 1 0)"), "a bare-condition proxy must be detected");
  // Negatives: numeric selectors and spaceships are NOT proxies.
  assert.ok(!isBoolProxy("(if (Type.eq (Type.of 5) Int64) 100 200)"), "a 100/200 numeric selector is not a proxy");
  assert.ok(!isBoolProxy('(if (= m #"gold") 3 (if (= m #"silver") 2 1))'), "a 3-way medal score is not a proxy");
  assert.ok(!isBoolProxy("(if (< a b) -1 (if (= a b) 0 1))"), "a -1/0/1 spaceship is not a proxy");
  assert.ok(!isBoolProxy("(if (= a b) 0 1)"), "the reversed 0/1 form is not the swept proxy shape");
  // The scan reads real files (not a vacuous empty pass).
  assert.ok(readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx")).length >= 30, "expected many chapter files");
  // The allowlist entry still names a real, present converter (else it silently waives nothing / drifts).
  const ahp = readFileSync(join(chaptersDir, "AdHocPolymorphism.tsx"), "utf8").replace(/\s+/g, " ");
  assert.ok(ahp.includes("(def (describe-bool b) (if b 1 0))"), "the allowlisted describe-bool converter must exist");
  // The playground scan reads a real file with real `(if …)` forms (not a vacuous pass on a moved/renamed file).
  const pg = readFileSync(playgroundExamples, "utf8");
  assert.ok(/export const EXAMPLES/.test(pg) && pg.includes("(if"), "playground examples.ts must be present and contain (if forms");
});

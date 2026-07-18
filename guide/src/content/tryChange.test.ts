/// Authoring-integrity gate for the guide's clickable "change X to Y → result" prose. A
/// `<TryChange example="id" …>` in a chapter drives the `<Runnable id="id">` it names — applying a
/// variant / one-token patch to that example's buffer and re-running it. Two ways this silently breaks
/// at authoring time, neither caught elsewhere:
///   1. `example=` names an id that no `<Runnable>` on the page declares (typo, or the Runnable's id was
///      renamed/removed) → the click is inert, the reader sees nothing happen.
///   2. a `find=` patch's token does NOT occur exactly once in the target's source (0 = not there, >1 =
///      ambiguous which one) → v-guide-editor's ruling is "fail loud, a silent mis-patch is worse than
///      authoring a full variant".
/// This test pins both, per chapter (a chapter is one page — the registry only ever holds the mounted
/// chapter's Runnables, so ids resolve within a file). Modeled on `links.test.ts`: derive the valid set
/// from source, scan every reference, diff, plus guard tests so a broken regex can't pass vacuously.
/// Shares `patchOnce` with the runtime (`useCadenzaEditor.applyPatch`) so the gate can't drift from the
/// click. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { patchOnce } from "../components/tryChangePatch.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");
const componentsDir = join(here, "..", "components");

function tsxFilesIn(dir: string): string[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".tsx"))
    .map((f) => join(dir, f));
}

/// Grab an attribute off a JSX open-tag slice — either the template-literal form `name={`…`}` or the
/// plain-string form `name="…"`. Mirrors check-examples.mjs's `grab`, kept local (the .mjs isn't a module
/// we can import from a .ts test). Returns null when the attribute is absent.
function grabAttr(attrs: string, name: string): string | null {
  const tl = new RegExp("\\b" + name + "=\\{`([\\s\\S]*?)`\\}").exec(attrs);
  if (tl) return tl[1];
  const s = new RegExp("\\b" + name + '="([^"]*)"').exec(attrs);
  return s ? s[1] : null;
}

interface RunnableDecl { id: string; source: string; line: number }
interface TryChangeDecl { example: string; find: string | null; variant: string | null; line: number }

/// Chunk a file between successive `<Runnable`/`<TryChange` open tags (the same chunk-to-next-open-tag
/// trick check-examples.mjs uses so a `/>` inside JSX children doesn't truncate the attributes), and pull
/// the attributes each cares about. Line numbers make a failure point at the offending tag.
function scan(src: string): { runnables: RunnableDecl[]; tries: TryChangeDecl[] } {
  const runnables: RunnableDecl[] = [];
  const tries: TryChangeDecl[] = [];
  const openRe = /<(Runnable|TryChange)\b/g;
  const opens: { kind: string; start: number }[] = [];
  for (let m = openRe.exec(src); m; m = openRe.exec(src)) opens.push({ kind: m[1], start: m.index });
  const lineOf = (idx: number) => src.slice(0, idx).split("\n").length;
  for (let i = 0; i < opens.length; i++) {
    const { kind, start } = opens[i];
    const end = i + 1 < opens.length ? opens[i + 1].start : src.length;
    const attrs = src.slice(start, end);
    const line = lineOf(start);
    if (kind === "Runnable") {
      const id = grabAttr(attrs, "id");
      if (id) runnables.push({ id, source: grabAttr(attrs, "source") ?? "", line });
    } else {
      const example = grabAttr(attrs, "example");
      if (example) tries.push({ example, find: grabAttr(attrs, "find"), variant: grabAttr(attrs, "variant"), line });
    }
  }
  return { runnables, tries };
}

const rel = (file: string) => file.replace(here, "src/content");

test("every <TryChange example=\"id\"> resolves to a <Runnable id=\"id\"> in the same chapter", () => {
  const dead: string[] = [];
  for (const file of tsxFilesIn(chaptersDir)) {
    const { runnables, tries } = scan(readFileSync(file, "utf8"));
    const ids = new Set(runnables.map((r) => r.id));
    for (const t of tries) {
      if (!ids.has(t.example)) dead.push(`${rel(file)}:${t.line} → example="${t.example}" (no <Runnable id> on this page)`);
    }
  }
  assert.equal(dead.length, 0, `TryChange targeting a missing Runnable id:\n  ${dead.join("\n  ")}`);
});

test("Runnable ids are unique within a chapter (an ambiguous target can't resolve)", () => {
  const dups: string[] = [];
  for (const file of tsxFilesIn(chaptersDir)) {
    const { runnables } = scan(readFileSync(file, "utf8"));
    const seen = new Map<string, number>();
    for (const r of runnables) {
      if (seen.has(r.id)) dups.push(`${rel(file)}:${r.line} → duplicate id="${r.id}" (also at line ${seen.get(r.id)})`);
      else seen.set(r.id, r.line);
    }
  }
  assert.equal(dups.length, 0, `duplicate Runnable id(s) on a page:\n  ${dups.join("\n  ")}`);
});

test("every <TryChange find=…> token occurs EXACTLY ONCE in its target's source (fail-loud, no silent mis-patch)", () => {
  // Best-effort authoring check against the AUTHORED source (the runtime patches the pretty-printed buffer
  // in the reader's surface, so this can't be exhaustive — the surface-stable single tokens find= targets
  // are preserved by the printer, and the runtime backstop declines a non-single match). Catches the common
  // typo / repeated-token authoring mistake before ship. A `variant=` TryChange skips this (no find token).
  const bad: string[] = [];
  for (const file of tsxFilesIn(chaptersDir)) {
    const { runnables, tries } = scan(readFileSync(file, "utf8"));
    const srcById = new Map(runnables.map((r) => [r.id, r.source]));
    for (const t of tries) {
      if (t.find === null) continue; // variant path
      const target = srcById.get(t.example);
      if (target === undefined) continue; // dead-id: reported by the resolution test above
      const res = patchOnce(target, t.find, "");
      if (!res.ok) bad.push(`${rel(file)}:${t.line} → find="${t.find}" ${res.reason} (${res.count}×) in example="${t.example}"`);
    }
  }
  assert.equal(bad.length, 0, `TryChange find= token not matching exactly once:\n  ${bad.join("\n  ")}`);
});

test("every <TryChange> has exactly one of {find+replace, variant} (a well-formed directive)", () => {
  const bad: string[] = [];
  for (const file of tsxFilesIn(chaptersDir)) {
    const { tries } = scan(readFileSync(file, "utf8"));
    // Re-scan raw for replace= presence (scan() only kept find/variant); a find without replace is malformed.
    const src = readFileSync(file, "utf8");
    for (const t of tries) {
      const hasVariant = t.variant !== null;
      const hasFind = t.find !== null;
      if (hasVariant === hasFind) {
        bad.push(`${rel(file)}:${t.line} → example="${t.example}" must have EITHER find+replace OR variant, not ${hasVariant ? "both" : "neither"}`);
      }
      if (hasFind) {
        const slice = src.slice(src.indexOf(`example="${t.example}"`));
        if (!/\breplace=/.test(slice.slice(0, slice.indexOf(">")))) {
          bad.push(`${rel(file)}:${t.line} → example="${t.example}" has find= but no replace=`);
        }
      }
    }
  }
  assert.equal(bad.length, 0, `malformed TryChange directive(s):\n  ${bad.join("\n  ")}`);
});

test("the scan finds Runnable ids + TryChanges when present (guards a broken regex passing vacuously)", () => {
  // Unlike links (always present), TryChange is a NEW feature — the guide may have zero at first, which is
  // legitimately fine. So this guard only asserts the SCANNER works on a known fixture, not a live count:
  // a synthetic page must round-trip through scan() intact, so a future regex break trips here.
  const fixture = `
    <Runnable id="demo" source={\`(+ 2 3)\`} />
    <TryChange example="demo" find="2" replace="4">bump it</TryChange>
    <TryChange example="demo" variant={\`(+ 10 20)\`}>the big one</TryChange>
  `;
  const { runnables, tries } = scan(fixture);
  assert.equal(runnables.length, 1, "scanner should find the one id'd Runnable");
  assert.equal(runnables[0].id, "demo");
  assert.equal(runnables[0].source, "(+ 2 3)");
  assert.equal(tries.length, 2, "scanner should find both TryChanges");
  assert.equal(tries[0].find, "2");
  assert.equal(tries[1].variant, "(+ 10 20)");
});

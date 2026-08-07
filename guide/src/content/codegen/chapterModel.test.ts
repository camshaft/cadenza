/// Pins the guide sexp→TSX codegen core (cadenza-docs I4): parsing each schema head into the model, the
/// two link kinds, the (br) line-break gap found in the PlatformOverview audit, the deferred-to-I5 rejects,
/// and DETERMINISM (same model → byte-identical TSX). A bug here would emit a chapter that doesn't match the
/// hand-written TSX (breaking the render-identically gate) or extract wrong for the editorial gates.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseChapter, renderChapter, type ChapterModel } from "./chapterModel.ts";

function parseOk(src: string): ChapterModel {
  const r = parseChapter(src);
  assert.ok(r.ok, r.ok ? "" : r.reason);
  if (!r.ok) throw new Error(r.reason);
  return r.model;
}

test("parses the chapter metadata (slug/title/pillar/section/blurb)", () => {
  const m = parseOk(`(chapter
    (slug "platform-overview")
    (title "Cadenza the Platform")
    (pillar "platform")
    (section "The kernel model")
    (blurb "The agent kernel."))`);
  assert.equal(m.slug, "platform-overview");
  assert.equal(m.title, "Cadenza the Platform");
  assert.equal(m.pillar, "platform");
  assert.equal(m.section, "The kernel model");
  assert.equal(m.blurb, "The agent kernel.");
  assert.equal(m.blocks.length, 0);
});

test("pillar defaults to language when omitted; rejects an invalid pillar", () => {
  const m = parseOk(`(chapter (slug "x") (title "X"))`);
  assert.equal(m.pillar, "language");
  const bad = parseChapter(`(chapter (slug "x") (title "X") (pillar "middleware"))`);
  assert.equal(bad.ok, false);
});

test("parses lede + block heads (h2/p/note) in order", () => {
  const m = parseOk(`(chapter (slug "x") (title "X")
    (lede "the intro")
    (h2 "A heading")
    (p "a paragraph")
    (note "a note"))`);
  assert.deepEqual(m.lede, [{ kind: "text", text: "the intro" }]);
  assert.deepEqual(m.blocks.map((b) => b.kind), ["h2", "p", "note"]);
});

test("parses inline em / c / br", () => {
  const m = parseOk(`(chapter (slug "x") (title "X")
    (p "run " (c "map") " over " (em "each") " item"))`);
  const p = m.blocks[0];
  assert.deepEqual(p.children, [
    { kind: "text", text: "run " },
    { kind: "code", text: "map" },
    { kind: "text", text: " over " },
    { kind: "em", children: [{ kind: "text", text: "each" }] },
    { kind: "text", text: " item" },
  ]);
});

test("the (br) gap: a note with hard breaks + leading-space indents round-trips", () => {
  const m = parseOk(`(chapter (slug "x") (title "X")
    (note "on an event arriving in session S:" (br) "  run S's reducer" (br) "  append the result"))`);
  const note = m.blocks[0];
  assert.deepEqual(note.children, [
    { kind: "text", text: "on an event arriving in session S:" },
    { kind: "br" },
    { kind: "text", text: "  run S's reducer" }, // leading 2 spaces preserved
    { kind: "br" },
    { kind: "text", text: "  append the result" },
  ]);
  // and the rendered TSX preserves the indent + emits <br />
  const tsx = renderChapter(m);
  assert.match(tsx, /<br \/>/);
  assert.match(tsx, /  run S's reducer/); // literal leading spaces survive into the TSX text
});

test("parses the two link kinds distinctly", () => {
  const m = parseOk(`(chapter (slug "x") (title "X")
    (p "see " (link (slug "effects") "effects & handlers") " and " (app-link (route "/explorer") "the explorer")))`);
  const p = m.blocks[0];
  assert.deepEqual(p.children[1], { kind: "link", slug: "effects", children: [{ kind: "text", text: "effects & handlers" }] });
  assert.deepEqual(p.children[3], { kind: "app-link", route: "/explorer", children: [{ kind: "text", text: "the explorer" }] });
});

test("rejects a chapter missing slug or title", () => {
  assert.equal(parseChapter(`(chapter (title "X"))`).ok, false);
  assert.equal(parseChapter(`(chapter (slug "x"))`).ok, false);
});

test("rejects an unknown head, a bare inline token, and a malformed link", () => {
  assert.equal(parseChapter(`(chapter (slug "x") (title "X") (h3 "nope"))`).ok, false);
  assert.equal(parseChapter(`(chapter (slug "x") (title "X") (p bareword))`).ok, false);
  assert.equal(parseChapter(`(chapter (slug "x") (title "X") (p (link "no-slug-attr")))`).ok, false);
});

test("rejects runnable/exercise/why as deferred to I5 (loud, not silently dropped)", () => {
  const r = parseChapter(`(chapter (slug "x") (title "X") (runnable (surface sexpr) (source "1")))`);
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /deferred to I5/);
  assert.equal(parseChapter(`(chapter (slug "x") (title "X") (why (tenet "t") "body"))`).ok, false);
});

test("rejects a top-level form that isn't (chapter …)", () => {
  assert.equal(parseChapter(`(module (slug "x"))`).ok, false);
  assert.equal(parseChapter(`"just a string"`).ok, false);
});

test("render emits a @generated header, a sorted tsc-clean import, and the PascalCase component", () => {
  const m = parseOk(`(chapter (slug "platform-overview") (title "Cadenza the Platform")
    (lede "intro") (h2 "H") (p "para") (note "n"))`);
  const tsx = renderChapter(m);
  assert.match(tsx, /^\/\/ @generated DO NOT EDIT/);
  assert.match(tsx, /import \{ H1, H2, Lede, Note, P \} from "\.\.\/\.\.\/components\/Prose\.tsx";/); // sorted
  assert.match(tsx, /export default function PlatformOverview\(\)/);
  // no react-router import when there are no links (noUnusedLocals would fail otherwise)
  assert.doesNotMatch(tsx, /react-router-dom/);
});

test("render imports Ch/AppLink from the shared ChapterLink module, only the kinds used", () => {
  // an internal (link …) → <Ch>, imports Ch (not a bare react-router Link)
  const withCh = renderChapter(parseOk(`(chapter (slug "x") (title "X") (p (link (slug "y") "y")))`));
  assert.match(withCh, /import \{ Ch \} from "\.\.\/\.\.\/components\/ChapterLink\.tsx";/);
  assert.match(withCh, /<Ch to="\/y">y<\/Ch>/);
  assert.doesNotMatch(withCh, /react-router-dom/);
  assert.doesNotMatch(withCh, /\bAppLink\b/); // app-link not used → not imported

  // an (app-link …) → <AppLink>, imports AppLink
  const withApp = renderChapter(parseOk(`(chapter (slug "x") (title "X") (p (app-link (route "/explorer") "the explorer")))`));
  assert.match(withApp, /import \{ AppLink \} from "\.\.\/\.\.\/components\/ChapterLink\.tsx";/);
  assert.match(withApp, /<AppLink to="\/explorer">the explorer<\/AppLink>/);

  // both kinds → both imported, sorted (AppLink, Ch)
  const both = renderChapter(parseOk(`(chapter (slug "x") (title "X") (p (link (slug "y") "y") " " (app-link (route "/cad") "cad")))`));
  assert.match(both, /import \{ AppLink, Ch \} from "\.\.\/\.\.\/components\/ChapterLink\.tsx";/);
});

test("render imports C when inline code is used (else <C> is undefined — tsc noUnusedLocals/undefined bug)", () => {
  const withCode = renderChapter(parseOk(`(chapter (slug "x") (title "X") (p "run " (c "map")))`));
  assert.match(withCode, /import \{ C, H1, P \} from "\.\.\/\.\.\/components\/Prose\.tsx";/); // C present + sorted
  assert.match(withCode, /<C>map<\/C>/);
  // and NOT imported when no inline code is present
  const noCode = renderChapter(parseOk(`(chapter (slug "x") (title "X") (p "plain"))`));
  assert.doesNotMatch(noCode, /\bC\b,|\bC\b }/);
});

test("determinism: the same model renders byte-identically", () => {
  const src = `(chapter (slug "x") (title "X") (lede "i") (h2 "h") (p "p " (em "e") " " (c "code")) (note "n" (br) "  x"))`;
  const a = renderChapter(parseOk(src));
  const b = renderChapter(parseOk(src));
  assert.equal(a, b);
});

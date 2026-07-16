/// Unit tests for the shared s-expr reader (tokenize + parse + unquote + strip-ascription). Pins the
/// corners the notebook renderers rely on: quoted strings with spaces/parens, nested lists, the
/// `(: value type)` wrapper strip, and clean errors on malformed input. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseSexpr, stripAscription, unquoteAtom, isList, isAtom, type Node } from "./sexpr.ts";

test("parses atoms, nested lists, and rationals as atoms", () => {
  assert.deepEqual(parseSexpr("42"), { atom: "42" });
  assert.deepEqual(parseSexpr("(list 1 2 3)"), {
    list: [{ atom: "list" }, { atom: "1" }, { atom: "2" }, { atom: "3" }],
  });
  assert.deepEqual(parseSexpr("(tuple 1 (tuple 2 3))"), {
    list: [{ atom: "tuple" }, { atom: "1" }, { list: [{ atom: "tuple" }, { atom: "2" }, { atom: "3" }] }],
  });
  // a rational is a single atom `n/d`
  assert.deepEqual(parseSexpr("127/20"), { atom: "127/20" });
});

test("a quoted string is one atom even with spaces / parens / commas inside", () => {
  const n = parseSexpr('(record (label "a (nested) value, ok"))');
  assert.ok(isList(n));
  const rec = (n as { list: Node[] }).list;
  const field = rec[1] as { list: Node[] };
  assert.deepEqual(field.list[1], { atom: '"a (nested) value, ok"' });
});

test("unquoteAtom strips quotes + resolves escapes; leaves non-strings alone", () => {
  assert.equal(unquoteAtom('"hello"'), "hello");
  assert.equal(unquoteAtom('"a \\"q\\" b"'), 'a "q" b');
  assert.equal(unquoteAtom("42"), "42");
  assert.equal(unquoteAtom("127/20"), "127/20");
});

test("stripAscription unwraps `(: value type)` to the value; passes a bare value through", () => {
  const wrapped = parseSexpr("(: (list 1 2) (List Int64))");
  assert.deepEqual(stripAscription(wrapped), { list: [{ atom: "list" }, { atom: "1" }, { atom: "2" }] });
  const bare = parseSexpr("(list 1 2)");
  assert.deepEqual(stripAscription(bare), bare);
});

test("malformed s-exprs throw a SyntaxError (caller renders a fallback, never crashes)", () => {
  assert.throws(() => parseSexpr(""), SyntaxError);
  assert.throws(() => parseSexpr("(list 1 2"), SyntaxError); // unclosed
  assert.throws(() => parseSexpr("1 2"), SyntaxError); // trailing tokens / multi-root
  assert.throws(() => parseSexpr(")"), SyntaxError);
});

test("deep nesting within the depth cap parses; past the cap throws (PR #475)", () => {
  const ok = 500;
  const n = parseSexpr("(list ".repeat(ok) + "1" + ")".repeat(ok));
  assert.ok(isList(n));
  // Past MAX_DEPTH (1000) the recursive descent throws a clean SyntaxError instead of overflowing.
  const tooDeep = 1200;
  assert.throws(() => parseSexpr("(".repeat(tooDeep) + ")".repeat(tooDeep)), SyntaxError);
});

test("a string ending in an escaped backslash terminates correctly (PR #475 escape run)", () => {
  // `"a\\"` = the two-char content `a\` — the `\\` is an escaped backslash, then the quote CLOSES.
  const n = parseSexpr('(record (path "a\\\\"))');
  assert.ok(isList(n));
  const field = (n as { list: Node[] }).list[1] as { list: Node[] };
  assert.deepEqual(field.list[1], { atom: '"a\\\\"' });
  assert.equal(unquoteAtom('"a\\\\"'), "a\\"); // unquote resolves it to `a\`
});

test("an unterminated string literal throws (not silently accepted) (PR #475)", () => {
  assert.throws(() => parseSexpr('(list "unclosed)'), SyntaxError);
  assert.throws(() => parseSexpr('"no end'), SyntaxError);
  // A string closed by an ESCAPED quote only (never a real close) is also unterminated.
  assert.throws(() => parseSexpr('"escaped quote \\""'.slice(0, -1)), SyntaxError);
});

test("isAtom / isList discriminate", () => {
  assert.equal(isAtom({ atom: "x" }), true);
  assert.equal(isList({ list: [] }), true);
  assert.equal(isAtom({ list: [] }), false);
});

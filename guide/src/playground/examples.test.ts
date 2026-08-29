/// Structural invariants for the playground's Examples dropdown data (src/playground/examples.ts). These
/// are PURE data lints — id/theme/surface shape, floors, the negative-case guarantee — with NO compiler
/// dependency, so they live here under `test:unit` rather than in the compile+run gate. (Historically these
/// were bundled in scripts/check-examples.mjs; that serial harness is being replaced by the cached nix
/// `guide-examples-shredded` matrix, which grades compile+run but NOT this source-data shape. Re-homed here
/// so the invariants survive that harness's retirement — a copy-paste dup id or a typo'd theme still FAILS.)
/// The COMPILE+RUN of every playground example (each must compile and run in both surfaces, honoring
/// expectError / expected) is covered by the shred matrix, not here. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { EXAMPLES } from "./examples.ts";

// The closed sets the `Example` union declares (examples.ts). examples.ts is loaded here with type-stripping
// (no typecheck), so a mistyped `theme`/`surface` would slip past the erased union — pin the sets explicitly.
const THEMES = new Set(["basics", "algorithms", "data-and-collections", "numbers"]);
const SURFACES = new Set(["ml", "sexpr"]);

test("EXAMPLES is a non-empty array above the vacuous-pass floor", () => {
  // If a bad merge/rename empties or guts the array, downstream sweeps would pass on nothing (false green).
  // The dropdown ships ~59; assert a sane minimum with margin for intentional churn (a legit prune below
  // the floor should lower it deliberately, not slip past).
  assert.ok(Array.isArray(EXAMPLES), "EXAMPLES must be an array");
  assert.ok(
    EXAMPLES.length >= 55,
    `expected ≥ 55 playground examples, found ${EXAMPLES.length} — the EXAMPLES array was gutted/renamed`,
  );
});

test("every example has a non-empty, unique id", () => {
  // The UI keys off `id`: deep-links resolve EXAMPLES.find(e => e.id === reqId) (a dup silently loads the
  // FIRST match) and the dropdown renders <option key={id}> (a dup is a React key collision). A dup compiles
  // + runs fine, so only a data lint catches it.
  const counts = new Map<string, number>();
  for (const p of EXAMPLES) {
    assert.ok(
      typeof p.id === "string" && p.id.length > 0,
      `a playground example has a missing/empty id (name="${p.name ?? "?"}")`,
    );
    counts.set(p.id, (counts.get(p.id) ?? 0) + 1);
  }
  const dups = [...counts].filter(([, n]) => n > 1).map(([id]) => id);
  assert.deepEqual(dups, [], `duplicate playground example id(s): ${dups.join(", ")}`);
});

test("at least one example is an intentional expectError case", () => {
  // The "see the squiggle" type-error teaching case is the sole assertion that the playground path still
  // REJECTS bad code; dropping every expectError example would silently remove that coverage.
  assert.ok(
    EXAMPLES.some((p) => p.expectError),
    "no playground example carries expectError: true — the intentional type-error teaching case was dropped",
  );
});

test("every theme is one of the declared sidebar buckets", () => {
  // The sidebar's "Examples" section groups by `theme`; a typo'd/new bucket ships an example into a
  // broken/empty nav group. (Extend both the Example union and this set together.)
  const bad = EXAMPLES.filter((p) => !THEMES.has(p.theme));
  assert.deepEqual(
    bad.map((p) => `${p.id}="${p.theme}"`),
    [],
    `unknown theme(s); allowed: {${[...THEMES].join(", ")}}`,
  );
});

test("every surface is a declared compiler surface", () => {
  // `surface` must be "ml" | "sexpr" (Surface, compiler/client.ts). A typo would compile the example in a
  // bogus surface (confusing downstream failure) rather than fail pointedly here.
  const bad = EXAMPLES.filter((p) => !SURFACES.has(p.surface));
  assert.deepEqual(
    bad.map((p) => `${p.id}="${p.surface}"`),
    [],
    `unknown surface(s); allowed: {${[...SURFACES].join(", ")}}`,
  );
});

test("an expected-value pin is only on a sexpr-authored example", () => {
  // `expected` is compared on the s-expr pass, so an ml-authored pin is only checked against the RENDERED
  // s-expr toggle output (brittle — depends on a byte-stable ML→s-expr render, and reads in a different
  // surface than it's maintained in). Require sexpr-authored pins (all playground examples are sexpr).
  const bad = EXAMPLES.filter((p) => p.expected != null && p.surface !== "sexpr");
  assert.deepEqual(
    bad.map((p) => `${p.id} (surface="${p.surface}")`),
    [],
    "expected pins must be on surface=\"sexpr\" examples",
  );
});

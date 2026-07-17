/// Registry ↔ file correspondence. The chapter registry (chapters.ts) and the chapter TSX files on
/// disk must line up exactly: every registered chapter must import a file that exists (a dangling
/// import breaks the lazy load / the build), and every chapter file on disk must be registered (an
/// UNregistered chapter is written content the reader can never reach — it's in the repo but not in
/// the sidebar, routing, or prev/next, so it silently ships invisible). chapters.test.ts checks slug
/// and exercise-count sync; this checks the file set on both sides. Run with `npm run test:unit`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { CHAPTERS } from "./chapters.ts";

const here = dirname(fileURLToPath(import.meta.url)); // src/content
const chaptersDir = join(here, "chapters");
const registrySrc = readFileSync(join(here, "chapters.ts"), "utf8");

/// The chapter TSX filenames the registry imports, in registry order.
function registeredFiles(): string[] {
  return [...registrySrc.matchAll(/import\("\.\/chapters\/([^"]+)"\)/g)].map((m) => m[1]);
}

/// The chapter TSX filenames present on disk.
function onDiskFiles(): string[] {
  return readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx"));
}

test("every registered chapter imports a file that exists on disk", () => {
  const missing = registeredFiles().filter((f) => !existsSync(join(chaptersDir, f)));
  assert.equal(
    missing.length,
    0,
    `registered chapter file(s) missing on disk (dangling import — breaks the lazy load / build):\n  ${missing.join(
      "\n  ",
    )}`,
  );
});

test("every chapter file on disk is registered (no unreachable, unregistered chapters)", () => {
  const registered = new Set(registeredFiles());
  const orphans = onDiskFiles().filter((f) => !registered.has(f));
  assert.equal(
    orphans.length,
    0,
    `chapter file(s) on disk but NOT in the registry — written content the reader can never reach:\n  ${orphans.join(
      "\n  ",
    )}`,
  );
});

test("the registry imports exactly one file per registered chapter (no double-import / count drift)", () => {
  // The import count should equal the CHAPTERS length — a stray or missing import would desync the
  // file map the other content tests rely on.
  assert.equal(
    registeredFiles().length,
    CHAPTERS.length,
    `registry imports ${registeredFiles().length} chapter files but CHAPTERS has ${CHAPTERS.length} entries`,
  );
});

// Stage the built compiler wasm and the value-heap runtime into the guide's source tree.
//
// Run after `wasm-pack build --target web` in ../implementation/seed/crates/cdz-wasm. Copies:
//   - the wasm-pack `pkg/` (JS glue + cdz_wasm_bg.wasm) -> src/wasm/pkg/
//   - the value-heap runtime component whose SHA-256 == cdz_wasm's `required_runtime_hash()`, found
//     in the cadenza store, -> src/wasm/runtime.wasm  (the guide bundles exactly the runtime the
//     compiler pins, so a compound program's `cadenza:runtime/heap@0.0.0+<hash>` import resolves).
//
// Keeping these in src/ (not public/) lets Vite fingerprint + serve them as hashed assets and lets
// the workers `?url`-import them. The staged files are git-ignored; `npm run wasm` regenerates them.

import { readFile, writeFile, mkdir, cp, readdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guide = join(here, "..");
const crate = join(guide, "..", "implementation", "seed", "crates", "cdz-wasm");
const pkg = join(crate, "pkg");
const dest = join(guide, "src", "wasm");

if (!existsSync(pkg)) {
  console.error(`[stage-wasm] no pkg/ at ${pkg} — run \`wasm-pack build --target web\` first.`);
  process.exit(1);
}

await mkdir(join(dest, "pkg"), { recursive: true });
await cp(pkg, join(dest, "pkg"), { recursive: true });
console.log("[stage-wasm] staged compiler pkg/ -> src/wasm/pkg/");

// Read the runtime hash the compiler pins, straight from the compiler wasm's data section (the
// `REQUIRED_RUNTIME_HASH` string literal is interned there), then find that .wasm in the store. This
// is the ground truth — no hard-coded hash to drift.
const compilerWasm = await readFile(join(pkg, "cdz_wasm_bg.wasm"));
// The data section packs the 64-char hex hash next to other bytes, so a bare /[0-9a-f]{64}/ can
// slide by one and capture a stray leading nibble. Collect EVERY 64-hex candidate (overlapping) and
// pick the one that actually names a .wasm in a store — the store filename is the authoritative hash.
const hay = compilerWasm.toString("latin1");
const candidates = new Set();
const re = /[0-9a-f]{64}/g;
let mm;
while ((mm = re.exec(hay))) {
  candidates.add(mm[0]);
  re.lastIndex = mm.index + 1; // overlapping scan — the real hash may be off by one byte
}
if (candidates.size === 0) {
  console.error("[stage-wasm] could not find any 64-hex runtime hash in the compiler wasm.");
  process.exit(1);
}

// Search likely store locations: an explicit CADENZA_STORE (passed by `cargo xtask guide-wasm`),
// then the worktree store, then the main repo's store.
const stores = [
  process.env.CADENZA_STORE,
  join(guide, "..", "target", "cadenza-store"),
  join(guide, "..", "..", "..", "..", "target", "cadenza-store"),
].filter(Boolean);
let hash = null;
let runtimePath = null;
outer: for (const h of candidates) {
  for (const s of stores) {
    const candidate = join(s, `${h}.wasm`);
    if (existsSync(candidate)) {
      hash = h;
      runtimePath = candidate;
      break outer;
    }
  }
}
hash ??= [...candidates][0];
if (!runtimePath) {
  console.error(
    `[stage-wasm] runtime ${hash}.wasm not found in any store (${stores.join(", ")}).\n` +
      `  Build it with \`cargo xtask build\` so the store holds the compiler's pinned runtime.`,
  );
  // Not fatal for scalar-only development — the guide runs scalar examples without a runtime.
  console.error("[stage-wasm] continuing WITHOUT a bundled runtime (scalar examples only).");
} else {
  await writeFile(join(dest, "runtime.wasm"), await readFile(runtimePath));
  await writeFile(join(dest, "runtime-hash.txt"), hash);
  console.log(`[stage-wasm] staged runtime ${hash.slice(0, 12)}… -> src/wasm/runtime.wasm`);
}

// Stage the CAD library sources into the guide tree so /cad can PRELOAD them via `compile_with_preloaded`
// — the reader's buffer holds only the model, the CAD vocab is link-merged from these preloaded modules
// (operator P5, ruling A). `exact.cdz` is the base geometry lib (Solid/Vec3/v3r/lower/…); `helpers.cdz` is
// the ergonomic surface (box/cyl/hole-through/…) the PARAMETRIC showcase models import; `units.cdz` is the
// UNIT edge constructors (inch/…) the units-parametric showcase uses (a slider read in inches, converted
// exactly over Rational to model mm). They live OUTSIDE guide/src (a raw `../../../implementation/cad/src/
// *.cdz` import is blocked by Vite's dev `server.fs.allow` with project root = guide/), so staging them here
// (git-ignored, regenerated with the wasm — SAME pattern as runtime.wasm) is the single-source, dev-and-
// build-safe way. CadPage `?raw`-imports the staged copies. Non-fatal if absent (only /cad needs them).
const cadLibs = ["exact.cdz", "helpers.cdz", "units.cdz", "snowflake.cdz", "prng.cdz"];
await mkdir(join(dest, "cad"), { recursive: true });
for (const lib of cadLibs) {
  const src = join(guide, "..", "implementation", "cad", "src", lib);
  if (existsSync(src)) {
    await writeFile(join(dest, "cad", lib), await readFile(src));
    console.log(`[stage-wasm] staged CAD lib ${lib} -> src/wasm/cad/${lib}`);
  } else {
    console.error(`[stage-wasm] CAD lib not found at ${src} — /cad preload of '${lib}' will be unavailable (non-fatal).`);
  }
}

// Sanity: report what we staged.
const staged = await readdir(join(dest, "pkg"));
console.log(`[stage-wasm] pkg contents: ${staged.join(", ")}`);

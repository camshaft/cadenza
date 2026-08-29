/// Thin wrapper: regenerate (or `--check`) the chapter registry (chapters.ts CHAPTERS[]) by invoking the Rust
/// xtask `xtask-codegen-guide --registry`. ALL derivation logic lives in the xtask (operator: no codegen in
/// JavaScript — keep it in small self-contained xtask scripts); this only resolves the binary and calls it,
/// exactly like codegen-chapters.mjs does for the .tsx codegen. The xtask reads chapter-order.txt (the
/// editorial reading-order stem list) + each chapter's .sexp and rewrites the `// <generated:chapters>` region.
///
/// MODES: default = regenerate in place; `--check` = verify committed chapters.ts is in sync (CI gate).
/// Run: `node scripts/codegen-registry.mjs [--check]`.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, delimiter } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const repoRoot = join(guideRoot, "..");
const chaptersTs = join(guideRoot, "src/content/chapters.ts");

// Find `name` on $PATH (pure node, no `which` dependency — the nix sandbox may lack one).
function resolveOnPath(name) {
  for (const d of (process.env.PATH || "").split(delimiter)) {
    if (d && existsSync(join(d, name))) return join(d, name);
  }
  return null;
}

// Binary resolution mirrors codegen-chapters.mjs: (1) $CDZ_XTASK_CODEGEN_GUIDE override; (2) on $PATH (v-nix's
// nativeBuildInputs injection — the guide-examples nix gate has no cargo vendor); (3) `cargo build -p` for
// native dev (the `-p` form falls through the all-nix cargo-shim).
let xtaskBin = process.env.CDZ_XTASK_CODEGEN_GUIDE || resolveOnPath("xtask-codegen-guide");
if (!xtaskBin) {
  try {
    execFileSync("cargo", ["build", "-p", "xtask-codegen-guide", "--quiet"], { cwd: repoRoot, stdio: "inherit" });
  } catch (e) {
    console.error(`codegen-registry: could not build xtask-codegen-guide — ${String(e.message || e).slice(0, 160)}`);
    process.exit(1);
  }
  xtaskBin = join(repoRoot, "target/debug/xtask-codegen-guide");
}

const args = ["--registry", ...(process.argv.includes("--check") ? ["--check"] : []), chaptersTs];
try {
  execFileSync(xtaskBin, args, { stdio: "inherit" });
} catch {
  process.exit(1); // the xtask already printed the reason
}

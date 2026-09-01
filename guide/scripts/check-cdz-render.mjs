/// GATE: every inline Cadenza span — the `(cdz …)` tag, emitted as `<Cadenza ast="<base64>" kind="…">` —
/// must RENDER cleanly in BOTH surfaces from its embedded binary-AST. Closes a real coverage gap
/// (v-guide-editor): check:examples / guide-shred gate only runnable/exercise SOURCES, so a mis-rendering
/// INLINE prose span (a fragment that errors in render_binary, or a stale/corrupt embedded AST) was caught
/// by NO gate — only by hand audits. This exercises each embedded AST through the SAME render_binary path
/// the <Cadenza> component ships (#7245), asserting decode+print succeeds for ml AND sexpr. A future span
/// that embeds a non-renderable AST fails HERE instead of silently in-browser. Fast (no jco/run), pure node.
/// Run: `npm run check:cdz-render`. Wired into the guide `check:*` battery.
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const guideRoot = join(here, "..");
const pkgDir = process.env.CDZ_WASM_PKG ?? join(guideRoot, "src/wasm/pkg");
const { default: init, render_binary } = await import(join(pkgDir, "cdz_wasm.js"));
await init({ module_or_path: readFileSync(join(pkgDir, "cdz_wasm_bg.wasm")) });

const chaptersDir = join(guideRoot, "src/content/chapters");
// The codegen emits AST-backed inline Cadenza as `<Cadenza ast="<base64>" kind="<expr|type|pattern>">…`.
// (A non-parseable fragment falls back to the plain `<Cadenza>text</Cadenza>` form with no `ast=` — those
// carry no embedded AST to check, and render via render_syntax on the text; not this gate's concern.)
const SPAN = /<Cadenza ast="([A-Za-z0-9+/=]+)" kind="(\w+)">/g;

const b64ToBytes = (b64) => new Uint8Array(Buffer.from(b64, "base64"));

let spans = 0;
const failures = [];
for (const file of readdirSync(chaptersDir).filter((f) => f.endsWith(".tsx"))) {
  const src = readFileSync(join(chaptersDir, file), "utf8");
  for (const m of src.matchAll(SPAN)) {
    const [, b64, kind] = m;
    spans++;
    const bytes = b64ToBytes(b64);
    // Both surfaces must render without throwing — a decode/print error is exactly the ungated failure mode.
    for (const surface of ["ml", "sexpr"]) {
      try {
        const out = render_binary(bytes, surface, kind);
        if (typeof out !== "string" || out.length === 0) {
          failures.push(`${file}: (cdz …) kind=${kind} rendered empty/${typeof out} in ${surface}`);
        }
      } catch (e) {
        failures.push(`${file}: (cdz …) kind=${kind} FAILED to render in ${surface} — ${String(e?.message ?? e).slice(0, 120)}`);
      }
    }
  }
}

// Guard a vacuous pass: the guide has many migrated (cdz …) spans, so zero means the extractor drifted
// (the emitted shape changed) — a silent green that would let a real render regression through.
if (spans < 50) {
  console.error(
    `check:cdz-render: found only ${spans} AST-backed <Cadenza ast=…> spans across the chapters — ` +
      `expected many more. The codegen's emitted shape likely changed; update the SPAN regex.`,
  );
  process.exit(2);
}

if (failures.length) {
  console.error(`check:cdz-render: ${failures.length} inline (cdz …) span(s) failed to render:\n  ` + failures.join("\n  "));
  process.exit(1);
}
console.log(`check:cdz-render: ${spans} inline (cdz …) spans render cleanly in both surfaces (ml + sexpr) from their embedded AST ✓`);

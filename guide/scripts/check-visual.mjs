// Visual/responsive smoke for the guide's app routes — the check that finally lets the fleet VERIFY
// guide UI instead of reasoning about CSS blind. It loads each route in a real headless Chromium at a
// phone AND a desktop viewport and asserts the two failures that CSS reasoning keeps missing:
//   1. NO horizontal overflow (document scrollWidth <= clientWidth) — the classic mobile break where a
//      long unbroken token (a big-integer calculator result, a long fraction, a wide code line) forces
//      the whole page to scroll sideways. This is exactly the bug the calculator's `break-all` fix
//      targeted; this gate proves it stays fixed.
//   2. NO uncaught console/page errors on load + basic interaction.
//
// It runs the BUILT site (npm run build first) via `vite preview`, so it checks what actually ships.
//
// ENV: Chromium needs the platform libstdc++ (the bundled Playwright browser links the system lib). Run:
//
//   LD_PRELOAD=/usr/lib64/libstdc++.so.6 LD_LIBRARY_PATH=/usr/lib64:/lib64 \
//     mise exec node@22 -- node scripts/check-visual.mjs
//
// (npm run check:visual wires the shim + node version; see package.json.) Playwright's chromium binary
// must be installed (npx playwright install chromium) — the check SKIPS with a clear message if the
// browser or the system lib is absent, so it never falsely fails a machine that simply can't run it
// (e.g. a CI image without the browser). A machine that CAN run it fails loudly on a real regression.

import { preview } from "vite";

const PORT = 4322;
const VIEWPORTS = [
  { name: "mobile-390", width: 390, height: 844 },
  { name: "desktop-1280", width: 1280, height: 900 },
];

// Each route + an optional interaction that exercises the overflow-prone content. `waitFor` is a
// selector that must appear before we measure (the route's shell has mounted).
const ROUTES = [
  { path: "/", waitFor: "a[href]", label: "home" },
  {
    path: "/calculator",
    waitFor: "input",
    label: "calculator",
    // 2^256 is a 78-digit big integer — a single unbroken token, the mobile-overflow case.
    async interact(page) {
      await page.fill("input", "2 ^ 256");
      await page.keyboard.press("Enter");
      await page.waitForTimeout(3500);
    },
  },
  { path: "/playground", waitFor: ".cm-content", label: "playground" },
  {
    // /cad's showcase must RENDER on first load: it auto-runs the starter Solid program on mount, which
    // compiles → runs → meshes → mounts a three.js <canvas>. A regression here (a broken starter, a
    // surface mismatch feeding the driver a non-s-expr render) leaves "Run to preview" / an error and NO
    // canvas — exactly the first-load break this route was added to catch. Desktop-only (the 3D preview
    // isn't a mobile-overflow surface, and the heavy three+manifold chunk is slow to double-run).
    path: "/cad",
    waitFor: "textarea",
    label: "cad (s-expr)",
    surface: "sexpr",
    onlyViewports: ["desktop-1280"],
    expectCanvas: true,
    async interact(page) {
      // Auto-run on mount meshes the starter; wait up to 30s for the canvas (compile+run+lazy 3D chunk).
      await page.waitForSelector("canvas", { timeout: 30000 }).catch(() => {});
    },
  },
  {
    // Same route in the ML surface — /cad respects the global surface toggle and ships a per-surface
    // starter, so BOTH must compile→render→mesh on first load (the ML starter was verified to render to
    // the same canonical Solidr as the s-expr one). Guards the ML editing path from regressing.
    path: "/cad",
    waitFor: "textarea",
    label: "cad (ml)",
    surface: "ml",
    onlyViewports: ["desktop-1280"],
    expectCanvas: true,
    async interact(page) {
      await page.waitForSelector("canvas", { timeout: 30000 }).catch(() => {});
    },
  },
];

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  console.log("check:visual SKIPPED — playwright not installed (npm i -D playwright && npx playwright install chromium).");
  process.exit(0);
}

let browser;
try {
  browser = await chromium.launch();
} catch (e) {
  const msg = e.message.split("\n")[0];
  // A machine that can't launch the browser (missing system lib / no browser binary / sandbox) SKIPS
  // rather than fails — this check is opportunistic, not a hard gate on every environment.
  console.log(`check:visual SKIPPED — chromium could not launch: ${msg}`);
  console.log("  (needs the libstdc++ shim: LD_PRELOAD=/usr/lib64/libstdc++.so.6 LD_LIBRARY_PATH=/usr/lib64:/lib64)");
  process.exit(0);
}

const server = await preview({ root: process.cwd(), preview: { port: PORT, strictPort: true } });
const base = `http://localhost:${PORT}`;

let failures = 0;
const check = (ok, msg) => {
  console.log(`${ok ? "  ✓" : "  ✗"} ${msg}`);
  if (!ok) failures++;
};

try {
  for (const vp of VIEWPORTS) {
    console.log(`\n[${vp.name}]`);
    for (const route of ROUTES) {
      // A route can restrict itself to certain viewports (e.g. /cad's 3D preview is desktop-only).
      if (route.onlyViewports && !route.onlyViewports.includes(vp.name)) continue;
      const page = await browser.newPage({ viewport: { width: vp.width, height: vp.height } });
      const errs = [];
      page.on("console", (m) => {
        if (m.type() === "error") errs.push(m.text());
      });
      page.on("pageerror", (e) => errs.push(String(e)));
      try {
        // A route can pin the guide surface (ML vs s-expr) — seed localStorage on the origin BEFORE the
        // SPA boots, so a surface-dependent route (e.g. /cad's per-surface starter) renders in that surface.
        if (route.surface) {
          await page.goto(`${base}/`, { waitUntil: "domcontentloaded", timeout: 30000 });
          await page.evaluate((s) => localStorage.setItem("cadenza.syntax", s), route.surface);
        }
        await page.goto(`${base}${route.path}`, { waitUntil: "networkidle", timeout: 30000 });
        await page.waitForSelector(route.waitFor, { timeout: 30000 });
        if (route.interact) await route.interact(page);

        const { scrollW, clientW } = await page.evaluate(() => ({
          scrollW: document.documentElement.scrollWidth,
          clientW: document.documentElement.clientWidth,
        }));
        // Allow 1px for sub-pixel rounding.
        check(scrollW <= clientW + 1, `${route.label}: no horizontal overflow (scrollW=${scrollW} clientW=${clientW})`);

        // Ignore benign vendor console noise (Vite's "use components" directive warning surfaces on the
        // heavy jco routes). Only fail on errors that aren't that known, harmless line.
        const realErrs = errs.filter((e) => !/use components|module level directives/i.test(e));
        check(realErrs.length === 0, `${route.label}: no console/page errors${realErrs.length ? ` (${realErrs.slice(0, 2).join(" | ")})` : ""}`);

        // A route that must render 3D (only /cad) has to have mounted a <canvas> after its interaction.
        if (route.expectCanvas) {
          const hasCanvas = await page.evaluate(() => !!document.querySelector("canvas"));
          check(hasCanvas, `${route.label}: 3D preview rendered a <canvas> on first load`);
        }
      } catch (e) {
        check(false, `${route.label}: driver error — ${e.message.split("\n")[0]}`);
      } finally {
        await page.close();
      }
    }
  }
} finally {
  await browser.close();
  await server.close();
}

console.log(failures === 0 ? "\n✓ visual smoke: every route fits its viewport with no console errors (mobile + desktop)." : `\n✗ visual smoke: ${failures} check(s) failed.`);
process.exit(failures === 0 ? 0 : 1);

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

// Each route entry: `path`, `label`, `waitFor` (selector that must appear before we measure), and any of
// the optional hooks — `surface` (seed ML/s-expr before boot), `onlyViewports`, `interact(page)` (exercise
// content before measuring), `expectCanvas` (assert a <canvas> mounted), and `assert(page, check, label)`
// (arbitrary route-specific assertions reported through the shared pass/fail tally, e.g. /notebook: a
// widget drag recomputes a cell's output). Every route always gets the baseline overflow + console checks.
const ROUTES = [
  {
    // Home — guards the HomePage header nav (Guide/Playground) + the live demo Runnable's controls at
    // mobile. EXCEPT the inline "Read the design tenets" prose link (→ /philosophy): it's a link inside a
    // sentence, not a primary control, so the 44px floor doesn't apply (forcing it would break the prose).
    path: "/",
    waitFor: "a[href]",
    label: "home",
    tapTargets: true,
    tapTargetsExcept: 'a[href*="philosophy"]',
  },
  {
    // A chapter page — exercises the shared shell (SyntaxToggle, nav drawer) AND the Runnable/Exercise
    // example controls (Run/Reset/Check/Hint/Show-solution/Open-in-playground), the guide's highest-count
    // controls. /ordering has both a Runnable and an Exercise, so it covers both control sets. Guards the
    // shell + example-control tap sizing (both landed) against regression.
    path: "/ordering",
    waitFor: ".cm-editor",
    label: "chapter (ordering)",
    tapTargets: true,
  },
  {
    path: "/calculator",
    waitFor: "input",
    label: "calculator",
    // Guard the mobile tap-target pass (=button / input / Clear / Playground→ / example chips sized to
    // min-h-11 below sm) against regression.
    tapTargets: true,
    // 2^256 is a 78-digit big integer — a single unbroken token, the mobile-overflow case.
    async interact(page) {
      await page.fill("input", "2 ^ 256");
      await page.keyboard.press("Enter");
      await page.waitForTimeout(3500);
    },
  },
  {
    // Playground — guards the toolbar (Run/Format/Share/Examples/surface-toggle) + output view tabs at
    // mobile. EXCEPT the dense 11px IDE status bar (Ln/Col + "home · the guide" footer credits): an
    // IDE status-line pattern, not primary controls, so exempt from the 44px floor (forcing it breaks the
    // strip). Marked via `data-testid="status-bar"`.
    path: "/playground",
    waitFor: ".cm-content",
    label: "playground",
    tapTargets: true,
    tapTargetsExcept: '[data-testid="status-bar"]',
  },
  {
    // /cad's showcase must RENDER on first load: it auto-runs the starter Solid program on mount, which
    // compiles → runs → meshes → mounts a three.js <canvas>. A regression here (a broken starter, a
    // surface mismatch feeding the driver a non-s-expr render) leaves "Run to preview" / an error and NO
    // canvas — exactly the first-load break this route was added to catch. Desktop-only (the 3D preview
    // isn't a mobile-overflow surface, and the heavy three+manifold chunk is slow to double-run).
    path: "/cad",
    waitFor: "button", // the ▶ Run button is always present; the editor is a lazy CodeMirror (no <textarea>)
    label: "cad (s-expr)",
    surface: "sexpr",
    onlyViewports: ["desktop-1280"],
    expectCanvas: true,
    async interact(page) {
      // Auto-run on mount meshes the starter; wait up to 30s for the canvas (compile+run+lazy 3D chunk).
      await page.waitForSelector("canvas", { timeout: 30000 }).catch(() => {});
    },
    async assert(page, check, label) {
      // The editor must be the shared CodeMirror IDE component (highlighting), not a plain textarea.
      const cm = await page.locator(".cm-editor").count();
      check(cm > 0, `${label}: source editor is the CodeMirror IDE component (Cadenza highlighting)`);
      // Example picker: present with >1 model, and switching to another model re-seeds the editor +
      // re-meshes (the editor buffer changes to the new model's source). Operator UX example-switcher.
      const picker = page.locator('[data-testid="cad-example-picker"]');
      if ((await picker.count()) > 0) {
        const optCount = await picker.locator("option").count();
        check(optCount > 1, `${label}: example picker offers multiple models (found ${optCount})`);
        const before = (await page.locator(".cm-content").first().innerText()).slice(0, 200);
        const secondVal = await picker.locator("option").nth(1).getAttribute("value");
        await picker.selectOption(secondVal);
        const changed = await page
          .waitForFunction(
            (prev) => {
              const cmc = document.querySelector(".cm-content");
              return cmc && cmc.innerText.slice(0, 200) !== prev;
            },
            before,
            { timeout: 20000 },
          )
          .then(() => true)
          .catch(() => false);
        check(changed, `${label}: switching the example picker loads a different model`);
      } else {
        check(false, `${label}: example picker present`);
      }
    },
  },
  {
    // Same route in the ML surface — /cad respects the global surface toggle and ships a per-surface
    // starter, so BOTH must compile→render→mesh on first load (the ML starter was verified to render to
    // the same canonical Solidr as the s-expr one). Guards the ML editing path from regressing.
    path: "/cad",
    waitFor: "button",
    label: "cad (ml)",
    surface: "ml",
    onlyViewports: ["desktop-1280"],
    expectCanvas: true,
    async interact(page) {
      await page.waitForSelector("canvas", { timeout: 30000 }).catch(() => {});
    },
  },
  {
    // /cad on MOBILE — the 3D preview pane must be LARGE (the operator flagged it rendering as a tiny
    // box). On a stacked (flex-col) mobile layout the preview gets a `min-h-[60vh]` floor so it fills
    // most of the section; this guards that it stays a meaningful fraction of the viewport (not the
    // small flex-split box it collapsed to before). Mobile-only; we don't wait for the heavy mesh here
    // (the desktop cases cover canvas rendering) — we measure the PANE, which is sized by CSS regardless.
    path: "/cad",
    waitFor: "button",
    label: "cad (mobile preview size)",
    surface: "sexpr",
    onlyViewports: ["mobile-390"],
    async assert(page, check, label) {
      // The preview pane is the dark (bg-slate-950) panel in the editor/preview split.
      const box = await page.locator("div.bg-slate-950").first().boundingBox().catch(() => null);
      const vh = page.viewportSize()?.height ?? 844;
      const pct = box ? Math.round((box.height / vh) * 100) : 0;
      // 60vh is the floor; allow slack for rounding/borders — anything ≥ 50% of the viewport is "big".
      check(pct >= 50, `${label}: preview pane fills the mobile viewport (${pct}% of ${vh}px, want ≥50%)`);
    },
  },
  {
    // /notebook must (1) run its starter cell to a computed value on first load and (2) reactively
    // recompute when a widget changes — the novel runtime-input mechanism. A regression (a broken cell
    // assembly, a dead reactive edge) leaves an error string / a frozen output. Notebook is surface-pinned
    // s-expr internally, so no `surface` seed needed. Desktop-only (not a mobile-overflow surface).
    path: "/notebook",
    waitFor: '[data-testid="notebook"]',
    label: "notebook",
    onlyViewports: ["desktop-1280"],
    async interact(page) {
      // Wait for the first cell to finish its initial run (leave "not run"/"running").
      await page
        .waitForFunction(
          () => {
            const e = document.querySelector('[data-testid="cell-output"]');
            return e && !/not run|running/i.test(e.innerText);
          },
          { timeout: 30000 },
        )
        .catch(() => {});
    },
    async assert(page, check, label) {
      const out0 = (await page.locator('[data-testid="cell-output"]').first().innerText()).trim();
      check(/\d/.test(out0) && !/error|running/i.test(out0), `${label}: first cell runs to a computed value (${JSON.stringify(out0)})`);
      // Drive the rate slider. A React CONTROLLED range input ignores a raw `el.value = v` (React's value
      // tracker overrides it) — Playwright's fill() uses the native setter path React observes, so the
      // onChange fires and the cell recomputes. (A manual el.value+dispatch does NOT work here.)
      const rate = page.locator('[data-testid="widget-rate"]');
      if ((await rate.count()) > 0) {
        await rate.fill("0.2");
        await page
          .waitForFunction(
            (prev) => {
              const e = document.querySelector('[data-testid="cell-output"]');
              return e && !/running/i.test(e.innerText) && e.innerText.trim() !== prev;
            },
            out0,
            { timeout: 30000 },
          )
          .catch(() => {});
        const out1 = (await page.locator('[data-testid="cell-output"]').first().innerText()).trim();
        check(out1 !== out0, `${label}: a widget change reactively recomputes the cell (${JSON.stringify(out0)} -> ${JSON.stringify(out1)})`);
      } else {
        check(false, `${label}: widget-rate slider present`);
      }
      // Per-cell editing (operator ruling): each code cell is its OWN CodeMirror IDE editor, always
      // visible (no "Edit source" toggle). So a `.cm-editor` must be present in the notebook, and there
      // should be MORE THAN ONE (the starter has multiple code cells, each its own editor) — confirming
      // the stacked per-cell layout rather than a single whole-doc editor.
      const cmCount = await page
        .waitForSelector('[data-testid="notebook"] .cm-editor', { timeout: 20000 })
        .then(() => page.locator('[data-testid="notebook"] .cm-editor').count())
        .catch(() => 0);
      check(cmCount > 1, `${label}: per-cell CodeMirror editors (found ${cmCount}, want >1 — stacked cells)`);
      // Example picker: present with >1 option, and SWITCHING to another example replaces the notebook
      // content (a different example renders different prose/first-cell text). The picker lets a reader
      // pick between the canonical notebooks (operator UX ask #2).
      const picker = page.locator('[data-testid="notebook-example-picker"]');
      if ((await picker.count()) > 0) {
        const optCount = await picker.locator("option").count();
        check(optCount > 1, `${label}: example picker offers multiple notebooks (found ${optCount})`);
        const before = (await page.locator('[data-testid="notebook"]').innerText()).slice(0, 400);
        // Select the 2nd example (index 1) and wait for the doc to re-render to different content.
        const secondVal = await picker.locator("option").nth(1).getAttribute("value");
        await picker.selectOption(secondVal);
        const changed = await page
          .waitForFunction(
            (prev) => {
              const nb = document.querySelector('[data-testid="notebook"]');
              return nb && nb.innerText.slice(0, 400) !== prev;
            },
            before,
            { timeout: 20000 },
          )
          .then(() => true)
          .catch(() => false);
        check(changed, `${label}: switching the example picker loads a different notebook`);
      } else {
        check(false, `${label}: example picker present`);
      }
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
        // `domcontentloaded`, NOT `networkidle`: the heavy routes (playground/cad/notebook) run the compile
        // + run workers and poll version.json, so the network is never idle — `networkidle` flakily times
        // out even though the page is interactive. The `waitForSelector` + each route's `interact` polling
        // below are the real readiness gate, so waiting for the DOM is sufficient and not flaky.
        await page.goto(`${base}${route.path}`, { waitUntil: "domcontentloaded", timeout: 30000 });
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

        // Mobile TAP TARGETS: on the phone viewport, a route that opted into `tapTargets` must have no
        // genuinely-tiny interactive control — the 44px touch guideline (Apple HIG / WCAG). The
        // mobile-responsive pass sizes controls to `min-h-11` below `sm`. We assert primarily on HEIGHT
        // (≥ 40px, 44 minus rounding/border slack) — the vertical tap axis the pass fixed, and the one that
        // matters on a phone where controls stack down the page. A control that is TALL enough but narrow
        // (a short-label segmented-control button like "ML", 33px wide × 44px tall) is a comfortable tap
        // area and passes; we still flag anything genuinely tiny in BOTH axes (width < 24 AND height < 40).
        // Only meaningful at mobile width, so gate on the viewport.
        if (route.tapTargets && vp.name === "mobile-390") {
          // `tapTargetsExcept` (a CSS selector) marks INTENTIONAL small-text exceptions the 44px floor does
          // not apply to — a dense IDE status-bar (Ln/Col + footer credits) or an inline PROSE link inside a
          // sentence, neither a primary control (forcing 44px there breaks the strip / the prose flow). An
          // element matching the selector (or inside a match) is skipped. Documented per route.
          const bad = await page.evaluate((exceptSel) => {
            const excepted = exceptSel ? new Set(document.querySelectorAll(exceptSel)) : new Set();
            const isExcepted = (el) => {
              for (let n = el; n; n = n.parentElement) if (excepted.has(n)) return true;
              return false;
            };
            const els = [...document.querySelectorAll('button, a[href], input, [role="button"], select')];
            let worst = null;
            for (const el of els) {
              const r = el.getBoundingClientRect();
              if (r.width === 0 || r.height === 0) continue; // hidden / not laid out
              if (isExcepted(el)) continue; // an intentional small-text exception (status bar / inline prose)
              // Too small = short height (the primary tap axis), OR genuinely tiny in both axes.
              const tooShort = r.height < 40;
              const tinyBoth = r.width < 24 && r.height < 40;
              if (tooShort || tinyBoth) {
                const h = Math.round(r.height);
                if (!worst || h < worst.h) {
                  worst = { h, w: Math.round(r.width), desc: (el.textContent || el.getAttribute("aria-label") || el.tagName).trim().slice(0, 24) };
                }
              }
            }
            return worst;
          }, route.tapTargetsExcept ?? null);
          check(
            bad == null,
            `${route.label}: mobile tap targets ≥ 44px tall${bad ? ` (found ${bad.w}×${bad.h}px "${bad.desc}")` : ""}`,
          );
        }

        // A route can run arbitrary custom assertions via the shared `check(ok, msg)` helper — for
        // route-specific behavior the flags above don't cover (e.g. /notebook: a widget drag recomputes a
        // cell's output). `assert(page, check, label)` reports through the same pass/fail tally.
        if (route.assert) await route.assert(page, check, route.label);
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

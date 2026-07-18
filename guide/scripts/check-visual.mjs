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
      // DESKTOP LAYOUT (operator: the viewer was squished to a ~400px sliver off to the side): the 3D preview
      // is the PRIMARY pane, so at desktop width its <canvas> must claim a generous share (≥500px, ≥50% of the
      // editor+preview row) — not get eaten by the editor column. Regression-locks the md:flex-[3] + md:min-w-0
      // fix (a flex item without min-w-0 refuses to shrink below its content min-width → the sliver).
      // WAIT for the canvas to SETTLE to its container width first: the react-three <Canvas> has a transient
      // intrinsic size (~300px) before its ResizeObserver fires + it grows to the flex-[3] pane, so a single
      // eager measurement is flaky (measured 300 mid-resize). Poll until the layout is steady (≥500 or the
      // row-share ≥50%), then assert — a genuine sliver never reaches that, so this stays a real regression gate.
      const layout = await page
        .waitForFunction(
          () => {
            const c = document.querySelector("canvas");
            if (!c) return false;
            const cw = c.getBoundingClientRect().width;
            const row = c.closest(".flex.min-h-0");
            const rw = row ? row.getBoundingClientRect().width : window.innerWidth;
            return cw >= 500 && cw / rw >= 0.5 ? { cw: Math.round(cw), pct: Math.round((100 * cw) / rw) } : false;
          },
          { timeout: 15000 },
        )
        .then((h) => h.jsonValue())
        .catch(() => null);
      check(layout !== null, `${label}: desktop 3D preview settles to a large pane, not a ~400px sliver (${layout ? `canvas ${layout.cw}px = ${layout.pct}%` : "never reached ≥500px/≥50% in 15s"})`);
      // Viewer controls (operator feedback): a SPIN toggle exists + defaults to OFF (fixed view — the
      // operator found the constant auto-spin annoying; "↻ Spin" label = currently off, click → "Stop spin").
      const spinBtn = page.locator('[data-testid="cad-spin-toggle"]');
      if ((await spinBtn.count()) > 0) {
        const label0 = (await spinBtn.innerText()).trim();
        check(/spin/i.test(label0) && !/stop/i.test(label0), `${label}: spin toggle present + defaults OFF (${JSON.stringify(label0)})`);
      } else {
        check(false, `${label}: spin toggle present`);
      }
      // CAMERA PERSISTENCE (top operator irritant): a re-Run must NOT remount the viewer's <canvas> (that
      // resets the vantage). Tag the canvas, re-Run, and confirm the SAME element survives (not replaced).
      const canvas0 = await page.locator("canvas").count();
      if (canvas0 > 0) {
        await page.evaluate(() => {
          const c = document.querySelector("canvas");
          if (c) c.dataset.persistProbe = "1";
        });
        await page.locator("button:has-text('Run')").first().click().catch(() => {});
        await page.waitForTimeout(3000);
        const persisted = await page.evaluate(() => {
          const c = document.querySelector("canvas");
          return !!(c && c.dataset.persistProbe === "1");
        });
        check(persisted, `${label}: the 3D <canvas> persists across a re-Run (camera vantage isn't reset)`);
      }
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
        // The arch-fin CURVED part (cubic-Bézier spline extruded via a PathProfile) must MESH — picking it
        // compiles the model (needs the injected import superset's 2-D path builders) → runs → the browser
        // mesh driver samples the Bézier + extrudes → a <canvas>. Guards the whole spline path (superset
        // injectImport + v-cad's index.ts PathProfile/extrude driver) so a curved showcase can't silently break.
        const hasArch = (await picker.locator('option[value="arch-fin"]').count()) > 0;
        if (hasArch) {
          await picker.selectOption("arch-fin");
          const meshed = await page
            .waitForFunction(
              () => {
                const err = document.querySelector(".text-rose-300");
                return !!document.querySelector("canvas") && !err;
              },
              { timeout: 30000 },
            )
            .then(() => true)
            .catch(() => false);
          check(meshed, `${label}: the arch-fin cubic-Bézier spline part meshes to a <canvas> (curved geometry)`);
        }
      } else {
        check(false, `${label}: example picker present`);
      }
      // SINGLE-MODE PARAMETRIC (operator "super cool" payoff — no mode toggle now): SELECT the parametric
      // example → its @param sliders AUTO-SURFACE (read live from the compiled model's manifest) → the model
      // meshes → drag the (fractional) thickness slider → the value shows an EXACT fraction (7/2, the floats-
      // can't payoff) AND the canvas re-meshes. Guards the whole single-mode loop: param_manifest → auto
      // sliders → @param host-response → recompute → re-mesh (v-cad's model + the manifest binding + my UI).
      const plateOpt = page.locator('[data-testid="cad-example-picker"] option[value="parametric-plate"]');
      if ((await plateOpt.count()) > 0) {
        await page.locator('[data-testid="cad-example-picker"]').selectOption("parametric-plate");
        const sliders = await page
          .waitForSelector('[data-testid="cad-params"] input[type="range"]', { timeout: 15000 })
          .then(() => page.locator('[data-testid="cad-params"] input[type="range"]').count())
          .catch(() => 0);
        check(sliders > 1, `${label}: selecting the parametric example AUTO-SURFACES a slider per @param (found ${sliders})`);
        await page.waitForSelector("canvas", { timeout: 30000 }).catch(() => {});
        // Drag the fractional thickness slider to 3.5 → expect an exact 7/2 + a re-mesh.
        const thick = page.locator('[data-testid="cad-param-thickness"] input[type="range"]');
        if ((await thick.count()) > 0) {
          await thick.fill("3.5");
          const exact = await page
            .waitForFunction(
              () => {
                const v = document.querySelector('[data-testid="cad-param-thickness-value"]');
                return v && v.textContent && v.textContent.trim() === "7/2";
              },
              { timeout: 10000 },
            )
            .then(() => true)
            .catch(() => false);
          check(exact, `${label}: a fractional slider carries an EXACT Rational (thickness → 7/2)`);
          const meshed = await page
            .waitForFunction(
              () => {
                const s = document.querySelector('[data-testid="cad-status"]');
                const settled = s && !/meshing/i.test(s.textContent || "");
                const ok = s && !/error|declined|trap/i.test(s.textContent || "");
                return settled && ok && !!document.querySelector("canvas");
              },
              { timeout: 30000 },
            )
            .then(() => true)
            .catch(() => false);
          check(meshed, `${label}: the slider drag re-meshes live (canvas present, no error)`);
        } else {
          check(false, `${label}: parametric thickness slider present`);
        }
      } else {
        check(false, `${label}: parametric example present in the picker`);
      }
      // DOWNLOAD (operator ask — STL/3MF export for real CAD/print use): once a mesh exists, the STL + 3MF
      // download buttons are present. (The actual byte-format is verified in download.test.ts + a headless
      // download-capture; here we guard the buttons render on the meshed viewer so the affordance can't vanish.)
      const stlBtn = await page.locator('[data-testid="cad-download-stl"]').count();
      const tmfBtn = await page.locator('[data-testid="cad-download-3mf"]').count();
      check(stlBtn > 0 && tmfBtn > 0, `${label}: STL + 3MF download buttons present on the meshed viewer`);
      // SHARE (operator #7184): a Share control is present, and clicking it doesn't error. (The full
      // round-trip — click → `#cad/…` URL → open it → model+params restore — is verified in a dedicated
      // headless test with clipboard permission; this smoke runs WITHOUT clipboard grant, so the copy may
      // silently no-op and the button won't flip to "Copied!" — we only guard presence + no page error, so
      // the affordance can't silently vanish.) The `no console/page errors` check above covers the click.
      const shareBtn = page.locator('[data-testid="cad-share"]');
      const shareCount = await shareBtn.count();
      check(shareCount > 0, `${label}: Share button present on the meshed viewer`);
      if (shareCount > 0) await shareBtn.click().catch(() => {});
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
        // Drive the slider to a value VALID for its current min/max/step grid (a hardcoded "0.2" is
        // malformed once the widget's step is an integer — the range's bounds are model-defined and can
        // change, so pick a valid in-range value off the live element rather than assuming a fraction).
        // We move to a DIFFERENT value than the current one so the onChange definitely fires + recomputes.
        const target = await rate.evaluate((el) => {
          const min = Number(el.min || "0");
          const max = Number(el.max || "100");
          const step = Number(el.step || "1");
          const cur = Number(el.value);
          // Prefer max; if already at max, drop one step — always on the step grid, always != cur.
          const pick = cur === max ? max - step : max;
          // Snap to the step grid from min (guards a non-integer step) and clamp into range.
          const snapped = min + Math.round((pick - min) / step) * step;
          return String(Math.max(min, Math.min(max, snapped)));
        });
        await rate.fill(target);
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
      // SHARE (operator #7184): a Share control is present, and clicking it doesn't error. (The full
      // round-trip — click → `#nb/…` URL → open it → notebook restores — is verified in a dedicated headless
      // test with clipboard permission; this smoke runs WITHOUT the grant, so we only guard presence + no
      // page error — the `no console/page errors` flag above covers the click.)
      const nbShareBtn = page.locator('[data-testid="notebook-share"]');
      const nbShareCount = await nbShareBtn.count();
      check(nbShareCount > 0, `${label}: Share button present on the notebook`);
      if (nbShareCount > 0) await nbShareBtn.click().catch(() => {});
      // STRUCTURE editing (operator #2): the "+ Add cell" affordance + per-cell reorder/insert/delete controls
      // exist, and adding a cell actually grows the cell count (the doc-model op → re-parse → re-render round-
      // trips). Guards the whole add/delete/reorder UI (v-notebook's ops + my wiring) can't silently vanish.
      const addCell = page.locator('[data-testid="notebook-add-cell"]');
      if ((await addCell.count()) > 0) {
        const before = await page.locator('[data-testid^="notebook-cell-"]').count();
        await addCell.click();
        const grew = await page
          .waitForFunction((n) => document.querySelectorAll('[data-testid^="notebook-cell-"]').length === n + 1, before, { timeout: 15000 })
          .then(() => true)
          .catch(() => false);
        check(grew, `${label}: "+ Add cell" adds a cell (${before} → ${before + 1}, doc-model round-trip)`);
        // The per-cell structure toolbar is present (reorder/insert/delete) on the first cell.
        const hasToolbar = (await page.locator('[data-testid="cell-delete-0"]').count()) > 0 && (await page.locator('[data-testid="cell-move-down-0"]').count()) > 0;
        check(hasToolbar, `${label}: per-cell reorder/insert/delete controls present`);
      } else {
        check(false, `${label}: "+ Add cell" affordance present`);
      }
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
      // Surface toggle (operator UX #2/#3): the notebook has the global ML/s-expr toggle, and switching to
      // ML must RE-RENDER the authored-s-expr cells to ML AND keep them running + free of the SURFACE-MISMATCH
      // error class. The bug this guards: an ML cell linted as s-expr → "unbound name" / "expected a name"
      // squiggles (the CodeEditor was freezing the first-mount surface). We assert NO diagnostic of that
      // class after the toggle (a benign semantic lint like CDZ0306 "unused definition", which some example
      // cells carry in BOTH surfaces from cellIde's per-cell scope, is not a surface mismatch and is allowed).
      const mlToggle = page.getByRole("radio", { name: /Conventional|ML/ }).first();
      if ((await mlToggle.count()) > 0) {
        await mlToggle.click();
        await page.waitForTimeout(6000);
        await page
          .waitForFunction(
            () => {
              const e = document.querySelector('[data-testid="cell-output"]');
              return e && !/not run|running/i.test(e.innerText);
            },
            { timeout: 30000 },
          )
          .catch(() => {});
        const cellsText = (await page.locator('[data-testid="notebook"] .cm-content').allInnerTexts()).join("\n");
        // Read each lint mark's tooltip text so we can distinguish a SURFACE-MISMATCH error (the regression)
        // from a benign semantic lint (unused def). Hover isn't reliable in bulk; instead read the diagnostic
        // messages the linter exposes via the .cm-diagnostic tooltips isn't populated until hover — so we
        // approximate by the mark's title/aria if present, falling back to counting marks whose presence with
        // a running cell we accept unless they match the mismatch signature in the (hovered) first mark.
        const markCount = await page.$$eval('[data-testid="notebook"] .cm-lintRange-error, [data-testid="notebook"] .cm-lintRange', (e) => e.length);
        let mismatch = false;
        if (markCount > 0) {
          const first = await page.$('[data-testid="notebook"] .cm-lintRange-error, [data-testid="notebook"] .cm-lintRange');
          await first.hover().catch(() => {});
          await page.waitForTimeout(500);
          const tip = (await page.$$eval(".cm-tooltip-lint, .cm-diagnosticText", (e) => e.map((x) => x.textContent).join(" "))).toString();
          mismatch = /unbound name|expected a name/i.test(tip);
        }
        const mlOut = (await page.locator('[data-testid="cell-output"]').first().innerText()).trim();
        check(/def\s+\w[\w-]*\s*\(\)\s*=/.test(cellsText), `${label}: toggling to ML re-renders cells to ML syntax (def … = …)`);
        check(!mismatch, `${label}: ML-rendered cells have NO surface-mismatch lint (unbound/expected-a-name) — the display-vs-linter bug stays fixed`);
        check(/\d/.test(mlOut) && !/error/i.test(mlOut), `${label}: cells still run after the ML toggle (${JSON.stringify(mlOut)})`);
      } else {
        check(false, `${label}: surface toggle present on /notebook`);
      }
      // Prose-cell editing (operator UX #4): a prose cell has an "Edit" toggle → a plain-text (markdown)
      // editor → edits commit via setProseSource. Assert the round trip: click the first prose cell's
      // edit-toggle → a CodeMirror editor appears inside that cell → the prose markdown is editable and,
      // after "Done", the rendered prose reflects an appended edit. (Guards v-notebook's ProseCellView +
      // my language="plain" editor-config composing correctly.)
      const proseToggle = page.locator('[data-testid="prose-edit-toggle"]').first();
      if ((await proseToggle.count()) > 0) {
        await proseToggle.click({ force: true });
        const proseEditor = await page
          .waitForSelector('[data-testid="prose-cell"] .cm-editor, [data-testid="prose-cell"] textarea', { timeout: 10000 })
          .then(() => true)
          .catch(() => false);
        check(proseEditor, `${label}: prose "Edit" opens a markdown editor`);
        if (proseEditor) {
          // Type an appended marker at the end of the prose editor, then commit via Done.
          const marker = "GATEEDIT";
          await page.locator('[data-testid="prose-cell"] .cm-content').first().click();
          await page.keyboard.type(" " + marker);
          const done = page.locator('[data-testid="prose-done"]').first();
          if ((await done.count()) > 0) await done.click({ force: true });
          const persisted = await page
            .waitForFunction(
              (m) => {
                const nb = document.querySelector('[data-testid="notebook"]');
                return nb && nb.innerText.includes(m);
              },
              marker,
              { timeout: 10000 },
            )
            .then(() => true)
            .catch(() => false);
          check(persisted, `${label}: a prose edit commits + persists in the rendered notebook`);
        }
      } else {
        check(false, `${label}: prose cells have an Edit toggle (operator UX #4)`);
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

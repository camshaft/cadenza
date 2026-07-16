// End-to-end headless verification for the /notebook route (GH #468) — the run-every-example bar applied
// to the notebook: it proves the flagship notebook actually RUNS its cells and REACTS to a widget, in a
// real headless Chromium against the BUILT site. This is the gate that would have caught the starter
// erroring on load (a def-block in replEval's entry slot / a surface mismatch) before it shipped — the
// exact bug v-guide-infra found by hand; this script (their recipe) makes it a repeatable check.
//
// It asserts the two things check:visual can't infer from CSS:
//   1. INITIAL RENDER: the first code cell's output is a COMPUTED VALUE (not an error / still "running"),
//      i.e. the compound-interest balance (~1050), so the cell compiled + ran end-to-end.
//   2. REACTIVE RECOMPUTE: dragging the rate slider RE-RUNS the dependent cell and its output CHANGES
//      (~1050 → ~1200), proving the widget→recompute dataflow works live.
//
// Runs the BUILT site (npm run build first) via `vite preview`, so it checks what actually ships.
//
// ENV (same as check:visual): Chromium needs the platform libstdc++; run via `npm run check:notebook`,
// which wires the shim + node 22. SKIPS with a clear message if the browser or system lib is absent, so
// it never falsely fails a machine that can't run it (a CI image without the browser); a machine that CAN
// run it fails loudly on a real regression.

import { preview } from "vite";

// Load Playwright lazily so the script SKIPS (exit 0) rather than crashing where it isn't installed.
let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  console.log("notebook-verify: playwright not installed — SKIP (install with `npx playwright install chromium`)");
  process.exit(0);
}

const PORT = 4370;
const base = `http://localhost:${PORT}`;
let server;
try {
  server = await preview({ root: process.cwd(), preview: { port: PORT, strictPort: true } });
} catch (e) {
  console.log(`notebook-verify: could not start preview (build first with \`npm run build\`) — ${e.message.split("\n")[0]}`);
  process.exit(0);
}

let browser;
try {
  browser = await chromium.launch();
} catch (e) {
  console.log(`notebook-verify: chromium won't launch (missing system lib?) — SKIP — ${e.message.split("\n")[0]}`);
  await server.close();
  process.exit(0);
}

let fail = 0;
const ok = (cond, msg) => {
  console.log((cond ? "  OK " : "  XX ") + msg);
  if (!cond) fail++;
};

const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
try {
  await page.goto(base + "/notebook", { waitUntil: "networkidle", timeout: 30000 });
  await page.waitForSelector('[data-testid="notebook"]', { timeout: 30000 });

  // Poll until the first cell-output settles (leaves "not run" / "running…").
  await page
    .waitForFunction(
      () => {
        const e = document.querySelector('[data-testid="cell-output"]');
        return e && !/not run|running/i.test(e.innerText);
      },
      { timeout: 30000 },
    )
    .catch(() => {});

  const out0 = await page.locator('[data-testid="cell-output"]').first().innerText();
  ok(/\d/.test(out0) && !/error|running/i.test(out0), "initial cell-output is a computed value: " + JSON.stringify(out0));

  // Drive the rate slider. It's a React CONTROLLED range input: React installs a value-tracker that
  // OVERRIDES a raw `el.value = v` assignment, so onChange never fires and the cell won't recompute
  // (v-guide-infra hit exactly this — the output stayed 1050). Go through the NATIVE value setter React
  // observes, then dispatch `input`. (Never keyboard.type into the CodeMirror editor — auto-indent
  // corrupts multi-line source.)
  await page.$eval(
    '[data-testid="widget-rate"]',
    (el, v) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(el, v);
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    },
    "0.2",
  );

  // Wait for the debounced (150ms) + serialized recompute to produce a new, settled value.
  await page
    .waitForFunction(
      (prev) => {
        const e = document.querySelector('[data-testid="cell-output"]');
        return e && !/running/i.test(e.innerText) && e.innerText !== prev;
      },
      out0,
      { timeout: 30000 },
    )
    .catch(() => {});

  const out1 = await page.locator('[data-testid="cell-output"]').first().innerText();
  ok(out1 !== out0, "rate drag recomputes the cell output: " + JSON.stringify(out0) + " -> " + JSON.stringify(out1));
} catch (e) {
  ok(false, "driver error: " + e.message.split("\n")[0]);
} finally {
  await browser.close();
  await server.close();
}

console.log(fail ? `\n${fail} FAIL` : "\nNOTEBOOK VERIFY: PASS");
process.exit(fail ? 1 : 0);

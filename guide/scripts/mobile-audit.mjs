#!/usr/bin/env node
/// Mobile-layout AUDIT (diagnostic, not a gate) — loads every guide route at a phone viewport (390×844)
/// and reports the layout metrics that decide whether a route reads well on mobile: horizontal overflow,
/// the size of the route's PRIMARY content region (a preview/editor/output pane — too-small or too-big),
/// the smallest interactive tap target (< 44px is below the touch guideline), and the base font size.
///
/// This is a REPORTING tool to inform the coherent mobile-responsive pass (concierge-approved, v-guide-infra
/// leads). It prints a catalog; it does NOT fail. Once each route's fix lands, the per-route assertions go
/// into `check:visual` (the gate) — this script is the survey that precedes them.
///
/// Run: `LD_PRELOAD=/usr/lib64/libstdc++.so.6 LD_LIBRARY_PATH=/usr/lib64:/lib64 node scripts/mobile-audit.mjs`
/// (needs the built site — `npm run build` first — and playwright chromium).

import { preview } from "vite";

const PORT = 4323;
const VP = { width: 390, height: 844 };

// Each route: path, a selector to wait for, an optional surface seed, and the CSS selector of its PRIMARY
// content region (the thing that must be appropriately sized on mobile). `interact` optionally exercises it.
const ROUTES = [
  { path: "/", waitFor: "a[href]", label: "home", primary: "main" },
  { path: "/calculator", waitFor: "input", label: "calculator", primary: "input" },
  { path: "/playground", waitFor: ".cm-content", label: "playground", primary: ".cm-editor" },
  { path: "/cad", waitFor: "button", label: "cad", surface: "sexpr", primary: "div.bg-slate-950" },
  { path: "/notebook", waitFor: '[data-testid="notebook"]', label: "notebook", primary: '[data-testid="notebook"]' },
];

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  console.log("mobile-audit SKIPPED — playwright not installed.");
  process.exit(0);
}
let browser;
try {
  browser = await chromium.launch();
} catch (e) {
  console.log(`mobile-audit SKIPPED — chromium could not launch: ${e.message.split("\n")[0]}`);
  console.log("  (needs: LD_PRELOAD=/usr/lib64/libstdc++.so.6 LD_LIBRARY_PATH=/usr/lib64:/lib64)");
  process.exit(0);
}

const server = await preview({ root: process.cwd(), preview: { port: PORT, strictPort: true } });
const base = `http://localhost:${PORT}`;

console.log(`\n=== MOBILE AUDIT @ ${VP.width}×${VP.height} ===\n`);

try {
  for (const route of ROUTES) {
    const page = await browser.newPage({ viewport: VP });
    const errs = [];
    page.on("console", (m) => m.type() === "error" && errs.push(m.text().slice(0, 80)));
    page.on("pageerror", (e) => errs.push(String(e).slice(0, 80)));
    try {
      if (route.surface) {
        await page.goto(`${base}/`, { waitUntil: "domcontentloaded", timeout: 30000 });
        await page.evaluate((s) => localStorage.setItem("cadenza.syntax", s), route.surface);
      }
      await page.goto(`${base}${route.path}`, { waitUntil: "domcontentloaded", timeout: 30000 });
      await page.waitForSelector(route.waitFor, { timeout: 30000 });
      await page.waitForTimeout(2000); // let lazy chunks / auto-run settle

      const m = await page.evaluate(
        ({ primary, vpH, vpW }) => {
          const doc = document.documentElement;
          const overflow = doc.scrollWidth - doc.clientWidth;
          const p = document.querySelector(primary);
          const pr = p ? p.getBoundingClientRect() : null;
          // Smallest interactive tap target (button / a / input / [role=button]).
          const interactives = [...document.querySelectorAll('button, a[href], input, [role="button"], select')];
          let minTap = Infinity;
          let minTapDesc = "";
          for (const el of interactives) {
            const r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) continue; // hidden
            const dim = Math.min(r.width, r.height);
            if (dim < minTap) {
              minTap = dim;
              minTapDesc = (el.textContent || el.getAttribute("aria-label") || el.tagName).trim().slice(0, 24);
            }
          }
          const bodyFont = parseFloat(getComputedStyle(document.body).fontSize);
          return {
            overflow,
            primaryH: pr ? Math.round(pr.height) : null,
            primaryW: pr ? Math.round(pr.width) : null,
            primaryPctH: pr ? Math.round((pr.height / vpH) * 100) : null,
            primaryPctW: pr ? Math.round((pr.width / vpW) * 100) : null,
            minTap: minTap === Infinity ? null : Math.round(minTap),
            minTapDesc,
            bodyFont,
          };
        },
        { primary: route.primary, vpH: VP.height, vpW: VP.width },
      );

      const flags = [];
      if (m.overflow > 0) flags.push(`⚠ H-OVERFLOW +${m.overflow}px`);
      if (m.primaryPctH != null && m.primaryPctH < 25) flags.push(`⚠ primary TINY (${m.primaryPctH}% vh)`);
      if (m.primaryPctH != null && m.primaryPctH > 92 && route.label !== "notebook" && route.label !== "home")
        flags.push(`⚠ primary DOMINATES (${m.primaryPctH}% vh)`);
      if (m.minTap != null && m.minTap < 32) flags.push(`⚠ tap target ${m.minTap}px ("${m.minTapDesc}") < 44`);
      if (m.bodyFont < 14) flags.push(`⚠ body font ${m.bodyFont}px < 14`);
      if (errs.length) flags.push(`⚠ ${errs.length} console err`);

      console.log(`[${route.label}] ${route.path}`);
      console.log(
        `   overflow=${m.overflow}px  primary(${route.primary})=${m.primaryW}×${m.primaryH}px (${m.primaryPctW}%w ${m.primaryPctH}%h)  minTap=${m.minTap}px  font=${m.bodyFont}px`,
      );
      console.log(`   ${flags.length ? flags.join("  ") : "✓ no obvious mobile issues"}`);
      if (errs.length) console.log(`   errs: ${errs.slice(0, 2).join(" | ")}`);
      console.log("");
    } catch (e) {
      console.log(`[${route.label}] ${route.path}\n   ✗ audit threw: ${String(e.message || e).slice(0, 100)}\n`);
    } finally {
      await page.close();
    }
  }
} finally {
  await browser.close();
  await server.httpServer.close();
}
console.log("=== audit complete (diagnostic only — no gate) ===");

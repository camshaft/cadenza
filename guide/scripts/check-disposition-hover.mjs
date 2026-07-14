// Playwright check for the compilation-DISPOSITION hover in the Playground: type a program with a
// non-recursive fn (inlined), a recursive generic (specialized), and an export (emitted); hover each
// def name; assert the hover bubble shows the right disposition. Deps (playwright) live in the main
// tree's guide/node_modules (symlinked into this worktree). Run with the libstdc++ shim:
//
//   LD_PRELOAD=/usr/lib64/libstdc++.so.6 LD_LIBRARY_PATH=/usr/lib64:/lib64 \
//     mise exec node@22 -- node scripts/check-disposition-hover.mjs

import { preview } from "vite";
import { chromium } from "playwright";

const server = await preview({
  root: process.cwd(),
  preview: { port: 4319, strictPort: true },
});
const base = `http://localhost:4319`;

const browser = await chromium.launch();
const page = await browser.newPage();
const consoleErrors = [];
page.on("console", (m) => {
  if (m.type() === "error") consoleErrors.push(m.text());
});

let failures = 0;
const check = (cond, msg) => {
  console.log(`${cond ? "  ✓" : "  ✗"} ${msg}`);
  if (!cond) failures++;
};

try {
  await page.goto(`${base}/playground`, { waitUntil: "networkidle" });
  await page.waitForSelector(".cm-content", { timeout: 30000 });

  const program = [
    "(def (ident v) v)",
    "(def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))",
    '(def (main (: a Int64)) (+ (ident a) (+ (loopn 3 a) (String.scalar-len (loopn 2 "hi")))))',
    "(export main)",
  ].join("\n");

  await page.click(".cm-content");
  await page.keyboard.press("Control+A");
  await page.keyboard.type(program);
  await page.waitForTimeout(1500);

  async function dismissTooltips() {
    // Move the pointer to a dead corner + press Escape, then wait for every hover bubble to leave the
    // DOM — so a stale tooltip from the previous token can't be mis-read as the current one.
    await page.mouse.move(2, 2);
    await page.keyboard.press("Escape");
    for (let i = 0; i < 20 && (await page.locator(".cm-cadenza-hover").count()) > 0; i++) {
      await page.waitForTimeout(100);
    }
  }

  async function hoverText(token) {
    const loc = page.locator(`.cm-content span:has-text("${token}")`).first();
    await loc.scrollIntoViewIfNeeded();
    for (let attempt = 0; attempt < 6; attempt++) {
      await dismissTooltips();
      await loc.hover({ force: true });
      await page.waitForTimeout(700);
      const tip = page.locator(".cm-cadenza-hover");
      const n = await tip.count();
      if (n > 0) {
        // Read the LAST bubble (the freshest), in case a previous one is mid-fade.
        const t = (await tip.nth(n - 1).innerText()).trim();
        if (t) return t;
      }
    }
    return "";
  }

  const identTip = await hoverText("ident");
  check(/inlined/.test(identTip), `hover 'ident' shows 'inlined' (got: ${JSON.stringify(identTip)})`);

  const loopnTip = await hoverText("loopn");
  check(/specialized/.test(loopnTip), `hover 'loopn' shows 'specialized' (got: ${JSON.stringify(loopnTip)})`);
  check(
    /Int64/.test(loopnTip) && /String/.test(loopnTip),
    `hover 'loopn' lists both instantiations Int64 + String (got: ${JSON.stringify(loopnTip)})`,
  );

  const mainTip = await hoverText("main");
  check(/emitted/.test(mainTip), `hover 'main' shows 'emitted' (got: ${JSON.stringify(mainTip)})`);

  check(consoleErrors.length === 0, `no console errors (saw ${consoleErrors.length}: ${consoleErrors.slice(0, 3).join(" | ")})`);
} catch (e) {
  console.error("driver error:", e);
  failures++;
} finally {
  await browser.close();
  await server.close();
}

console.log(failures === 0 ? "\nDISPOSITION HOVER: all green ✓" : `\nDISPOSITION HOVER: ${failures} failed ✗`);
process.exit(failures === 0 ? 0 : 1);

// Takes a screenshot of a page in headless Chromium. Used by the gateway's code-session preview.
//
//   node tools/screenshot.cjs <url> <output.png>
//
// Needs the `playwright` package and a Chromium it can start (`npx playwright install chromium`).
// Exits non-zero, with the reason on stderr, when the page cannot be loaded or drawn.

const { chromium } = require("playwright");

const [url, output] = process.argv.slice(2);
if (!url || !output) {
  console.error("usage: screenshot.cjs <url> <output.png>");
  process.exit(2);
}

(async () => {
  const browser = await chromium.launch({ args: ["--no-sandbox"] });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const response = await page.goto(url, { waitUntil: "load", timeout: 30000 });
    if (!response) throw new Error("the page did not load");
    await page.waitForTimeout(500);
    await page.screenshot({ path: output, fullPage: false });
  } finally {
    await browser.close();
  }
})().catch((err) => {
  console.error(String(err && err.message ? err.message : err).slice(0, 500));
  process.exit(1);
});

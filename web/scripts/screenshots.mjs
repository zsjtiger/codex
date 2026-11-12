// Screenshot test for the main screens. It creates its own user, workspace and sessions through the
// gateway's API, then opens each screen in a browser and saves a PNG for it.
//
// Run with the gateway and the web dev server already running:
//   node scripts/screenshots.mjs [outputDir]
// Environment: GATEWAY_URL (default http://127.0.0.1:8080), WEB_URL (default http://127.0.0.1:5173).
// Uses the Playwright that is already installed; nothing is downloaded.

import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const GATEWAY = process.env.GATEWAY_URL ?? "http://127.0.0.1:8080";
const WEB = process.env.WEB_URL ?? "http://127.0.0.1:5173";
const outDir = resolve(process.argv[2] ?? "screenshots");
mkdirSync(outDir, { recursive: true });

async function api(method, path, token, body) {
  const response = await fetch(`${GATEWAY}/api/v1${path}`, {
    method,
    headers: { "Content-Type": "application/json", ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`${method} ${path} ${response.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

async function seed() {
  const username = `shots${Date.now()}`;
  const password = "screens-pass-123";
  await api("POST", "/auth/users", null, { username, email: `${username}@example.test`, password, display_name: "Baize" });
  const { token } = await api("POST", "/auth/sessions", null, { username, password });
  const workspace = await api("POST", "/workspaces", token, { slug: `shots-${Date.now()}`, name: "Baize" });
  const environment = await api("POST", `/workspaces/${workspace.id}/environments`, token, { name: "local", endpoint: null });
  await api("POST", `/workspaces/${workspace.id}/threads`, token, { title: "Plan the trip", environment_id: environment.id });
  await api("POST", `/workspaces/${workspace.id}/threads`, token, { title: "Refactor the parser", environment_id: environment.id });
  return { username, password };
}

async function signIn(page, { username, password }) {
  await page.goto(WEB);
  await page.evaluate((gateway) => localStorage.setItem("codex.gatewayUrl", gateway), GATEWAY);
  await page.reload();
  await page.waitForSelector('input[placeholder="your-name"]', { timeout: 15000 });
  await page.fill('input[placeholder="your-name"]', username);
  await page.fill('input[type="password"]', password);
  await page.click('button:has-text("Sign in")');
  await page.waitForSelector(".nav-item", { timeout: 15000 });
}

const shots = [];
async function shot(page, name) {
  await page.waitForTimeout(400);
  const path = join(outDir, `${name}.png`);
  await page.screenshot({ path });
  shots.push(path);
}

const account = await seed();
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1076, height: 760 }, deviceScaleFactor: 2 });
const errors = [];
page.on("pageerror", (error) => errors.push(String(error)));

await signIn(page, account);
await shot(page, "home");

await page.click('.nav-item:has-text("Search")');
await page.waitForSelector('.search-row:has-text("Plan the trip")', { timeout: 10000 });
await shot(page, "search-all");
await page.click('.search-type:has-text("Scheduled")');
await shot(page, "search-scheduled");
await page.keyboard.press("Escape");

await page.click('.mode-toggle button[aria-label="Code"]');
await page.waitForSelector(".code-page", { timeout: 10000 });
await shot(page, "code-page");

await browser.close();

console.log(JSON.stringify({ outDir, screenshots: shots, pageErrors: errors }, null, 2));
if (errors.length > 0) process.exitCode = 1;

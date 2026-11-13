// Browser check for workspace invite links. An admin creates a link in Settings > Members; a guest
// opens it signed out and ends up in the admin's workspace; a second person using the same link is
// refused, because a link works once.
//
// Run with the gateway and the web dev server already running:
//   node scripts/invite-flow.cjs [outputDir]
// Environment: GATEWAY_URL (default http://127.0.0.1:8080), WEB_URL (default http://127.0.0.1:5173).

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

async function register(name) {
  const username = `${name}${Date.now()}`;
  const password = "invite-pass-123";
  await api("POST", "/auth/users", null, { username, email: `${username}@example.test`, password, display_name: name });
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
  await page.waitForSelector(".nav-item", { timeout: 20000 });
}

async function sessionToken(account) {
  return (await api("POST", "/auth/sessions", null, account)).token;
}

const admin = await register("owner");
const guest = await register("guest");
const late = await register("late");
const browser = await chromium.launch();
const failures = [];
const check = (name, ok) => {
  if (!ok) failures.push(name);
  console.log(`${ok ? "ok  " : "FAIL"} ${name}`);
};

// The admin creates the link in Settings > Members.
const adminPage = await browser.newPage({ viewport: { width: 1076, height: 900 } });
await signIn(adminPage, admin);
await adminPage.click(".account-button");
await adminPage.click('.account-menu-item:has-text("Settings")');
await adminPage.click('.settings-nav-item:has-text("Members")');
await adminPage.click('button:has-text("Create invite link")');
await adminPage.waitForSelector('input[aria-label="Invite link"]', { timeout: 15000 });
const link = await adminPage.inputValue('input[aria-label="Invite link"]');
check("the admin gets an invite link", link.includes("?invite="));
await adminPage.screenshot({ path: join(outDir, "invite-admin.png") });

// The guest opens the link signed out: sign-in first, then the invite is accepted.
const target = link.replace(/^https?:\/\/[^/]+/, WEB);
const guestPage = await browser.newPage({ viewport: { width: 1076, height: 900 } });
await guestPage.goto(WEB);
await guestPage.evaluate((gateway) => localStorage.setItem("codex.gatewayUrl", gateway), GATEWAY);
await guestPage.goto(target);
await guestPage.waitForSelector('input[placeholder="your-name"]', { timeout: 15000 });
await guestPage.fill('input[placeholder="your-name"]', guest.username);
await guestPage.fill('input[type="password"]', guest.password);
await guestPage.click('button:has-text("Sign in")');
await guestPage.waitForSelector(".nav-item", { timeout: 20000 });
await guestPage.waitForTimeout(1500);
check("the invite address is cleaned after joining", !new URL(guestPage.url()).search.includes("invite"));
await guestPage.screenshot({ path: join(outDir, "invite-guest.png") });

const adminWorkspaces = await api("GET", "/workspaces", await sessionToken(admin));
const guestWorkspaces = await api("GET", "/workspaces", await sessionToken(guest));
check("the guest is a member of the admin's workspace", guestWorkspaces.some((w) => adminWorkspaces.some((a) => a.id === w.id)));

// The same link refuses a second person.
const latePage = await browser.newPage({ viewport: { width: 1076, height: 900 } });
await latePage.goto(WEB);
await latePage.evaluate((gateway) => localStorage.setItem("codex.gatewayUrl", gateway), GATEWAY);
await latePage.goto(target);
await latePage.waitForSelector('input[placeholder="your-name"]', { timeout: 15000 });
await latePage.fill('input[placeholder="your-name"]', late.username);
await latePage.fill('input[type="password"]', late.password);
await latePage.click('button:has-text("Sign in")');
await latePage.waitForTimeout(3000);
const lateWorkspaces = await api("GET", "/workspaces", await sessionToken(late));
check("a used link does not add a second person", !lateWorkspaces.some((w) => adminWorkspaces.some((a) => a.id === w.id)));
await latePage.screenshot({ path: join(outDir, "invite-reused.png") });

await browser.close();
if (failures.length > 0) {
  console.error(`failed: ${failures.join(", ")}`);
  process.exitCode = 1;
}

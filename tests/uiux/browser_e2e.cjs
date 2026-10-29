#!/usr/bin/env node
/*
 * Browser workflow checks for the embedded pages and Odoo-inspired UI shell.
 * Run against a local gateway:
 * NODE_PATH=/home/ubuntu/codeatlas-platform/node_modules \
 * PLAYWRIGHT_EXECUTABLE_PATH=/home/ubuntu/.cache/ms-playwright/chromium-1228/chrome-linux/chrome \
 * GATEWAY_BASE_URL=http://127.0.0.1:20130 \
 *   node tests/uiux/browser_e2e.cjs
 */
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const baseUrl = process.env.GATEWAY_BASE_URL || "http://127.0.0.1:20130";

async function newPage(browser, viewport = { width: 1440, height: 900 }) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  return { context, page, pageErrors };
}

async function imageSuccess(browser) {
  const { context, page, pageErrors } = await newPage(browser);
  await page.route("**/muse-ai/v1/images/fixture/content", (route) => route.fulfill({
    contentType: "image/svg+xml",
    body: '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>',
  }));
  await page.route("**/muse-ai/v1/create-image*", (route) => route.fulfill({
    contentType: "application/json",
    body: JSON.stringify({ status: "completed", image_url: "/muse-ai/v1/images/fixture/content" }),
  }));
  await page.goto(`${baseUrl}/image-template`, { waitUntil: "domcontentloaded" });
  await page.locator("#prompt").fill("A fixture image");
  const contentResponse = page.waitForResponse((response) => response.url().endsWith("/muse-ai/v1/images/fixture/content"));
  await page.locator("#btn-create").click();
  await page.locator("#result-card").waitFor({ state: "visible" });
  assert.equal(await page.locator("#image-viewer").getAttribute("src"), "/muse-ai/v1/images/fixture/content");
  assert.equal((await contentResponse).ok(), true);
  await page.waitForFunction(() => {
    const image = document.getElementById("image-viewer");
    return image.complete && image.naturalWidth > 0;
  });
  await page.screenshot({ path: "/tmp/screenshots/image_studio_desktop.png", fullPage: true });
  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function imageFailure(browser) {
  const { context, page, pageErrors } = await newPage(browser);
  await page.route("**/muse-ai/v1/create-image*", (route) => route.fulfill({
    status: 400,
    contentType: "application/json",
    body: JSON.stringify({ error: { message: "Fixture image failure" } }),
  }));
  await page.goto(`${baseUrl}/image-template`, { waitUntil: "domcontentloaded" });
  await page.locator("#prompt").fill("A rejected fixture image");
  await page.locator("#btn-create").click();
  await page.getByText("Fixture image failure").waitFor();
  assert.equal(await page.locator("#btn-create").isDisabled(), false);
  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function videoSuccess(browser) {
  const { context, page, pageErrors } = await newPage(browser);
  await page.route("**/muse-ai/v1/videos/fixture/content", (route) => route.fulfill({
    status: 200,
    contentType: "video/mp4",
    body: Buffer.alloc(1024),
  }));
  await page.route("**/muse-ai/v1/create-video", (route) => route.fulfill({
    contentType: "application/json",
    body: JSON.stringify({ status: "completed", media_validated: true, video_url: "/muse-ai/v1/videos/fixture/content" }),
  }));
  await page.goto(`${baseUrl}/video-template`, { waitUntil: "domcontentloaded" });
  await page.locator("#topic").fill("a fixture sunrise");
  await page.locator("#character").fill("a fixture fox");
  const contentResponse = page.waitForResponse((response) => response.url().endsWith("/muse-ai/v1/videos/fixture/content"));
  await page.locator("#go").click();
  await page.locator("#video-player").waitFor({ state: "visible" });
  assert.equal(await page.locator("#video-player").getAttribute("src"), "/muse-ai/v1/videos/fixture/content");
  assert.equal((await contentResponse).ok(), true);
  await page.screenshot({ path: "/tmp/screenshots/video_studio_desktop.png", fullPage: true });
  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function videoNetworkFailure(browser) {
  const { context, page, pageErrors } = await newPage(browser);
  await page.route("**/muse-ai/v1/create-video", (route) => route.abort("failed"));
  await page.goto(`${baseUrl}/video-template`, { waitUntil: "domcontentloaded" });
  await page.locator("#topic").fill("a fixture outage");
  await page.locator("#character").fill("a fixture fox");
  await page.locator("#go").click();
  await page.locator("#errbox").waitFor({ state: "visible" });
  assert.equal(await page.locator("#go").isDisabled(), false);
  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function harFailure(browser) {
  const { context, page, pageErrors } = await newPage(browser);
  await page.route("**/muse-ai/v1/config", (route) => route.fulfill({
    contentType: "application/json",
    body: JSON.stringify({ config: [] }),
  }));
  await page.route("**/muse-ai/v1/config/har", (route) => route.fulfill({
    status: 400,
    contentType: "application/json",
    body: JSON.stringify({ error: { message: "Fixture HAR failure" } }),
  }));
  await page.goto(`${baseUrl}/muse-config`, { waitUntil: "domcontentloaded" });
  await page.locator("#har-file").setInputFiles({
    name: "fixture.har",
    mimeType: "application/json",
    buffer: Buffer.from('{"log":{"entries":[]}}'),
  });
  await page.locator("#apply").click();
  await page.getByText("Fixture HAR failure").waitFor();
  assert.equal(await page.locator("#apply").isDisabled(), false);
  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function dashboardOverviewAndNavigation(browser) {
  const { context, page, pageErrors } = await newPage(browser, { width: 1440, height: 900 });
  await page.goto(`${baseUrl}/dashboard`, { waitUntil: "domcontentloaded" });

  // 1. Verify Topbar & Shell
  await page.locator(".brand").waitFor({ state: "visible" });
  assert.match(await page.locator(".brand-name").textContent(), /Gateway Dashboard/i);
  assert.equal(await page.locator("#view-overview").isVisible(), true);

  // 2. Verify Stats
  await page.locator("#omni-target").waitFor({ state: "visible" });
  await page.locator("#jev-status").waitFor({ state: "visible" });
  await page.locator("#muse-status").waitFor({ state: "visible" });

  // 3. Test Navigation to Installed Modules
  await page.locator('.side-nav [data-go="modules"]').click();
  assert.equal(await page.locator("#view-modules").isVisible(), true);
  assert.equal(await page.locator("#view-overview").isVisible(), false);
  assert.match(await page.locator("#dashboard-breadcrumb").textContent(), /Installed Modules/);

  // 4. Test Search filter in Installed Modules
  await page.locator("#modules-search").fill("omniroute");
  await page.waitForTimeout(100);
  assert.equal(await page.locator("#modules-tbody tr").count() >= 1, true);

  // 5. Test List vs Card Mode toggle
  await page.locator("#cardMode").click();
  assert.equal(await page.locator("#modules-cards-wrap").isVisible(), true);
  await page.locator("#listMode").click();
  assert.equal(await page.locator("#modules-table-wrap").isVisible(), true);

  // 6. Test Navigation to Module Catalog
  await page.locator('.side-nav [data-go="catalog"]').click();
  assert.equal(await page.locator("#view-catalog").isVisible(), true);
  assert.match(await page.locator("#dashboard-breadcrumb").textContent(), /Catalog/);

  // 7. Test Navigation to Provider Settings
  await page.locator('.side-nav [data-go="providers"]').click();
  assert.equal(await page.locator("#view-providers").isVisible(), true);
  assert.equal(await page.locator("#panel-omni").isVisible(), true);

  // Switch Provider Tab
  await page.locator("#tab-muse").click();
  assert.equal(await page.locator("#panel-muse").isVisible(), true);
  assert.equal(await page.locator("#panel-omni").isVisible(), false);

  // 8. Test Navigation to Accounts
  await page.locator('.side-nav [data-go="accounts"]').click();
  assert.equal(await page.locator("#view-accounts").isVisible(), true);
  await page.locator("#accounts-table").waitFor({ state: "visible" });

  // 9. Test Navigation to MCP Control Plane
  await page.locator('.side-nav [data-go="mcp"]').click();
  assert.equal(await page.locator("#view-mcp").isVisible(), true);
  assert.equal(await page.locator("#mcp-tools-list").isVisible(), true);

  // 10. Test Theme Toggle
  await page.locator("#themebtn").click();
  const isDark = await page.evaluate(() => document.body.classList.contains("dark"));
  assert.equal(isDark, true);
  await page.locator("#themebtn").click();
  const isLight = await page.evaluate(() => document.body.classList.contains("dark"));
  assert.equal(isLight, false);

  // Save screenshot of desktop overview
  await page.locator('.side-nav [data-go="overview"]').click();
  const screenshotDir = path.join("/tmp", "screenshots");
  fs.mkdirSync(screenshotDir, { recursive: true });
  await page.screenshot({ path: path.join(screenshotDir, "gateway_odoo_desktop_overview.png"), fullPage: true });

  assert.deepEqual(pageErrors, []);
  await context.close();
}

async function dashboardResponsiveMobile(browser) {
  const { context, page, pageErrors } = await newPage(browser, { width: 390, height: 844 });
  await page.goto(`${baseUrl}/dashboard`, { waitUntil: "domcontentloaded" });

  // Mobile menu button is visible, sidebar is hidden initially
  await page.locator("#menubtn").waitFor({ state: "visible" });
  assert.equal(await page.locator("#sidebar").isVisible(), false);

  // Open mobile menu
  await page.locator("#menubtn").click();
  assert.equal(await page.locator("#sidebar").isVisible(), true);

  // Close with Escape key
  await page.keyboard.press("Escape");
  assert.equal(await page.locator("#sidebar").isVisible(), false);

  const screenshotDir = path.join("/tmp", "screenshots");
  fs.mkdirSync(screenshotDir, { recursive: true });
  await page.screenshot({ path: path.join(screenshotDir, "gateway_odoo_mobile_overview.png"), fullPage: true });

  assert.deepEqual(pageErrors, []);
  await context.close();
}

(async () => {
  const executablePath = process.env.PLAYWRIGHT_EXECUTABLE_PATH || "/home/ubuntu/.cache/ms-playwright/chromium-1228/chrome-linux/chrome";
  const browser = await chromium.launch({
    headless: true,
    executablePath,
  });
  try {
    console.log("Running image workflow tests...");
    await imageSuccess(browser);
    await imageFailure(browser);
    console.log("Running video workflow tests...");
    await videoSuccess(browser);
    await videoNetworkFailure(browser);
    console.log("Running HAR workflow tests...");
    await harFailure(browser);
    console.log("Running Odoo dashboard overview and navigation tests...");
    await dashboardOverviewAndNavigation(browser);
    console.log("Running mobile responsive and drawer tests...");
    await dashboardResponsiveMobile(browser);
    console.log("ALL browser workflow and UI/UX checks passed successfully!");
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error("Browser test error:", error);
  process.exit(1);
});

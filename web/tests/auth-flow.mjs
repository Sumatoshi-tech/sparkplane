// Real TLS/loopback fixture invoked by the ignored Rust browser integration test.
import { chromium } from "@playwright/test";
import assert from "node:assert/strict";

const base = new URL(process.argv[2]);
assert.equal(base.hostname, "localhost");
assert.equal(base.protocol, "https:");
const browser = await chromium.launch();
try {
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  const page = await context.newPage();
  await page.goto(new URL("/panel/#local-auth", base).href);
  await page.getByRole("link", { name: "Authenticate", exact: true }).click();
  await page.waitForURL(new URL("/panel/", base).href);
  await page.getByRole("heading", { name: "Overview", exact: true }).waitFor();
  const result = await page.evaluate(async () => {
    const response = await fetch("/api/sparkplane/v1/web-session");
    const session = await response.json();
    return {
      status: response.status,
      scopes: session.scopes,
      storage: Object.keys(localStorage),
      visibleCookies: document.cookie,
    };
  });
  assert.equal(result.status, 200);
  assert.deepEqual(result.scopes, ["analytics:read"]);
  assert.deepEqual(result.storage, []);
  assert.equal(result.visibleCookies.includes("sparkplane_web="), false);
  await page.getByRole("button", { name: "Scoped access" }).click();
  await page.getByRole("menuitem", { name: "Log out" }).click();
  await page.getByText("Your Spark, one place.").waitFor();
} finally {
  await browser.close();
}

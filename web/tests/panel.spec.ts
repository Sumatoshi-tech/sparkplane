import { expect, test, type Page } from "@playwright/test";

async function fixture(page: Page, scopes = ["admin"]) {
  const summary = {
    requests: 10,
    input_tokens: 12000,
    output_tokens: 3000,
    unknown_usage_requests: 1,
    pending_requests: 0,
    failed_requests: 1,
    accounting_errors: 1,
    long_context_input_tokens: 0,
    long_context_output_tokens: 0,
    cloud_equivalent_usd_nanos: 540000000,
    unpriced_requests: 0,
    comparisons: [
      {
        model: "local-model",
        cloud_model: "local-model",
        provider: "Fixture",
        source: "https://example.com/prices",
        verified_at: "2026-09-30",
        cloud_equivalent_usd_nanos: 540000000,
      },
    ],
    mean_duration_ms: 3000,
    mean_ttft_ms: 80,
    output_tokens_per_second: 30,
  };
  await page.route("**/api/sparkplane/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace(
      "/api/sparkplane/v1/",
      "",
    );
    const payload: Record<string, unknown> = {
      "web-session": {
        token_id: "fixture-token",
        scopes,
        csrf: "fixture-csrf",
      },
      status: {
        agent: "0.1.6",
        executor: "0.1.6",
        read_only: false,
        degraded_reasons: [],
      },
      analytics: {
        from: "2026-09-25",
        to: "2026-10-02",
        summary,
        series: [
          { day: "2026-09-30", input_tokens: 12000, output_tokens: 3000 },
        ],
        breakdown: { model: [{ model: "local-model", ...summary }] },
        rtk: {
          covered_sessions: 2,
          reported_sessions: 2,
          estimated_saved_tokens: 1000,
          estimated_saved_usd_nanos: 2000000,
          unpriced_sessions: 0,
          commands: 8,
        },
        catalog: {
          version: "fixture",
          models: [
            {
              model: "local-model",
              input_microusd_per_million: 2000000,
              verified_at: "2026-09-30",
              source: "https://example.com/prices",
            },
          ],
        },
      },
      models: {
        models: [
          {
            id: "m_fixture",
            canonical: "local-model",
            repository: "owner/model",
            unique_bytes: 1024 ** 3,
            verified_at: "2026-10-01",
            active_instances: [],
          },
        ],
      },
      instances: {
        instances: [
          {
            id: "i_fixture",
            name: "local",
            model: "local-model",
            engine_id: "vllm",
            healthy: true,
            desired: "running",
            observed: "healthy",
            generation: 1,
            context_window: 262144,
            resources: { steady_peak_bytes: 1024 ** 3 },
          },
        ],
      },
      operations: {
        operations: [
          {
            id: "operation1",
            kind: "model.download",
            state: "running",
            actor_token_id: "fixture-token",
            target: "model",
            updated_at: "2026-10-01",
            progress: {
              stage: "downloading",
              current: 50,
              total: 100,
              unit: "bytes",
            },
          },
        ],
      },
      health: {
        observed_at_unix_ms: Date.now() - 30000,
        cpu_percent: 10,
        executor: {
          gpu: {
            name: "NVIDIA GB10",
            utilization_percent: 20,
            dedicated_memory_total_mib: null,
          },
          resources: {
            mem_available_bytes: 32 * 1024 ** 3,
            mem_total_bytes: 128 * 1024 ** 3,
            disk_available_bytes: 500 * 1024 ** 3,
          },
          health: { guard_heartbeat: true, event_heartbeat: true },
        },
        database: {
          backup_valid: true,
          journal_mode: "wal",
          queue_capacity: 64,
        },
      },
      doctor: { checks: [] },
      tokens: { tokens: [] },
      "certificates/status": { valid: true, dns_sans: ["spark"], ip_sans: [] },
    };
    await route.fulfill({
      json: payload[path] ?? { items: [], next_offset: null },
    });
  });
}
test("direct visit waits for the helper and discovers it without a reload", async ({
  page,
}) => {
  await page.route("**/api/sparkplane/v1/web-session", (route) =>
    route.fulfill({ status: 401, json: { detail: "authentication failed" } }),
  );
  let running = false;
  await page.route("http://127.0.0.1:9844/health", (route) =>
    running
      ? route.fulfill({
          headers: { "access-control-allow-origin": "http://127.0.0.1:5173" },
          json: {
            protocol: "sparkplane.webauth/v1",
            origin: "http://127.0.0.1:5173",
          },
        })
      : route.abort(),
  );
  await page.goto("/panel/");
  await expect(
    page.getByText("sparkplane webauthsvc start", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Authenticate", exact: true }),
  ).toHaveCount(0);
  running = true;
  await expect(
    page.getByRole("link", { name: "Authenticate", exact: true }),
  ).toBeVisible({ timeout: 7000 });
  await expect(
    page.getByRole("link", { name: "Authenticate", exact: true }),
  ).toHaveAttribute("href", "http://127.0.0.1:9844/auth/start");
});
test("CLI entry and blocked discovery retain the direct redirect path", async ({
  page,
}) => {
  await page.route("**/api/sparkplane/v1/web-session", (route) =>
    route.fulfill({ status: 401, json: { detail: "authentication failed" } }),
  );
  await page.route("http://127.0.0.1:9844/health", (route) => route.abort());
  await page.goto("/panel/#local-auth");
  await expect(
    page.getByRole("link", { name: "Authenticate", exact: true }),
  ).toBeVisible();
  await page.goto("/panel/");
  await page
    .getByRole("button", { name: "I started the local service" })
    .click();
  await expect(
    page.getByRole("link", { name: "Authenticate", exact: true }),
  ).toBeVisible();
});
test("overview renders real data and scoped navigation", async ({
  page,
}, testInfo) => {
  const paths: string[] = [];
  page.on("request", (request) => paths.push(new URL(request.url()).pathname));
  await fixture(page, ["analytics:read"]);
  await page.goto("/panel/");
  await expect(page.getByText("15K", { exact: true })).toBeVisible();
  await expect(page.getByText("$0.54", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(page.viewportSize()!.width);
  await page.screenshot({
    path: testInfo.outputPath("overview.png"),
    fullPage: true,
  });
  await expect(page.getByRole("link", { name: "Administration" })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("link", { name: "Models", exact: true }),
  ).toHaveCount(0);
  await page.goto("/panel/admin");
  await expect(
    page.getByRole("heading", { name: "Access unavailable" }),
  ).toBeVisible();
  expect(await page.evaluate(() => Object.keys(localStorage))).toEqual([]);
  expect(paths.some((path) => path.endsWith("/status"))).toBe(false);
  expect(paths.some((path) => path.endsWith("/operations"))).toBe(false);
});
test("changing analytics range never labels the previous data as the new range", async ({
  page,
}) => {
  await fixture(page);
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    "**/api/sparkplane/v1/analytics?days=30**",
    async (route) => {
      await pending;
      await route.fallback();
    },
  );
  await page.goto("/panel/analytics");
  await expect(page.getByText("15K", { exact: true })).toBeVisible();
  await page.getByRole("combobox", { name: "Date range" }).click();
  await page.getByRole("option", { name: "Last 30 days", exact: true }).click();
  try {
    await expect(page.getByText("15K", { exact: true })).toHaveCount(0);
  } finally {
    release();
  }
  await expect(page.getByText("15K", { exact: true })).toBeVisible();
});
test("unpriced usage never falls back to another model's tariff", async ({
  page,
}) => {
  await fixture(page);
  await page.route("**/api/sparkplane/v1/analytics?**", (route) =>
    route.fulfill({
      json: {
        summary: {
          requests: 1,
          input_tokens: 100,
          output_tokens: 20,
          cloud_equivalent_usd_nanos: null,
          unpriced_requests: 1,
          comparisons: [
            {
              model: "unpriced-model",
              cloud_model: null,
              cloud_equivalent_usd_nanos: null,
            },
          ],
        },
        rtk: {
          covered_sessions: 1,
          estimated_saved_tokens: 1000,
          estimated_saved_usd_nanos: null,
          unpriced_sessions: 1,
        },
        catalog: {
          version: "fixture",
          models: [
            { model: "gpt-6.1-sol", input_microusd_per_million: 2000000 },
          ],
        },
        breakdown: { model: [{ model: "unpriced-model" }] },
        series: [],
      },
    }),
  );
  await page.goto("/panel/analytics");
  await expect(page.getByText("120", { exact: true })).toBeVisible();
  await expect(page.getByText("Unknown", { exact: true })).toHaveCount(2);
  await expect(
    page.getByText("1 requests without a verified same-model tariff"),
  ).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Cloud comparison" }),
  ).toHaveCount(0);
  await expect(page.getByText("gpt-6.1-sol", { exact: true })).toHaveCount(0);
});
test("managed controls preview before sending a CSRF-protected mutation", async ({
  page,
}) => {
  await fixture(page);
  const mutations: { body: unknown; csrf: string | undefined }[] = [];
  await page.route(
    "**/api/sparkplane/v1/instances/i_fixture",
    async (route) => {
      mutations.push({
        body: route.request().postDataJSON(),
        csrf: route.request().headers()["x-sparkplane-csrf"],
      });
      await route.fulfill({ json: { id: "stop-op", state: "accepted" } });
    },
  );
  await page.goto("/panel/instances");
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Confirm", exact: true }),
  ).toBeVisible();
  expect(mutations[0].body).toEqual({ dry_run: true });
  await page.getByRole("button", { name: "Confirm", exact: true }).click();
  await expect.poll(() => mutations.length).toBe(2);
  expect(mutations[1].body).toEqual({ timeout_seconds: 30 });
  expect(mutations[1].csrf).toBe("fixture-csrf");
});
test("health labels stale samples and unavailable dedicated memory", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/panel/health");
  await expect(page.getByText("stale", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Unavailable · unified memory architecture"),
  ).toBeVisible();
});
test("keyboard opens navigation on a narrow viewport", async ({ page }) => {
  await fixture(page);
  await page.goto("/panel/");
  await page.getByRole("button", { name: "Toggle Sidebar" }).focus();
  await page.keyboard.press("Enter");
  if (test.info().project.name === "mobile") {
    await expect(
      page.getByRole("link", { name: "Analytics", exact: true }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
  }
  await expect(
    page.getByRole("heading", { name: "Overview", exact: true }),
  ).toBeVisible();
});
test("all panel pages fit the viewport without shrinking the mobile layout", async ({
  page,
}) => {
  await fixture(page);
  const operationViews: (string | null)[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/sparkplane/v1/operations")
      operationViews.push(url.searchParams.get("view"));
  });
  for (const path of [
    "overview",
    "analytics",
    "models",
    "instances",
    "operations",
    "health",
    "admin",
  ]) {
    await page.goto(`/panel/${path}`);
    await page.waitForLoadState("networkidle");
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
      path,
    ).toBeLessThanOrEqual(page.viewportSize()!.width);
  }
  expect(operationViews.length).toBeGreaterThan(0);
  expect(operationViews.every((view) => view === "summary")).toBe(true);
});

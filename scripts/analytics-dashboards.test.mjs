import { test } from "node:test";
import assert from "node:assert/strict";
import { analyticsDashboards } from "./analytics-dashboards.mjs";
import { provisionAnalytics } from "./provision-analytics.mjs";

test("dashboard definitions separate durable events, navigation, models and usage coverage", () => {
  assert.equal(analyticsDashboards.length, 4);
  const tiles = analyticsDashboards.flatMap(dashboard => dashboard.tiles);
  assert.equal(new Set(tiles.map(tile => tile.name)).size, tiles.length);
  const eventSeries = tiles.flatMap(tile => tile.query.source.series ?? []);
  for (const series of eventSeries) {
    assert(series.properties.some(property => property.key === "analytics_schema_version" && property.value[0] === 2));
    assert(series.properties.some(property => property.key === "surface"));
    assert(["total", "dau", "sum", "median", "p95"].includes(series.math));
    if (["sum", "median", "p95"].includes(series.math)) assert(series.math_property);
  }
  assert(eventSeries.some(series => series.event === "chat created"));
  assert(eventSeries.some(series => series.event === "chat opened"));
  assert(eventSeries.some(series => series.event === "message submitted"));
  assert(tiles.some(tile => tile.query.source.breakdownFilter?.breakdown === "actual_model"));
  assert(tiles.some(tile => tile.query.source.breakdownFilter?.breakdown === "usage_status"));
  const funnel = tiles.find(tile => tile.query.source.kind === "FunnelsQuery").query.source;
  assert.deepEqual(funnel.series.map(series => series.order), [0, 1, 2]);
});

test("provisioning twice updates existing objects and preserves dashboard memberships", async () => {
  const rows = { dashboards: [], insights: [] };
  let id = 1;
  let creations = 0;
  const fetchImpl = async (url, options) => {
    assert.equal(url.origin, "https://posthog.test");
    assert.equal(options.headers.authorization, "Bearer operations-test-key");
    assert.equal(options.redirect, "error");
    const [, , , , resource, rowId] = url.pathname.split("/");
    if (options.method === "GET") return Response.json({ results: rows[resource], next: null });
    const body = JSON.parse(options.body);
    if (options.method === "POST") {
      const row = { id: id++, ...body };
      rows[resource].push(row);
      creations++;
      return Response.json(row);
    }
    const row = rows[resource].find(row => row.id === Number(rowId));
    Object.assign(row, body);
    return Response.json(row);
  };
  const options = { host: "https://posthog.test", projectId: "123", apiKey: "operations-test-key", fetchImpl };
  const first = await provisionAnalytics(options);
  const originalCreations = creations;
  rows.insights[0].dashboards.push(999);
  const second = await provisionAnalytics(options);
  assert.deepEqual(first, second);
  assert.equal(creations, originalCreations);
  assert(rows.insights[0].dashboards.includes(999));
  assert.equal(rows.dashboards.length, 4);
});

test("pagination cannot forward operations credentials to another host", async () => {
  let calls = 0;
  await assert.rejects(provisionAnalytics({
    host: "https://posthog.test", projectId: "123", apiKey: "operations-test-key",
    fetchImpl: async () => { calls++; return Response.json({ results: [], next: "https://evil.test/steal" }); },
  }), /Invalid PostHog pagination URL/);
  assert.equal(calls, 1);
});

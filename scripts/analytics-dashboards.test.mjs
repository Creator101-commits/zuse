import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { analyticsDashboards } from "./analytics-dashboards.mjs";
import { provisionAnalytics } from "./provision-analytics.mjs";

test("dashboard definitions separate durable events, navigation, models and usage coverage", () => {
  assert.equal(analyticsDashboards.length, 5);
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
  assert.equal(rows.dashboards.length, 5);
});

test("pagination cannot forward operations credentials to another host", async () => {
  let calls = 0;
  await assert.rejects(provisionAnalytics({
    host: "https://posthog.test", projectId: "123", apiKey: "operations-test-key",
    fetchImpl: async () => { calls++; return Response.json({ results: [], next: "https://evil.test/steal" }); },
  }), /Invalid PostHog pagination URL/);
  assert.equal(calls, 1);
});


test("native dashboard series correspond to the closed Rust catalog and actual native chat events", () => {
  const catalog = readFileSync(new URL("../crates/analytics/src/desktop.rs", import.meta.url), "utf8");
  const events = new Set([...catalog.matchAll(/#\[serde\(rename = "([^"]+)"\)\]/g)].map(match => match[1]));
  events.add("chat opened");
  const native = analyticsDashboards.find(dashboard => dashboard.name === "Zuse — Native desktop");
  assert(native);
  const nodes = native.tiles.flatMap(tile => tile.query.source.series);
  for (const node of nodes) {
    const surface = node.properties.find(property => property.key === "surface").value[0];
    if (surface === "desktop") assert(events.has(node.event), node.event);
    else assert(["message submitted", "turn completed"].includes(node.event));
  }
  const funnel = native.tiles.find(tile => tile.query.source.kind === "FunnelsQuery").query.source;
  assert.deepEqual(funnel.series.map(node => node.order), [0, 1, 2, 3]);
  assert(nodes.some(node => node.math_property === "active_seconds"));
});

test("native release and distribution source builds use optional public repository variables", () => {
  for (const file of ["native-compile.yml", "distributions.yml"]) {
    const workflow = readFileSync(new URL(`../.github/workflows/${file}`, import.meta.url), "utf8");
    for (const key of ["ZUSE_POSTHOG_KEY", "ZUSE_POSTHOG_HOST"]) {
      assert(workflow.includes(key + ": ${{ vars." + key + " }}"));
    }
    assert(!workflow.includes("POSTHOG_PERSONAL_API_KEY"));
  }
  const prebuild = readFileSync(new URL("../.github/workflows/prebuild.yml", import.meta.url), "utf8");
  const binaries = readFileSync(new URL("../.github/workflows/native-binaries.yml", import.meta.url), "utf8");
  const release = readFileSync(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");
  assert(prebuild.includes("./.github/workflows/native-binaries.yml"));
  assert(binaries.includes("./.github/workflows/native-compile.yml"));
  assert(release.includes("./.github/workflows/native-compile.yml"));
});

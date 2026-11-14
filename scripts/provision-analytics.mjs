#!/usr/bin/env node
import { pathToFileURL } from "node:url";
import { analyticsDashboards } from "./analytics-dashboards.mjs";

// Dry-run by default. Operations credentials are confined to this script.
export async function provisionAnalytics({ host, projectId, apiKey, fetchImpl = fetch }) {
  const base = new URL(`/api/projects/${encodeURIComponent(projectId)}/`, host);
  const request = async (path, method = "GET", body) => {
    const url = new URL(path, base);
    if (url.origin !== base.origin || !url.pathname.startsWith(base.pathname)) throw new Error("Invalid PostHog pagination URL");
    const response = await fetchImpl(url, {
      method, redirect: "error", signal: AbortSignal.timeout(30_000),
      headers: { authorization: `Bearer ${apiKey}`, "content-type": "application/json" },
      ...(body ? { body: JSON.stringify(body) } : {}),
    });
    if (!response.ok) throw new Error(`PostHog ${method} ${url.pathname}: HTTP ${response.status}`);
    return response.json();
  };
  const list = async (resource) => {
    const rows = [];
    let next = `${resource}/?limit=200`;
    const visited = new Set();
    while (next) {
      if (visited.has(next)) throw new Error("PostHog pagination cycle");
      visited.add(next);
      const page = await request(next);
      rows.push(...page.results.filter(row => !row.deleted));
      next = page.next;
    }
    return rows;
  };
  const dashboards = await list("dashboards");
  const insights = await list("insights");
  const upsert = async (resource, rows, definition) => {
    const existing = rows.find(row => row.name === definition.name);
    const saved = await request(`${resource}/${existing ? `${existing.id}/` : ""}`, existing ? "PATCH" : "POST", definition);
    if (!existing) rows.push(saved);
    return saved;
  };
  const links = [];
  for (const definition of analyticsDashboards) {
    const dashboard = await upsert("dashboards", dashboards, { name: definition.name, description: definition.description });
    for (const tile of definition.tiles) {
      const existing = insights.find(insight => insight.name === tile.name);
      const memberships = [...new Set([...(existing?.dashboards ?? []), dashboard.id])];
      await upsert("insights", insights, { ...tile, dashboards: memberships, saved: true });
    }
    links.push(`${new URL(host).origin}/project/${projectId}/dashboard/${dashboard.id}`);
  }
  return links;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (!process.argv.includes("--apply")) {
    console.log(JSON.stringify(analyticsDashboards, null, 2));
  } else {
    const apiKey = process.env.POSTHOG_PERSONAL_API_KEY?.trim();
    const projectId = process.env.POSTHOG_PROJECT_ID?.trim();
    if (!apiKey || !projectId) throw new Error("Set POSTHOG_PERSONAL_API_KEY and POSTHOG_PROJECT_ID for --apply");
    const links = await provisionAnalytics({ host: process.env.POSTHOG_CONTROL_HOST || "https://us.posthog.com", projectId, apiKey });
    console.log(links.join("\n"));
  }
}

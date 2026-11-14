import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { ActiveTimeTracker, analyticsAccountId, sanitizeAnalyticsProperties } from "../src/index";
import { sanitizeAnalyticsEnvelope } from "../src/envelope";

describe("analytics privacy boundary", () => {
  test("strips SDK URLs, referrers, content, profiles, and unknown events", () => {
    const id = "anonymous_00000000-0000-4000-8000-000000000000";
    const result = sanitizeAnalyticsEnvelope({ event: "screen viewed", properties: {
      token: "public-ingest-key", distinct_id: id, screen: "settings",
      $current_url: "https://private.example/repo?token=secret", $referrer: "private",
      title: "private", prompt: "private", $set: { email: "private@example.com" },
    } });
    expect(result?.properties).toEqual({ token: "public-ingest-key", distinct_id: id,
      screen: "settings", analytics_schema_version: 2, $process_person_profile: false });
    expect(sanitizeAnalyticsEnvelope({ event: "$identify", properties: {} })).toBeNull();
    expect(sanitizeAnalyticsEnvelope({ event: "$autocapture", properties: {} })).toBeNull();
    expect(sanitizeAnalyticsEnvelope({ event: "screen viewed", properties: { distinct_id: "private-account" } })).toBeNull();
  });
  test("normalizes private dimensions and rejects invalid measurements", () => {
    expect(sanitizeAnalyticsProperties("turn completed", { model: "my-company-model", provider: "private-provider",
      duration_ms: Infinity, prompt: "private", path: "/private/file", tool_count: 3,
    })).toEqual({ model: "custom", provider: "other", tool_count: 3, analytics_schema_version: 2 });
    expect(sanitizeAnalyticsProperties("screen viewed", { screen: "private-title" }).screen).toBe("other");
  });
  test("account hashes use the same namespace as Rust and mobile", () => {
    const expected = createHash("sha256").update("zuse:analytics:account:v1:example-user").digest("hex");
    expect(analyticsAccountId("example-user")).toBe(`account_${expected}`);
  });
});

test("active time excludes background and idle time", () => {
  let now = 0;
  const intervals: number[] = [];
  const tracker = new ActiveTimeTracker({ now: () => now, onInterval: ({ activeSeconds }) => intervals.push(activeSeconds) });
  tracker.foreground();
  now = 120_000;
  tracker.background();
  expect(intervals).toEqual([60]);
  now = 240_000;
  tracker.tick();
  tracker.flush();
  expect(intervals).toEqual([60]);
  tracker.foreground();
  now = 250_000;
  tracker.background();
  expect(intervals).toEqual([60, 10]);
});

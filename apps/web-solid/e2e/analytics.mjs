// Real slim SDK, intercepted beacons, no engine or production ingestion.
// Run: bun e2e/analytics.mjs
import { spawn } from "node:child_process";
import { strict as assert } from "node:assert";
import { openBrowser } from "./harness.mjs";

const port = 31000 + Math.floor(Math.random() * 1000);
const url = `http://127.0.0.1:${port}/e2e/analytics-fixture/?secret=private-query#private-fragment`;
const vite = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", String(port)], {
  cwd: new URL("..", import.meta.url),
  env: { ...process.env, VITE_POSTHOG_KEY: "test-project", VITE_POSTHOG_HOST: `http://127.0.0.1:${port}`, VITE_POSTHOG_ENABLE_DEV: "1" },
  stdio: "inherit",
});
let page;
try {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if ((await fetch(url, { signal: AbortSignal.timeout(1000) })).ok) break; } catch {}
    if (attempt === 99) throw new Error("Vite did not start");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  page = await openBrowser("about:blank");
  await page.call("Network.enable");
  await page.call("Network.setUserAgentOverride", { userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36" });
  await page.call("Network.setBlockedURLs", { urls: ["*posthog.com*", "*posthog.net*"] });
  await page.call("Page.addScriptToEvaluateOnNewDocument", { source: `
    window.beacons = [];
    navigator.sendBeacon = (url, blob) => {
      (async () => {
        const bytes = new Uint8Array(await blob.arrayBuffer());
        let text;
        if (bytes[0] === 31 && bytes[1] === 139) {
          text = await new Response(blob.stream().pipeThrough(new DecompressionStream('gzip'))).text();
        } else {
          text = await blob.text();
          if (text.startsWith('data=')) text = atob(new URLSearchParams(text).get('data'));
        }
        const parsed = JSON.parse(text);
        window.beacons.push(...(Array.isArray(parsed) ? parsed : parsed.batch ?? [parsed]));
      })();
      return true;
    };
  ` });
  await page.goto(url);
  await page.waitFor("window.beacons.length >= 2");
  const first = await page.eval("window.beacons");
  assert(first.some(event => event.event === "app opened"));
  assert(first.some(event => event.event === "screen viewed"));
  const serialized = JSON.stringify(first);
  assert(!serialized.includes("private-query"));
  assert(!serialized.includes("private-fragment"));
  assert(!serialized.includes("$current_url"));
  assert(first.every(event => event.properties.$process_person_profile === false));
  assert(first.every(event => event.properties.distinct_id.startsWith("anonymous_")));
  await page.eval("window.analyticsTest.chat('local-chat-one')");
  await page.waitFor("window.opened.length === 1");
  await page.eval("window.analyticsTest.reconnect()");
  assert.equal(await page.eval("window.opened.length"), 1);
  await page.eval("window.analyticsTest.chat('local-chat-one')");
  assert.equal(await page.eval("window.opened.length"), 1);
  await page.eval("window.analyticsTest.chat('local-chat-two')");
  await page.waitFor("window.opened.length === 2");
  assert.deepEqual(await page.eval("window.opened.map(event => event.surface)"), ["web", "web"]);
  await page.eval(`window.analyticsTest.account('private-account-id')`);
  await page.eval(`window.analyticsTest.captureAnalytics('screen viewed', {screen:'settings', prompt:'private-prompt'})`);
  await page.waitFor("window.beacons.some(event => event.properties.distinct_id.startsWith('account_'))");
  assert(!(await page.eval("JSON.stringify(window.beacons)")).includes("private-account-id"));
  assert(!(await page.eval("JSON.stringify(window.beacons)")).includes("private-prompt"));
  await page.eval("window.analyticsTest.setAnalyticsEnabled(false)");
  const count = await page.eval("window.beacons.length");
  await page.eval("window.analyticsTest.chat('local-chat-three')");
  assert.equal(await page.eval("window.opened.length"), 2);
  await page.eval("window.analyticsTest.captureAnalytics('screen viewed', {screen:'settings'})");
  assert.equal(await page.eval("window.beacons.length"), count);
  await page.eval("window.analyticsTest.setAnalyticsEnabled(true)");
  await page.waitFor(`window.beacons.length > ${count}`);
  await page.eval("window.analyticsTest.setAnalyticsEnabled(false)");
  await page.goto(url.replace("?secret=", "?reload=1&secret="));
  await page.waitFor("!!window.analyticsTest");
  assert.equal(await page.eval("window.beacons.length"), 0);
  assert.equal(await page.eval("window.analyticsTest.analyticsEnabled()"), false);
  // A privacy signal overrides an enabled saved preference on the next load.
  await page.call("Page.addScriptToEvaluateOnNewDocument", { source: `
    localStorage.setItem('zuse.web.analytics.enabled.v1', 'true');
    Object.defineProperty(navigator, 'globalPrivacyControl', {value: true});
  ` });
  await page.goto(url.replace("?secret=", "?reload=2&secret="));
  await page.waitFor("!!window.analyticsTest");
  assert.equal(await page.eval("window.beacons.length"), 0);
  assert.deepEqual(page.errors, []);
  console.log("analytics browser smoke passed: real SDK, privacy, identity, opt-out, reload, GPC");
} finally {
  await page?.close();
  vite.kill("SIGTERM");
}

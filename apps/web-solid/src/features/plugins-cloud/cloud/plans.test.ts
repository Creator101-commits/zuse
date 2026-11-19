// Run with `bun test src`. Mirrors crates/protocol/src/cloud_billing.rs `plans` tests for the web copy.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { columnCell, filledRows, litColumns, meterCell } from "./ditherCells";
import { CLOUD_MAX_OFFER, CLOUD_OFFER, CLOUD_TRIAL_OFFER, cloudSubscribed, count, dollars, hours, LEGACY_OFFER } from "./money";
import {
  allowanceLine,
  approxHours,
  cloudErrorText,
  defaultProvider,
  extraLine,
  FALLBACK_CATALOG,
  holdNotice,
  meterHint,
  meterLine,
  offerAllowance,
  offerMachines,
  planTitle,
  readCatalog,
  renewLine,
  tierOf,
  TRIAL_COPY,
  trialCopy,
  trialLine,
  usageHint,
  usageMinutes,
  usedFraction,
} from "./plans";
import type { BillingUsageItem, HoldReason, PlanUsage } from "./types";
import { byChat, byDay, bySize } from "./usage";

/** Same rule as `names_infrastructure`: a provider name as a whole word, any case. */
function namesInfrastructure(text: string) {
  const lower = text.toLowerCase();
  return ["boxd", "boat", "prized", "e2b", "daytona"].some((name) => {
    for (let at = lower.indexOf(name); at >= 0; at = lower.indexOf(name, at + 1)) {
      const before = lower[at - 1];
      const after = lower[at + name.length];
      if (!(before && /[a-z0-9]/.test(before)) && !(after && /[a-z0-9]/.test(after))) return true;
    }
    return false;
  });
}

const renewsAt = Date.UTC(2026, 11, 1, 12);
const cloud: PlanUsage = {
  planId: "cloud",
  planName: "Cloud",
  priceMicros: 20_000_000,
  trial: false,
  renewsAt,
  includedMachineMinutes: 33_600,
  usedMachineMinutes: 12_480,
  remainingMachineMinutes: 21_120,
  extraMachineMinutes: 0,
  extraChargeMicros: 0,
  overageCapMicros: 25_000_000,
  payAsYouGo: true,
  usageHold: false,
  defaultMachineRate: 4,
  maxVcpus: 4,
  maxMemoryMib: 16384,
};
const over: PlanUsage = { ...cloud, usedMachineMinutes: 36_480, remainingMachineMinutes: 0, extraMachineMinutes: 2_880, extraChargeMicros: 1_714_286 };
const trial: PlanUsage = {
  ...cloud,
  planId: "cloud-trial",
  planName: "Cloud trial",
  priceMicros: 1_000_000,
  trial: true,
  trialEndsAt: Date.UTC(2026, 10, 26, 12),
  includedMachineMinutes: 8_400,
  usedMachineMinutes: 5_200,
  remainingMachineMinutes: 3_200,
  payAsYouGo: false,
};
const item = (patch: Partial<BillingUsageItem>): BillingUsageItem => ({
  entryId: "e",
  resourceKind: "workspace",
  resourceId: "w1",
  provider: "cloud",
  startedAt: 0,
  endedAt: 0,
  vcpuCount: 4,
  memoryMib: 16384,
  providerCostMicros: 0,
  status: "final",
  ...patch,
});

test("counts use separators, hours are whole from 10 h, one decimal below", () => {
  assert.equal(count(33_600), "33,600");
  assert.equal(count(-3), "0");
  assert.equal(hours(0), "0 h");
  assert.equal(hours(2), "<0.1 h");
  assert.equal(hours(90), "1.5 h");
  assert.equal(hours(140 * 60), "140 h");
  assert.equal(approxHours(33_600), "about 140 h");
  assert.equal(approxHours(84_000), "about 350 h");
  assert.equal(approxHours(8_400), "about 35 h");
  assert.equal(approxHours(21_120, 4), "about 88 h");
});

test("plan prices drop zero cents", () => {
  assert.equal(dollars(20_000_000), "$20");
  assert.equal(dollars(1_714_286), "$1.71");
});

test("every cloud plan offer counts as subscribed while active", () => {
  const now = 1_000;
  const subscribed = (offerId: string, status = "active", paidThrough: number | null = 2_000) => cloudSubscribed({ entitlements: [{ offerId, status, paidThrough }] }, now);
  for (const offer of [CLOUD_OFFER, CLOUD_MAX_OFFER, LEGACY_OFFER]) {
    assert.equal(subscribed(offer), true, offer);
    assert.equal(subscribed(offer, "pending", null), true, offer);
    assert.equal(subscribed(offer, "grace"), true, offer);
    assert.equal(subscribed(offer, "ended"), false, offer);
    assert.equal(subscribed(offer, "active", 500), false, offer);
  }
  assert.equal(subscribed(CLOUD_TRIAL_OFFER), false);
  assert.equal(subscribed("persistent-standard-v1"), false);
  assert.equal(cloudSubscribed({}, now), null);
});

test("the catalog falls back to Cloud and Cloud Max without a trial", () => {
  assert.deepEqual(readCatalog({ entitlements: [] }), FALLBACK_CATALOG);
  assert.deepEqual(
    FALLBACK_CATALOG.plans.map((p) => [p.offerId, p.planId, p.name, p.includedMachineMinutes]),
    [
      [CLOUD_OFFER, "cloud", "Cloud", 33_600],
      [CLOUD_MAX_OFFER, "cloud-max", "Cloud Max", 84_000],
    ],
  );
  const catalog = readCatalog({
    entitlements: [],
    catalog: {
      plans: [{ ...FALLBACK_CATALOG.plans[0]!, available: false }, { offerId: "old", priceMicros: 1, includedHours: 3 }],
      trialAvailable: true,
      trialDays: 7,
      trialPriceMicros: 1_000_000,
      trialMachineMinutes: 8_400,
      enterpriseContactUrl: "https://zuse.sh/contact",
    },
  });
  assert.equal(catalog.trialAvailable, true);
  assert.equal(catalog.plans.length, 1);
  assert.equal(catalog.plans[0]!.available, false);
  assert.equal(catalog.enterpriseContactUrl, "https://zuse.sh/contact");
  assert.equal(tierOf("legacy"), "cloud");
  assert.equal(tierOf("cloud-trial"), "cloud");
  assert.equal(tierOf("cloud-max"), "cloud-max");
});

test("plan copy reads in machine minutes with hours as a hint", () => {
  assert.equal(TRIAL_COPY, "Try Cloud for $1 for a week — 8,400 machine minutes (about 35 h on a 4 vCPU / 16 GB computer), then $20/month for 33,600.");
  assert.equal(trialCopy({ ...FALLBACK_CATALOG, trialAvailable: true }), TRIAL_COPY);
  assert.equal(planTitle(cloud), "Cloud · $20/month");
  assert.equal(planTitle({ ...cloud, priceMicros: 40_000_000 }), "Cloud · $40/month");
  assert.equal(planTitle(trial), "Cloud trial");
  assert.equal(allowanceLine(cloud), "33,600 machine minutes · about 140 h on your 4 vCPU / 16 GB machine");
  assert.equal(renewLine(cloud, "en-US"), "Renews Dec 1");
  assert.equal(meterLine(cloud), "12,480 of 33,600 machine minutes used · 21,120 left");
  assert.equal(meterHint(cloud), "about 88 h left on 4 vCPU / 16 GB");
  assert.equal(meterLine(over), "33,600 of 33,600 machine minutes used · 0 left");
  assert.equal(usedFraction(cloud), 12_480 / 33_600);
  assert.equal(usedFraction(over), 1);
  assert.equal(extraLine(cloud), null);
  assert.equal(extraLine(over), "2,880 extra machine minutes this month ($1.71) · cap $25");
  assert.equal(trialLine(trial, "en-US"), "Trial · ends Nov 26 · 3,200 of 8,400 machine minutes left");
  const [cloudOffer, maxOffer] = FALLBACK_CATALOG.plans;
  assert.equal(offerAllowance(cloudOffer!), "33,600 machine minutes · about 140 h on 4 vCPU / 16 GB");
  assert.equal(offerAllowance(maxOffer!), "84,000 machine minutes · about 350 h on 4 vCPU / 16 GB");
  assert.equal(offerMachines(maxOffer!), "Computers up to 8 vCPU / 32 GB");
});

test("holds explain the pause and offer the way out", () => {
  const hold = (plan: PlanUsage, holdReason: HoldReason) => holdNotice({ ...plan, usageHold: true, holdReason }, "en-US");
  assert.equal(holdNotice(cloud), null);
  assert.deepEqual(hold(trial, "trial-allowance-used"), { message: "Your trial minutes are used up. Cloud computers are paused.", actions: ["start-cloud"] });
  assert.deepEqual(hold(trial, "trial-unavailable"), { message: "This card or account has already used a trial.", actions: ["start-cloud"] });
  assert.deepEqual(hold(cloud, "cap-reached"), {
    message: "You reached your $25 pay-as-you-go cap. Cloud computers are paused until Dec 1.",
    actions: ["raise-cap", "upgrade-cloud-max"],
  });
  assert.deepEqual(hold({ ...cloud, planId: "cloud-max" }, "cap-reached")?.actions, ["raise-cap"]);
});

test("usage entries count machine minutes, with hours and the rate as a hint", () => {
  assert.equal(usageMinutes({ machineMinutes: 1_240 }), 1_240);
  assert.equal(usageMinutes({ awakeMs: 3_600_000, machineRate: 8 }), 480);
  assert.equal(usageMinutes({ awakeMs: 3_600_000 }), 240);
  assert.equal(usageMinutes({}), null);
  assert.equal(usageHint(1_240), "about 5.2 h on 4 vCPU / 16 GB");
  assert.equal(usageHint(1_240, 8), "about 2.6 h on 8 vCPU / 32 GB · 8 per minute");
});

test("usage groups by day, computer size and chat", () => {
  const day = (d: number, h = 0) => new Date(2026, 10, d, h).getTime();
  const items = [
    item({ entryId: "a", startedAt: day(2, 10), endedAt: day(2, 11), machineMinutes: 240 }),
    // Spans midnight evenly: half on each day.
    item({ entryId: "b", resourceId: "w2", startedAt: day(2, 23), endedAt: day(3, 1), machineMinutes: 960, machineRate: 8, vcpuCount: 8, memoryMib: 32768 }),
    item({ entryId: "c", resourceKind: "build", resourceId: "b1", startedAt: day(3, 5), endedAt: day(3, 6), machineMinutes: 100 }),
    item({ entryId: "d", startedAt: day(3, 7), endedAt: day(3, 7), machineMinutes: 0 }),
  ];
  const days = byDay(items, day(1, 9), day(3, 20), "en-US");
  assert.deepEqual(
    days.map((d) => [d.label, Math.round(d.machineMinutes)]),
    [
      ["Nov 1", 0],
      ["Nov 2", 720],
      ["Nov 3", 580],
    ],
  );
  assert.deepEqual(
    bySize(items).map((b) => [b.label, b.machineMinutes, b.rate]),
    [
      ["Large · 8 vCPU · 32 GB", 960, 8],
      ["Standard · 4 vCPU · 16 GB", 340, 4],
    ],
  );
  assert.deepEqual(
    byChat(items, (id) => (id === "w1" ? "Fix login" : null)).map((b) => [b.label, b.machineMinutes]),
    [
      ["Cloud chat", 960],
      ["Fix login", 240],
      ["Setup builds", 100],
    ],
  );
});

test("dither cells fill to the fraction and thicken toward the leading edge", () => {
  for (const [fraction, lit] of [[0, 0], [0.001, 1], [0.25, 8], [0.5, 15], [1, 30]] as const) {
    assert.equal(litColumns(fraction, 30), lit);
    for (let y = 0; y < 3; y++) for (let x = 0; x < 30; x++) assert.equal(["on", "faint"].includes(meterCell(x, y, lit)), x < lit, `${fraction} at ${x},${y}`);
  }
  // The leading column is solid.
  for (let y = 0; y < 4; y++) assert.equal(meterCell(9, y, 10), "on");
  assert.equal(filledRows(0, 10, 20), 0);
  assert.equal(filledRows(0.01, 10, 20), 1);
  assert.equal(filledRows(10, 10, 20), 20);
  for (let x = 0; x < 4; x++) assert.equal(columnCell(x, 4, 5), "on");
  assert.equal(columnCell(0, 6, 5), "empty");
});

test("errors from the server read in plain words", () => {
  assert.match(cloudErrorText("cloud_size_requires_cloud_max"), /need Cloud Max/);
  assert.match(cloudErrorText("request failed: cloud_usage_hold"), /paused/);
  assert.equal(cloudErrorText("cloud_trial_unavailable"), "This card or account has already used a trial.");
  assert.equal(cloudErrorText("Something else"), "Something else");
});

test("new computers use the server's default provider, else the first ready one", () => {
  assert.equal(defaultProvider(null), null);
  assert.equal(defaultProvider({ defaultProvider: "p2", providers: [{ id: "p1", configured: true, managedBaseReady: true }] }), "p2");
  assert.equal(
    defaultProvider({ providers: [{ id: "p1", configured: true, managedBaseReady: false }, { id: "p3", configured: true, managedBaseReady: true, preservesDisk: true, sizes: [{}] }] }),
    "p3",
  );
  assert.equal(defaultProvider({ providers: [] }), null);
});

test("billing copy never names the infrastructure", () => {
  assert.ok(namesInfrastructure("View boxd build logs"));
  assert.ok(!namesInfrastructure("Keyboard and dashboard"));
  const copy = [
    TRIAL_COPY,
    planTitle(cloud),
    allowanceLine(cloud),
    meterLine(cloud),
    meterHint(cloud),
    extraLine(over)!,
    trialLine(trial),
    usageHint(1_240, 8),
    ...FALLBACK_CATALOG.plans.flatMap((p) => [offerAllowance(p), offerMachines(p)]),
    ...(["trial-allowance-used", "trial-unavailable", "cap-reached"] as const).map((holdReason) => holdNotice({ ...cloud, usageHold: true, holdReason })!.message),
    ...["cloud_size_requires_cloud_max", "cloud_usage_hold", "cloud_trial_unavailable"].map(cloudErrorText),
  ];
  for (const text of copy) assert.ok(!namesInfrastructure(text), text);
});

test("the web app's source shows no provider names", () => {
  const root = join(import.meta.dirname, "../../..");
  const offenders: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.(tsx?|css|json|html)$/.test(name) && !/\.test\.tsx?$/.test(name)) {
        readFileSync(path, "utf8")
          .split("\n")
          .forEach((line, index) => namesInfrastructure(line) && offenders.push(`${path.slice(root.length + 1)}:${index + 1}`));
      }
    }
  };
  walk(root);
  assert.deepEqual(offenders, []);
});

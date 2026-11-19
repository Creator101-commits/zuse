// Machine minutes grouped by day, computer size and chat for the Usage view. Pure, so grouping is testable without a DOM.
import { usageMinutes } from "./plans";
import type { BillingUsageItem } from "./types";

const DAY = 86_400_000;
const MAX_DAYS = 62;

export interface Bucket {
  key: string;
  label: string;
  machineMinutes: number;
  /** Machine minutes per awake minute when every entry shares one, for the hours hint. */
  rate: number | null;
}
export interface Day {
  start: number;
  label: string;
  machineMinutes: number;
}

/** Machine minutes per awake minute: the reported rate, else one per 4 GB of memory. */
export const rateOf = (item: Pick<BillingUsageItem, "machineRate" | "memoryMib">) =>
  typeof item.machineRate === "number" && item.machineRate > 0 ? item.machineRate : Math.max(1, Math.ceil((item.memoryMib || 0) / 4096));

const midnight = (ms: number) => {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
};

/** Every day from `from` through `until`, each with the machine minutes used then. Entries spanning days split by overlap. */
export function byDay(items: readonly BillingUsageItem[], from: number, until: number, locale?: string): Day[] {
  const first = midnight(from);
  const days: Day[] = [];
  for (let start = first; start <= until && days.length < MAX_DAYS; start = midnight(start + DAY + 3_600_000)) {
    days.push({ start, label: new Date(start).toLocaleDateString(locale, { month: "short", day: "numeric" }), machineMinutes: 0 });
  }
  if (!days.length) return days;
  const end = (index: number) => days[index + 1]?.start ?? days[index]!.start + DAY;
  for (const item of items) {
    const minutes = usageMinutes(item);
    if (!minutes) continue;
    const a = item.startedAt;
    const b = Math.max(item.endedAt, a);
    const span = b - a;
    days.forEach((day, index) => {
      const overlap = Math.min(b, end(index)) - Math.max(a, day.start);
      if (span <= 0) {
        if (a >= day.start && a < end(index)) day.machineMinutes += minutes;
      } else if (overlap > 0) day.machineMinutes += (minutes * overlap) / span;
    });
  }
  return days;
}

function group(items: readonly BillingUsageItem[], keyOf: (item: BillingUsageItem) => [string, string] | null): Bucket[] {
  const buckets = new Map<string, Bucket>();
  for (const item of items) {
    const minutes = usageMinutes(item);
    const key = keyOf(item);
    if (minutes === null || !key) continue;
    const rate = rateOf(item);
    const bucket = buckets.get(key[0]);
    if (bucket) {
      bucket.machineMinutes += minutes;
      if (bucket.rate !== rate) bucket.rate = null;
    } else buckets.set(key[0], { key: key[0], label: key[1], machineMinutes: minutes, rate });
  }
  return [...buckets.values()].filter((b) => b.machineMinutes > 0).sort((a, b) => b.machineMinutes - a.machineMinutes);
}

/** The computer sizes that ran, largest use first. */
export function bySize(items: readonly BillingUsageItem[]): Bucket[] {
  return group(items, (item) => {
    const rate = rateOf(item);
    if (rate === 4) return ["4", "Standard · 4 vCPU · 16 GB"];
    if (rate === 8) return ["8", "Large · 8 vCPU · 32 GB"];
    return [String(rate), `${item.vcpuCount || rate} vCPU · ${Math.round((item.memoryMib || rate * 4096) / 1024)} GB`];
  });
}

/** Each chat's cloud computer, plus setup builds and storage as one row. */
export function byChat(items: readonly BillingUsageItem[], titleOf: (workspaceId: string) => string | null | undefined): Bucket[] {
  return group(items, (item) => (item.resourceKind === "workspace" ? [`w:${item.resourceId}`, titleOf(item.resourceId) || "Cloud chat"] : ["setup", "Setup builds"]));
}

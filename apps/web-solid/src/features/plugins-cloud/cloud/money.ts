// Exact money conversion from crates/model/src/cloud_billing.rs. Amounts are integers; never floats.
export const MAX_SAFE_MICROS = Number.MAX_SAFE_INTEGER;

export function capMicros(input: string): number | null {
  const parts = input.trim().split(".");
  if (parts.length > 2) return null;
  const whole = parts[0] ?? "";
  const fraction = parts[1] ?? "";
  if (!whole || fraction.length > 6 || !/^\d*$/.test(whole + fraction)) return null;
  const micros = BigInt(whole) * 1_000_000n + BigInt(fraction.padEnd(6, "0") || "0");
  return micros <= BigInt(MAX_SAFE_MICROS) ? Number(micros) : null;
}

export function capDollars(micros: number) {
  const whole = Math.trunc(micros / 1_000_000);
  const fraction = micros % 1_000_000;
  return fraction === 0 ? String(whole) : `${whole}.${String(fraction).padStart(6, "0")}`.replace(/0+$/, "");
}

export function usd(micros: number) {
  const value = BigInt(Math.trunc(micros));
  const abs = value < 0n ? -value : value;
  const cents = (abs + 5000n) / 10000n;
  return `${value < 0n ? "−" : ""}$${cents / 100n}.${String(cents % 100n).padStart(2, "0")}`;
}

export function cents(value: number) {
  const amount = Math.abs(Math.trunc(value));
  return `${value < 0 ? "-" : ""}$${Math.floor(amount / 100)}.${String(amount % 100).padStart(2, "0")}`;
}

/** Whole dollars without cents ("$20"), otherwise to the cent ("$1.71"). Plan prices and caps read this way. */
export function dollars(micros: number) {
  return micros % 1_000_000 === 0 ? usd(micros).replace(/\.00$/, "") : usd(micros);
}

const COUNT = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
/** Whole machine minutes with thousands separators ("33,600"). */
export const count = (value: number) => COUNT.format(Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0);

/** Awake minutes as hours: whole hours from 10 h, one decimal below, "<0.1 h" for a sliver. */
export function hours(minutes: number) {
  if (!Number.isFinite(minutes) || minutes <= 0) return "0 h";
  const tenths = Math.round(minutes / 6);
  if (tenths === 0) return "<0.1 h";
  if (tenths >= 100) return `${Math.round(minutes / 60)} h`;
  return `${(tenths / 10).toFixed(1)} h`;
}

// Offer ids from crates/protocol/src/cloud_billing.rs `plans`.
export const CLOUD_OFFER = "cloud-starter-v1";
export const CLOUD_MAX_OFFER = "cloud-pro-v1";
/** Cloud with the first week for $1; the subscription itself is Cloud. */
export const CLOUD_TRIAL_OFFER = "cloud-starter-trial-v1";
/** The original $40 plan; existing subscribers get Cloud's allowance. */
export const LEGACY_OFFER = "cloud-workspace-standard-v1";
/** Every offer whose subscription grants cloud computers. */
export const CLOUD_OFFERS: readonly string[] = [CLOUD_OFFER, CLOUD_MAX_OFFER, LEGACY_OFFER];
export const CREDIT_AMOUNTS = [1000, 2500, 5000, 10000] as const;

interface Entitlement {
  offerId?: string;
  status?: string;
  paidThrough?: number | null;
}
/** Presentation only; checkout reconciles canonical provider state. */
export function cloudSubscribed(value: unknown, now: number): boolean | null {
  const list = (value as { entitlements?: unknown } | null)?.entitlements;
  if (!Array.isArray(list)) return null;
  return (list as Entitlement[]).some(
    (ent) =>
      CLOUD_OFFERS.includes(ent.offerId ?? "") && ["pending", "active", "grace"].includes(ent.status ?? "") && (typeof ent.paidThrough !== "number" || ent.paidThrough > now),
  );
}

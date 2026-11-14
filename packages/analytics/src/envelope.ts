import { isAnalyticsEventName, sanitizeAnalyticsProperties } from "./events";

/** Scrub SDK-added URLs, referrers, DOM metadata, and profile updates too. */
export function sanitizeAnalyticsEnvelope<T extends { event: string; properties?: Record<string, unknown> }>(event: T | null): T | null {
  if (!event || !isAnalyticsEventName(event.event)) return null;
  const raw = event.properties ?? {};
  event.properties = sanitizeAnalyticsProperties(event.event, raw);
  if (typeof raw.token === "string") event.properties.token = raw.token;
  if (typeof raw.distinct_id === "string" && /^(anonymous_[a-f0-9-]{36}|account_[a-f0-9]{64})$/.test(raw.distinct_id)) {
    event.properties.distinct_id = raw.distinct_id;
  } else return null;
  event.properties.$process_person_profile = false;
  return event;
}

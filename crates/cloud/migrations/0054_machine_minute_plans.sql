-- Cloud plans sold in machine minutes (one minute of a 1 vCPU / 4 GB
-- machine). Periods record which plan they grant; existing periods keep their
-- original terms (plan_id stays NULL) until their subscription renews.
--
-- usage_hold pauses an account's cloud computers when its allowance and
-- pay-as-you-go cap are used up, or a trial allowance is used up. It is
-- recomputed whenever usage settles or the cap changes.
ALTER TABLE api_cloud_billing_periods ADD COLUMN IF NOT EXISTS plan_id text;
ALTER TABLE api_cloud_billing_periods ADD COLUMN IF NOT EXISTS usage_hold boolean NOT NULL DEFAULT false;
ALTER TABLE api_cloud_billing_periods ADD COLUMN IF NOT EXISTS hold_reason text;

-- One first-week trial per account and per card. A trial subscription whose
-- card or account already had one is recorded as rejected and grants no
-- allowance until the person starts the plan.
CREATE TABLE IF NOT EXISTS api_cloud_trials (
    subscription_id text PRIMARY KEY,
    account_id text NOT NULL,
    card_fingerprint text,
    state text NOT NULL CHECK (state IN ('active', 'rejected', 'converted', 'ended')),
    started_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS api_cloud_trials_account_idx
  ON api_cloud_trials (account_id) WHERE state <> 'rejected';
CREATE UNIQUE INDEX IF NOT EXISTS api_cloud_trials_card_idx
  ON api_cloud_trials (card_fingerprint) WHERE card_fingerprint IS NOT NULL AND state <> 'rejected';

-- Wake the runtime worker when a usage hold turns on, so online computers are
-- paused right away (same latency hint as 0047's shared-credit hold).
DROP TRIGGER IF EXISTS zuse_cloud_usage_hold_wake ON api_cloud_billing_periods;
CREATE TRIGGER zuse_cloud_usage_hold_wake
AFTER UPDATE OF usage_hold ON api_cloud_billing_periods
FOR EACH ROW WHEN (NEW.usage_hold AND NOT COALESCE(OLD.usage_hold, false))
EXECUTE FUNCTION zuse_cloud_notify_work('idle_sleep');

DO $grants$
DECLARE schema_name text := current_schema(); app_role text;
BEGIN
 app_role := CASE schema_name WHEN 'zuse_staging' THEN 'zuse_cloud_staging' WHEN 'zuse_production' THEN 'zuse_cloud_production' END;
 IF app_role IS NOT NULL AND EXISTS(SELECT 1 FROM pg_roles WHERE rolname=app_role) THEN
  EXECUTE format('GRANT SELECT,INSERT,UPDATE,DELETE ON TABLE %I.api_cloud_trials TO %I',schema_name,app_role);
 END IF;
END $grants$;

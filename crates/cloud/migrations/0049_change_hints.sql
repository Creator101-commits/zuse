-- Change hints for views that used to poll: machine image builds and billing.
--
-- Like every other account event these carry only a kind and a resource ID
-- (payload '{}'); clients refetch through the authorized HTTP reads. The API
-- only forwards these kinds to sockets whose ticket asked for them, so older
-- clients that reject unknown kinds never see them.
--
-- A burst inside one transaction (a settlement batch inserting many usage
-- rows, several build steps) collapses to one outbox row per owner and kind.
-- Only rows this transaction wrote are reused: skipping because another
-- transaction's row is pending could deliver that hint, and the client's
-- refetch, before this transaction commits.
ALTER TABLE zuse_rust_account_event_outbox
 DROP CONSTRAINT zuse_rust_account_event_outbox_kind_check;
ALTER TABLE zuse_rust_account_event_outbox
 ADD CONSTRAINT zuse_rust_account_event_outbox_kind_check
 CHECK(kind IN ('workspace','conversation','connection','authentication','github','environment','task-command-queue','task-command-progress','machine-image','billing'));

CREATE INDEX IF NOT EXISTS zuse_rust_account_event_outbox_hints
 ON zuse_rust_account_event_outbox(owner_id, kind) WHERE kind IN ('machine-image','billing');

-- TG_ARGV: kind, owner column, resource column.
CREATE OR REPLACE FUNCTION zuse_rust_change_hint_outbox() RETURNS trigger
LANGUAGE plpgsql SET search_path FROM CURRENT AS $body$
DECLARE item jsonb; owner text; resource text;
BEGIN
    IF TG_OP='DELETE' THEN item:=to_jsonb(OLD); ELSE item:=to_jsonb(NEW); END IF;
    owner:=item->>TG_ARGV[1];
    resource:=left(item->>TG_ARGV[2],512);
    IF owner IS NULL OR owner='' OR resource IS NULL OR resource='' THEN RETURN NULL; END IF;
    INSERT INTO zuse_rust_account_event_outbox(owner_id,kind,resource_id,payload,created_at)
    SELECT owner,TG_ARGV[0],resource,'{}'::jsonb,(extract(epoch FROM clock_timestamp())*1000)::bigint
    WHERE NOT EXISTS(SELECT 1 FROM zuse_rust_account_event_outbox o WHERE o.owner_id=owner AND o.kind=TG_ARGV[0]
        AND o.xmin::text=(txid_current()%4294967296)::text);
    RETURN NULL;
END $body$;

-- Machine image builds (Settings → Machine): state, phase or failure changes.
-- Lease renewals and log tail writes stay silent.
CREATE TRIGGER zuse_rust_machine_image_hints
AFTER INSERT OR DELETE ON cloud_machine_images
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','owner_id','image_id');
CREATE TRIGGER zuse_rust_machine_image_update_hints
AFTER UPDATE ON cloud_machine_images
FOR EACH ROW WHEN ((NEW.state,NEW.phase,NEW.error_code,NEW.artifact_id) IS DISTINCT FROM (OLD.state,OLD.phase,OLD.error_code,OLD.artifact_id))
EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','owner_id','image_id');
-- A saved recipe changes what the machine view shows on every device.
CREATE TRIGGER zuse_rust_machine_version_hints
AFTER INSERT ON cloud_machine_versions
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','owner_id','owner_id');

-- Account image builds and their stored snapshots (Settings → Repositories).
CREATE TRIGGER zuse_rust_project_build_hints
AFTER INSERT ON api_cloud_project_builds
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','account_id','build_id');
CREATE TRIGGER zuse_rust_project_build_update_hints
AFTER UPDATE ON api_cloud_project_builds
FOR EACH ROW WHEN ((NEW.state,NEW.last_error_code,NEW.snapshot_id,NEW.included,NEW.settings->'rustImagePhase')
    IS DISTINCT FROM (OLD.state,OLD.last_error_code,OLD.snapshot_id,OLD.included,OLD.settings->'rustImagePhase'))
EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','account_id','build_id');
CREATE TRIGGER zuse_rust_snapshot_hints
AFTER INSERT ON api_cloud_snapshots
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','account_id','snapshot_id');
CREATE TRIGGER zuse_rust_snapshot_update_hints
AFTER UPDATE ON api_cloud_snapshots
FOR EACH ROW WHEN (NEW.state IS DISTINCT FROM OLD.state)
EXECUTE FUNCTION zuse_rust_change_hint_outbox('machine-image','account_id','snapshot_id');

-- Billing (Settings → Usage and billing): plan entitlements, billing period
-- status/cap/dates, confirmed usage and ledger entries. Short-lived compute
-- reservations are excluded; they move on every metering tick.
CREATE TRIGGER zuse_rust_entitlement_hints
AFTER INSERT ON api_entitlements
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','entitlement_id');
CREATE TRIGGER zuse_rust_entitlement_update_hints
AFTER UPDATE ON api_entitlements
FOR EACH ROW WHEN ((NEW.status,NEW.offer_id,NEW.paid_through,NEW.ended_at,NEW.credit_balance,NEW.period_start)
    IS DISTINCT FROM (OLD.status,OLD.offer_id,OLD.paid_through,OLD.ended_at,OLD.credit_balance,OLD.period_start))
EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','entitlement_id');
CREATE TRIGGER zuse_rust_billing_period_hints
AFTER INSERT ON api_cloud_billing_periods
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','period_id');
CREATE TRIGGER zuse_rust_billing_period_update_hints
AFTER UPDATE ON api_cloud_billing_periods
FOR EACH ROW WHEN ((NEW.status,NEW.period_start,NEW.period_end,NEW.overage_cap_micros,NEW.base_price_micros,NEW.included_provider_cost_micros)
    IS DISTINCT FROM (OLD.status,OLD.period_start,OLD.period_end,OLD.overage_cap_micros,OLD.base_price_micros,OLD.included_provider_cost_micros))
EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','period_id');
CREATE TRIGGER zuse_rust_billing_usage_hints
AFTER INSERT ON api_cloud_billing_usage
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','period_id');
CREATE TRIGGER zuse_rust_billing_usage_update_hints
AFTER UPDATE ON api_cloud_billing_usage
FOR EACH ROW WHEN ((NEW.status,NEW.provider_cost_micros) IS DISTINCT FROM (OLD.status,OLD.provider_cost_micros))
EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','period_id');
CREATE TRIGGER zuse_rust_billing_ledger_hints
AFTER INSERT ON api_cloud_billing_ledger
FOR EACH ROW EXECUTE FUNCTION zuse_rust_change_hint_outbox('billing','account_id','period_id');

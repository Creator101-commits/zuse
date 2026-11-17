-- Wake the runtime worker when an account's shared-credit hold turns on, so
-- its online computers are paused right away instead of on the next idle
-- scan. Optional: installed with 0032_brokeboy.sql and 0033_shared_credits.sql
-- (which adds the column) by `database.py apply-brokeboy`. Requires
-- zuse_cloud_notify_work() from 0043_work_wakeups.sql.
--
-- Only the false -> true transition notifies; the hold is recomputed whenever
-- its inputs change and unchanged values are not rewritten. NOTIFY is a
-- latency hint: the runtime worker's idle scan still pauses held accounts.
CREATE TRIGGER zuse_cloud_billing_hold_wake
AFTER UPDATE OF shared_credit_hold ON api_cloud_billing_periods
FOR EACH ROW WHEN (NEW.shared_credit_hold AND NOT COALESCE(OLD.shared_credit_hold, false))
EXECUTE FUNCTION zuse_cloud_notify_work('idle_sleep');

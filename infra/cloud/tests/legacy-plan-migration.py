#!/usr/bin/env python3
"""Planning tests for the legacy plan migration. Never calls Stripe."""
import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "legacy-plan-migration.py"
spec = importlib.util.spec_from_file_location("legacy_plan_migration", SCRIPT)
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


def sub(id, price, status="active"):
    return {"id": id, "status": status, "customer": "cus_" + id, "metadata": {"account_id": "user-" + id},
            "latest_invoice": "in_" + id,
            "items": {"data": [{"id": "si_" + id, "price": {"id": price}, "current_period_start": 100, "current_period_end": 200}]}}


class LegacyPlanMigrationTest(unittest.TestCase):
    def test_only_active_legacy_subscriptions_are_switched(self):
        actions = migration.plan_actions(
            [sub("a", "price_old"), sub("b", "price_new"), sub("c", "price_old", "canceled"), sub("d", "price_old", "past_due")],
            "price_old", "price_new")
        self.assertEqual([a["subscription"] for a in actions], ["a", "d"])
        self.assertEqual(actions[0]["item"], "si_a")

    def test_switch_has_no_proration_and_sets_the_offer(self):
        action = migration.plan_actions([sub("a", "price_old")], "price_old", "price_new")[0]
        form = dict(migration.switch_form(action, "price_new"))
        self.assertEqual(form["proration_behavior"], "none")
        self.assertEqual(form["items[0][price]"], "price_new")
        self.assertEqual(form["metadata[offer_id]"], "cloud-starter-v1")
        refund = dict(migration.refund_form("pi_1", action))
        self.assertEqual(refund["amount"], "2000")
        self.assertEqual(migration.idempotency("refund", action), "zuse-legacy-refund:a:100")

    def test_apply_requires_owner_approval(self):
        with self.assertRaises(SystemExit):
            migration.main(["--legacy-price", "p", "--cloud-price", "q", "--apply"])


if __name__ == "__main__":
    unittest.main()

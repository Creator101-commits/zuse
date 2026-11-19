#!/usr/bin/env python3
"""Move subscriptions on the original $40 cloud price to the $20 Cloud price.

For every active subscription on the legacy price this:
  1. switches its subscription item to the Cloud price with no proration, so
     the current period is not re-invoiced and the next invoice is $20;
  2. sets subscription metadata offer_id=cloud-starter-v1 (the API checks the
     price and this metadata agree);
  3. refunds $20 of the current period's paid invoice (the difference).

Dry run by default: it only reads Stripe and prints the plan. Writing needs
--apply and --owner-approved, and is meant to run once, with the owner's
explicit approval. Every write carries a stable Idempotency-Key, so a rerun
never switches or refunds twice; already-migrated subscriptions are skipped.

Environment: STRIPE_SECRET_KEY (sk_test_... for rehearsal).
Usage:
  legacy-plan-migration.py --legacy-price price_old --cloud-price price_new
  legacy-plan-migration.py ... --apply --owner-approved
"""
import argparse
import json
import os
import sys
import urllib.parse
import urllib.request

API = "https://api.stripe.com/v1/"
STRIPE_VERSION = "2026-09-30.endive"
CLOUD_OFFER = "cloud-starter-v1"
REFUND_CENTS = 2000


def plan_actions(subscriptions, legacy_price, cloud_price):
    """Pure planning: which subscriptions to switch, and their item ids."""
    actions = []
    for sub in subscriptions:
        if sub.get("status") not in ("active", "past_due"):
            continue
        items = sub.get("items", {}).get("data", [])
        legacy = [i for i in items if i.get("price", {}).get("id") == legacy_price]
        if not legacy:
            continue
        if any(i.get("price", {}).get("id") == cloud_price for i in items):
            continue
        item = legacy[0]
        actions.append({
            "subscription": sub["id"],
            "customer": sub.get("customer") if isinstance(sub.get("customer"), str) else sub.get("customer", {}).get("id"),
            "account": sub.get("metadata", {}).get("account_id"),
            "item": item["id"],
            "period_start": item.get("current_period_start"),
            "period_end": item.get("current_period_end"),
            "latest_invoice": sub.get("latest_invoice") if isinstance(sub.get("latest_invoice"), str) else (sub.get("latest_invoice") or {}).get("id"),
        })
    return actions


def switch_form(action, cloud_price):
    return [
        ("items[0][id]", action["item"]),
        ("items[0][price]", cloud_price),
        ("proration_behavior", "none"),
        ("metadata[offer_id]", CLOUD_OFFER),
        ("metadata[zuse_migration]", "legacy-40-to-cloud-20"),
    ]


def refund_form(payment_intent, action):
    return [
        ("payment_intent", payment_intent),
        ("amount", str(REFUND_CENTS)),
        ("reason", "requested_by_customer"),
        ("metadata[zuse_migration]", "legacy-40-to-cloud-20"),
        ("metadata[subscription]", action["subscription"]),
    ]


def idempotency(kind, action):
    return f"zuse-legacy-{kind}:{action['subscription']}:{action['period_start']}"


class Stripe:
    def __init__(self, key):
        if not key.startswith(("sk_", "rk_")):
            raise SystemExit("STRIPE_SECRET_KEY is required")
        self.key = key

    def call(self, method, path, form=None, query=None, key=None):
        url = API + path
        if query:
            url += "?" + urllib.parse.urlencode(query)
        data = urllib.parse.urlencode(form).encode() if form is not None else None
        request = urllib.request.Request(url, data=data, method=method)
        request.add_header("Authorization", f"Bearer {self.key}")
        request.add_header("Stripe-Version", STRIPE_VERSION)
        if key:
            request.add_header("Idempotency-Key", key)
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    def subscriptions(self, price):
        cursor = None
        while True:
            query = [("price", price), ("status", "all"), ("limit", "100")]
            if cursor:
                query.append(("starting_after", cursor))
            page = self.call("GET", "subscriptions", query=query)
            yield from page["data"]
            if not page.get("has_more"):
                return
            cursor = page["data"][-1]["id"]

    def payment_intent(self, invoice):
        payments = self.call("GET", "invoice_payments", query=[("invoice", invoice), ("status", "paid")])
        for payment in payments.get("data", []):
            intent = payment.get("payment", {}).get("payment_intent")
            if intent:
                return intent
        return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--legacy-price", required=True)
    parser.add_argument("--cloud-price", required=True)
    parser.add_argument("--apply", action="store_true", help="write to Stripe (default: dry run)")
    parser.add_argument("--owner-approved", action="store_true", help="required with --apply")
    args = parser.parse_args(argv)
    if args.apply and not args.owner_approved:
        raise SystemExit("--apply needs --owner-approved: run only with the owner's explicit approval")
    stripe = Stripe(os.environ.get("STRIPE_SECRET_KEY", ""))
    actions = plan_actions(list(stripe.subscriptions(args.legacy_price)), args.legacy_price, args.cloud_price)
    for action in actions:
        intent = stripe.payment_intent(action["latest_invoice"]) if action["latest_invoice"] else None
        line = f"{action['subscription']} account={action['account']} switch {action['item']} -> {args.cloud_price}; refund $20 of {intent or 'NO PAID INVOICE'}"
        if not args.apply:
            print("DRY RUN", line)
            continue
        stripe.call("POST", f"subscriptions/{action['subscription']}", switch_form(action, args.cloud_price), key=idempotency("switch", action))
        if intent:
            stripe.call("POST", "refunds", refund_form(intent, action), key=idempotency("refund", action))
        print("DONE", line)
    print(f"{len(actions)} subscription(s) {'migrated' if args.apply else 'would be migrated'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

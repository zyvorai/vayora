import hashlib, hmac, io, json, unittest, urllib.error
from zyvor import Zyvor, ZyvorError, verify_webhook


class FakeRes(io.BytesIO):
    def __enter__(self): return self
    def __exit__(self, *a): return False


class Tests(unittest.TestCase):
    def test_headers_and_minor_units(self):
        seen = {}
        def opener(req, timeout=None):
            seen.update(url=req.full_url, headers={k.lower(): v for k, v in req.header_items()}, body=json.loads(req.data))
            return FakeRes(b'{"id":"pi_1"}')
        z = Zyvor("zk_test_x", actor="priya", opener=opener)
        self.assertEqual(z.create_intent(49900, external_ref="FEE-1")["id"], "pi_1")
        self.assertEqual(seen["headers"]["authorization"], "Bearer zk_test_x")
        self.assertEqual(seen["headers"]["zyvor-actor"], "priya")
        self.assertTrue(seen["headers"]["idempotency-key"])
        self.assertEqual(seen["body"], {"amount_minor": 49900, "currency": "INR", "external_ref": "FEE-1"})

    def test_rejects_bad_amounts(self):
        z = Zyvor("k", opener=lambda *a, **k: self.fail("no request expected"))
        for bad in (10.5, 0, -1, True, "5"):
            with self.assertRaises(ValueError): z.create_intent(bad)

    def test_errors(self):
        def opener(req, timeout=None):
            raise urllib.error.HTTPError(req.full_url, 409, "Conflict", {}, io.BytesIO(b'{"error":{"code":"payment_outcome_unresolved","message":"resolve first"}}'))
        with self.assertRaises(ZyvorError) as c: Zyvor("k", opener=opener).create_attempt("pi_1", "upi")
        self.assertEqual((c.exception.status, c.exception.code), (409, "payment_outcome_unresolved"))

    def test_billing_and_collections_paths(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        z.subscribe("cus_1", "plan_1", start_date="2026-11-01"); z.change_plan("sub_1", "plan_2")
        z.record_usage("sub_1", 5, reference="u1"); z.credit_note("inv_1", 100, "goodwill")
        z.ageing("2026-10-31"); z.promise_to_pay("inv_1", 500, "2026-11-05"); z.remind("inv_1", "sms")
        self.assertEqual([(m, p) for m, p, _ in seen], [
            ("POST", "/v1/subscriptions"), ("POST", "/v1/subscriptions/sub_1/change_plan"), ("POST", "/v1/subscriptions/sub_1/usage"),
            ("POST", "/v1/invoices/inv_1/credit_notes"), ("GET", "/v1/collections/ageing?as_of=2026-10-31"),
            ("POST", "/v1/invoices/inv_1/promises"), ("POST", "/v1/invoices/inv_1/reminders")])
        self.assertEqual(seen[2][2], {"quantity": 5, "reference": "u1"})

    def test_fx_paths(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        z.set_fx_rate("83.2150", as_of="2026-10-01"); z.fx_rates(); z.fx_convert(100, "USD", "INR"); z.fx_exposure("2026-10-31"); z.fx_exposure(); z.einvoice_draft("inv_1"); z.update_customer("cus_1", pincode="110001")
        self.assertEqual(seen, [
            ("POST", "/v1/fx/rates", {"currency": "USD", "rate": "83.2150", "as_of": "2026-10-01"}),
            ("GET", "/v1/fx/rates", None), ("GET", "/v1/fx/convert?amount_minor=100&from=USD&to=INR", None),
            ("GET", "/v1/fx/exposure?as_of=2026-10-31", None), ("GET", "/v1/fx/exposure", None), ("GET", "/v1/invoices/inv_1/einvoice", None), ("PATCH", "/v1/customers/cus_1", {"pincode": "110001"})])

    def test_links_dunning_and_rotation_paths(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        z.create_payment_link(5000, description="Term fee", max_uses=3); z.list_payment_links(); z.cancel_payment_link("plk_1")
        z.run_dunning(channel="sms"); z.rotate_key("key_1", 2)
        self.assertEqual([x[:2] for x in seen], [("POST", "/v1/payment_links"), ("GET", "/v1/payment_links"), ("POST", "/v1/payment_links/plk_1/cancel"),
            ("POST", "/v1/collections/dunning/run"), ("POST", "/v1/api_keys/key_1/rotate")])
        self.assertEqual(seen[0][2], {"amount_minor": 5000, "description": "Term fee", "max_uses": 3})
        self.assertEqual(seen[3][2], {"channel": "sms"})
        self.assertEqual(seen[4][2], {"overlap_hours": 2})

    def test_reminder_drafts(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        z.draft_reminder("inv_1", ai=True); z.reminder_drafts("pending"); z.approve_draft("rdf_1", "Edited"); z.reject_draft("rdf_2")
        self.assertEqual([x[:2] for x in seen], [("POST", "/v1/invoices/inv_1/reminders/draft"), ("GET", "/v1/collections/drafts?status=pending"),
            ("POST", "/v1/collections/drafts/rdf_1/approve"), ("POST", "/v1/collections/drafts/rdf_2/reject")])
        self.assertEqual(seen[0][2], {"ai": True})
        self.assertEqual(seen[2][2], {"message": "Edited"})

    def test_merchant_operations_paths(self):
        seen = []

        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")

        z = Zyvor("k", opener=opener)
        z.payment_timeline("pi_1"); z.onboarding(); z.save_profile("Sri Clinic", "llp", "ABCDE1234F", "12 MG Road")
        z.add_bank_account("Sri Clinic", "HDFC0001234", "123456789012"); z.review_onboarding("reject", "blurry PAN")
        z.create_counter("br_1", "Till 1"); z.branch_summary("br_1", "2026-10-01"); z.add_cash("br_1", "sale", 500)
        z.close_day("br_1", "2026-10-30", 500); z.sign_off_closing("clo_1"); z.cashflow(2)
        paths = [(m, p) for m, p, _ in seen]
        self.assertEqual(paths, [
            ("GET", "/v1/payment_intents/pi_1/timeline"), ("GET", "/v1/onboarding"), ("PUT", "/v1/onboarding"),
            ("POST", "/v1/onboarding/bank_accounts"), ("POST", "/v1/onboarding/review"), ("POST", "/v1/counters"),
            ("GET", "/v1/branches/br_1/summary?from=2026-10-01"), ("POST", "/v1/cash_entries"), ("POST", "/v1/closings"),
            ("POST", "/v1/closings/clo_1/signoff"), ("GET", "/v1/insights/cashflow?weeks=2"),
        ])
        self.assertEqual(seen[4][2], {"decision": "reject", "note": "blurry PAN"})

    def test_intent_can_carry_branch_and_counter(self):
        seen = []

        def opener(req, timeout=None):
            seen.append(json.loads(req.data))
            return FakeRes(b"{}")

        Zyvor("k", opener=opener).create_intent(500, branch_id="br_1", counter_id="ctr_1")
        self.assertEqual((seen[0]["branch_id"], seen[0]["counter_id"], seen[0]["amount_minor"]), ("br_1", "ctr_1", 500))

    def test_list_payments_sends_the_search_and_filters(self):
        seen = []

        def opener(req, timeout=None):
            seen.append(req.full_url.split("8080")[1])
            return FakeRes(b"{}")

        z = Zyvor("k", opener=opener)
        z.list_payments(); z.list_payments(status="succeeded", q="ORD 1&x", limit=10)
        self.assertEqual(seen, ["/v1/payment_intents", "/v1/payment_intents?status=succeeded&q=ORD+1%26x&limit=10"])

    def test_mandates_use_the_right_routes_and_keys(self):
        seen = []

        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None, req.get_header("Idempotency-key")))
            return FakeRes(b"{}")

        z = Zyvor("k", opener=opener)
        z.create_mandate("cus_1", 500000, "monthly", "2027-10-31", idempotency_key="m1")
        z.list_mandates(status="active"); z.mandate_action("mnd_1", "pause")
        z.schedule_debit("mnd_1", 125000, "2026-11-30", idempotency_key="d1"); z.list_mandate_debits("mnd_1")
        self.assertEqual([(m, p, k) for m, p, _, k in seen], [
            ("POST", "/v1/mandates", "m1"), ("GET", "/v1/mandates?status=active", None), ("POST", "/v1/mandates/mnd_1/pause", None),
            ("POST", "/v1/mandates/mnd_1/debits", "d1"), ("GET", "/v1/mandates/mnd_1/debits", None)])
        self.assertEqual(seen[0][2], {"customer_id": "cus_1", "max_amount_minor": 500000, "frequency": "monthly", "end_date": "2027-10-31"})
        with self.assertRaises(ValueError):
            z.create_mandate("c", 0, "daily", "2027-01-01")
        with self.assertRaises(ValueError):
            z.mandate_action("mnd_1", "delete")

    def test_simulated_settlement_routes(self):
        seen = []

        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data)))
            return FakeRes(b"{}")

        z = Zyvor("k", opener=opener)
        z.sync_settlements("sim", "2026-11-01"); z.simulate_bank_credit("stl_1", 5000); z.send_test_event()
        self.assertEqual(seen, [("POST", "/v1/settlements/sync", {"provider": "sim", "date": "2026-11-01"}),
                                ("POST", "/v1/settlements/stl_1/simulate_bank", {"short_minor": 5000}),
                                ("POST", "/v1/outbox/test", {})])
        with self.assertRaises(ValueError):
            z.simulate_bank_credit("stl_1", -1)

    def test_webhook(self):
        body = b'{"type":"payment.captured"}'
        sig = lambda t, s=b"whsec": f"t={t},v1=" + hmac.new(s, f"{t}.".encode() + body, hashlib.sha256).hexdigest()
        self.assertTrue(verify_webhook("whsec", sig(1000), body, now=1100))
        self.assertFalse(verify_webhook("whsec", sig(1000), body + b" ", now=1100))
        self.assertFalse(verify_webhook("other", sig(1000), body, now=1100))
        self.assertFalse(verify_webhook("whsec", sig(1000), body, now=9000))
        self.assertFalse(verify_webhook("whsec", "junk", body))


if __name__ == "__main__":
    unittest.main()

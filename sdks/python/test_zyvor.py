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

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

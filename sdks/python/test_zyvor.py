import hashlib, hmac, io, json, unittest, urllib.error
from zyvor import Zyvor, ZyvorError, verify_webhook


class FakeRes(io.BytesIO):
    def __enter__(self): return self
    def __exit__(self, *a): return False


class Tests(unittest.TestCase):
    def test_methods_are_annotated_with_generated_response_types(self):
        from zyvor import types
        self.assertEqual(Zyvor.list_plans.__annotations__["return"], "types.GetPlansResponse")
        self.assertIn("data", types.GetPlansResponse.__annotations__)
        # a key that is a Python keyword still works, because the types use the functional TypedDict form
        self.assertTrue(any("from" in t.__annotations__ for t in vars(types).values() if hasattr(t, "__annotations__")))

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

    def test_runs_can_name_a_test_day(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        z.run_billing(); z.run_billing(as_of="2027-01-15"); z.run_dunning(as_of="2027-01-15")
        self.assertEqual(seen, [("/v1/billing/run", {}), ("/v1/billing/run", {"as_of": "2027-01-15"}),
                                ("/v1/collections/dunning/run", {"as_of": "2027-01-15"})])

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

    def test_profile_change_request(self):
        seen = []

        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data)))
            return FakeRes(b"{}")

        Zyvor("k", opener=opener).request_profile_change("Sri Clinic Pvt Ltd", "private_limited", "ABCPE1234F", "Pune 411001", "converted to a company")
        self.assertEqual(seen[0][:2], ("POST", "/v1/onboarding/profile_change"))
        self.assertEqual(seen[0][2]["reason"], "converted to a company")
        self.assertEqual(seen[0][2]["legal_name"], "Sri Clinic Pvt Ltd")

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
        z.sync_settlements("sim", "2026-11-01"); z.simulate_bank_credit("stl_1", 5000); z.send_test_event(); z.simulate_payment_event("att_1", "failed")
        self.assertEqual(seen, [("POST", "/v1/settlements/sync", {"provider": "sim", "date": "2026-11-01"}),
                                ("POST", "/v1/settlements/stl_1/simulate_bank", {"short_minor": 5000}),
                                ("POST", "/v1/outbox/test", {}),
                                ("POST", "/v1/payment_attempts/att_1/simulate_event", {"outcome": "failed"})])
        with self.assertRaises(ValueError):
            z.simulate_payment_event("att_1", "refunded")
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

    def test_every_endpoint_method_sends_the_right_verb_path_and_body(self):
        seen = []
        def opener(req, timeout=None):
            seen.append((req.get_method(), req.full_url.split("8080")[1], json.loads(req.data) if req.data else None))
            return FakeRes(b"{}")
        z = Zyvor("k", opener=opener)
        rows = [
            ("z.get_attempt('att_1')", 'GET', '/v1/payment_attempts/att_1', None),
            ("z.refresh_attempt('att_1')", 'POST', '/v1/payment_attempts/att_1/refresh', {}),
            ("z.cancel_attempt('att_1')", 'POST', '/v1/payment_attempts/att_1/cancel', {}),
            ("z.capture_attempt('att_1')", 'POST', '/v1/payment_attempts/att_1/capture', {}),
            ("z.list_refunds(status='x1', limit=7)", 'GET', '/v1/refunds?limit=7&status=x1', None),
            ("z.approve_refund('ref_1')", 'POST', '/v1/refunds/ref_1/approve', {}),
            ("z.reject_refund('ref_1')", 'POST', '/v1/refunds/ref_1/reject', {}),
            ("z.retry_refund('ref_1')", 'POST', '/v1/refunds/ref_1/retry', {}),
            ("z.refresh_refund('ref_1')", 'POST', '/v1/refunds/ref_1/refresh', {}),
            ('z.list_settlements()', 'GET', '/v1/settlements', None),
            ("z.list_cases(status='x1', kind='x1')", 'GET', '/v1/recon/cases?kind=x1&status=x1', None),
            ("z.case_suggestions('cas_1')", 'GET', '/v1/recon/cases/cas_1/suggestions', None),
            ("z.assign_case('cas_1', 'x1')", 'POST', '/v1/recon/cases/cas_1/assign', {'assignee': 'x1'}),
            ("z.resolve_case('cas_1', 'x1')", 'POST', '/v1/recon/cases/cas_1/resolve', {'resolution': 'x1'}),
            ('z.list_plans()', 'GET', '/v1/plans', None),
            ('z.list_subscriptions()', 'GET', '/v1/subscriptions', None),
            ("z.pause_subscription('sub_1')", 'POST', '/v1/subscriptions/sub_1/pause', {}),
            ("z.resume_subscription('sub_1')", 'POST', '/v1/subscriptions/sub_1/resume', {}),
            ("z.cancel_subscription('sub_1')", 'POST', '/v1/subscriptions/sub_1/cancel', {}),
            ("z.list_invoices(status='x1')", 'GET', '/v1/invoices?status=x1', None),
            ("z.get_invoice('inv_1')", 'GET', '/v1/invoices/inv_1', None),
            ("z.create_invoice('x1', amount_minor=7, customer_id='x1', due_date='x1', tax_minor=7, external_ref='x1', lines=['a'], place_of_supply='x1', buyer_gstin='x1', reverse_charge=True)", 'POST', '/v1/invoices', {'number': 'x1', 'amount_minor': 7, 'customer_id': 'x1', 'due_date': 'x1', 'tax_minor': 7, 'external_ref': 'x1', 'lines': ['a'], 'place_of_supply': 'x1', 'buyer_gstin': 'x1', 'reverse_charge': True}),
            ("z.void_invoice('inv_1')", 'POST', '/v1/invoices/inv_1/void', {}),
            ("z.invoice_lines('inv_1')", 'GET', '/v1/invoices/inv_1/lines', None),
            ("z.list_reminders('inv_1')", 'GET', '/v1/invoices/inv_1/reminders', None),
            ("z.list_promises(status='x1')", 'GET', '/v1/collections/promises?status=x1', None),
            ("z.cancel_promise('pro_1')", 'POST', '/v1/collections/promises/pro_1/cancel', {}),
            ("z.create_vendor('x1', account_number='x1', ifsc='x1', vpa='x1', external_ref='x1')", 'POST', '/v1/vendors', {'name': 'x1', 'account_number': 'x1', 'ifsc': 'x1', 'vpa': 'x1', 'external_ref': 'x1'}),
            ('z.list_vendors()', 'GET', '/v1/vendors', None),
            ("z.vendor_details('ven_1')", 'GET', '/v1/vendors/ven_1/details', None),
            ("z.vendor_balance('ven_1')", 'GET', '/v1/vendors/ven_1/balance', None),
            ("z.create_payout('x1', 7, 'x1', currency='x1', external_ref='x1', from_balance=True)", 'POST', '/v1/payouts', {'vendor_id': 'x1', 'amount_minor': 7, 'purpose': 'x1', 'currency': 'x1', 'external_ref': 'x1', 'from_balance': True}),
            ("z.create_payouts_bulk(['a'])", 'POST', '/v1/payouts/bulk', {'payouts': ['a']}),
            ("z.list_payouts(status='x1')", 'GET', '/v1/payouts?status=x1', None),
            ("z.approve_payout('pay_1')", 'POST', '/v1/payouts/pay_1/approve', {}),
            ("z.reject_payout('pay_1')", 'POST', '/v1/payouts/pay_1/reject', {}),
            ("z.cancel_payout('pay_1')", 'POST', '/v1/payouts/pay_1/cancel', {}),
            ("z.mark_payout_paid('pay_1', 'x1')", 'POST', '/v1/payouts/pay_1/mark_paid', {'bank_ref': 'x1'}),
            ("z.list_customers(external_ref='x1')", 'GET', '/v1/customers?external_ref=x1', None),
            ("z.get_customer('cus_1')", 'GET', '/v1/customers/cus_1', None),
            ("z.create_customer('x1', email='x1', phone='x1', external_ref='x1', gstin='x1', address1='x1', location='x1', pincode='x1')", 'POST', '/v1/customers', {'name': 'x1', 'email': 'x1', 'phone': 'x1', 'external_ref': 'x1', 'gstin': 'x1', 'address1': 'x1', 'location': 'x1', 'pincode': 'x1'}),
            ("z.refresh_mandate('man_1')", 'POST', '/v1/mandates/man_1/refresh', {}),
            ("z.pause_mandate('man_1')", 'POST', '/v1/mandates/man_1/pause', {}),
            ("z.resume_mandate('man_1')", 'POST', '/v1/mandates/man_1/resume', {}),
            ("z.revoke_mandate('man_1')", 'POST', '/v1/mandates/man_1/revoke', {}),
            ('z.run_mandates()', 'POST', '/v1/mandates/run', {}),
            ('z.get_settings()', 'GET', '/v1/settings', None),
            ("z.create_coupon('x1', 'x1', 7, max_uses=7, expires_at='x1')", 'POST', '/v1/coupons', {'code': 'x1', 'kind': 'x1', 'value': 7, 'max_uses': 7, 'expires_at': 'x1'}),
            ('z.list_coupons()', 'GET', '/v1/coupons', None),
            ("z.deactivate_coupon('cou_1')", 'POST', '/v1/coupons/cou_1/deactivate', {}),
            ('z.list_milestones()', 'GET', '/v1/milestones', None),
            ("z.create_milestone('x1', 'x1', 7, due_days=7, external_ref='x1')", 'POST', '/v1/milestones', {'customer_id': 'x1', 'name': 'x1', 'amount_minor': 7, 'due_days': 7, 'external_ref': 'x1'}),
            ("z.cancel_milestone('mil_1')", 'POST', '/v1/milestones/mil_1/cancel', {}),
            ("z.create_installment_plan('x1', 'x1', 7, 7, 'x1', 'x1', every_unit='x1', every=7, currency='x1', external_ref='x1')", 'POST', '/v1/installment_plans', {'customer_id': 'x1', 'name': 'x1', 'total_minor': 7, 'instalments': 7, 'first_due_date': 'x1', 'number_prefix': 'x1', 'every_unit': 'x1', 'every': 7, 'currency': 'x1', 'external_ref': 'x1'}),
            ("z.get_installment_plan('pla_1')", 'GET', '/v1/installment_plans/pla_1', None),
            ("z.open_dispute('x1', 'x1', amount_minor=7, provider_ref='x1', respond_by='x1')", 'POST', '/v1/disputes', {'attempt_id': 'x1', 'reason': 'x1', 'amount_minor': 7, 'provider_ref': 'x1', 'respond_by': 'x1'}),
            ("z.list_disputes(status='x1')", 'GET', '/v1/disputes?status=x1', None),
            ("z.add_dispute_evidence('dis_1', 'x1', url='x1')", 'POST', '/v1/disputes/dis_1/evidence', {'note': 'x1', 'url': 'x1'}),
            ("z.resolve_dispute('dis_1', 'x1')", 'POST', '/v1/disputes/dis_1/resolve', {'outcome': 'x1'}),
            ('z.list_branches()', 'GET', '/v1/branches', None),
            ("z.create_branch('x1')", 'POST', '/v1/branches', {'name': 'x1'}),
            ('z.verify_audit()', 'GET', '/v1/audit_events/verify', None),
            ('z.seal_audit()', 'POST', '/v1/audit_events/seal', {}),
            ('z.list_keys()', 'GET', '/v1/api_keys', None),
            ("z.create_key('x1', ['a'], branch_id='x1', expires_at='x1', allowed_cidrs=['a'])", 'POST', '/v1/api_keys', {'name': 'x1', 'scopes': ['a'], 'branch_id': 'x1', 'expires_at': 'x1', 'allowed_cidrs': ['a']}),
            ("z.revoke_key('key_1')", 'POST', '/v1/api_keys/key_1/revoke', {}),
            ('z.list_providers()', 'GET', '/v1/provider_accounts', None),
            ("z.check_provider('pro_1')", 'POST', '/v1/provider_accounts/pro_1/check', {}),
            ("z.set_provider_enabled('pro_1', True)", 'POST', '/v1/provider_accounts/pro_1/enabled', {'enabled': True}),
            ('z.live_access_status()', 'GET', '/v1/live_access', None),
            ('z.request_live_access()', 'POST', '/v1/live_access', {}),
            ('z.event_catalogue()', 'GET', '/v1/events/catalogue', None),
            ("z.list_outbox(status='x1')", 'GET', '/v1/outbox?status=x1', None),
            ("z.replay_event('eve_1')", 'POST', '/v1/outbox/eve_1/replay', {}),
        ]
        for call, verb, url, body in rows:
            seen.clear()
            eval(call, {"z": z})
            got = seen[0]
            # a GET carries no body; a POST with no fields carries an empty object
            self.assertEqual((got[0], got[1]), (verb, url), call)
            self.assertEqual(got[2], body, call)
        self.assertEqual(len(rows), 74)

    def test_update_settings_sends_only_what_it_is_given(self):
        seen = []
        z = Zyvor("k", opener=lambda req, timeout=None: (seen.append(json.loads(req.data)), FakeRes(b"{}"))[1])
        z.update_settings({"dunning": {"enabled": True}})
        self.assertEqual(seen, [{"dunning": {"enabled": True}}])

    def test_the_go_sdk_has_every_method_this_one_has_and_the_other_way_round(self):
        """Parity guard: both SDKs expose the same operations (Go's *At/*AsOf variants are Python keyword arguments)."""
        import pathlib, re
        go_src = (pathlib.Path(__file__).resolve().parent.parent / "go" / "zyvor.go").read_text()
        go = {re.sub(r"(?<!^)(?=[A-Z])", "_", m).lower() for m in re.findall(r"func \(c \*Client\) ([A-Z]\w+)", go_src)}
        py = {n for n in dir(Zyvor) if not n.startswith("_")}
        go -= {"create_intent_at", "run_dunning_as_of"}
        self.assertEqual(sorted(py - go), [], "in Python but not Go")
        self.assertEqual(sorted(go - py), [], "in Go but not Python")


if __name__ == "__main__":
    unittest.main()

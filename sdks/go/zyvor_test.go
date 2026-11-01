package zyvor

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestCreateIntentSendsAuthIdempotencyAndMinorUnits(t *testing.T) {
	var gotKey, gotAuth, gotActor string
	var body map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotKey, gotAuth, gotActor = r.Header.Get("Idempotency-Key"), r.Header.Get("Authorization"), r.Header.Get("Zyvor-Actor")
		_ = json.NewDecoder(r.Body).Decode(&body)
		w.WriteHeader(201)
		_, _ = w.Write([]byte(`{"id":"pi_1","amount_minor":49900,"currency":"INR","status":"requires_payment_method"}`))
	}))
	defer srv.Close()
	c := New("zk_test_x", srv.URL)
	c.Actor = "priya"
	in, err := c.CreateIntent(49900, "", "")
	if err != nil || in.ID != "pi_1" {
		t.Fatalf("%v %+v", err, in)
	}
	if gotAuth != "Bearer zk_test_x" || gotActor != "priya" || len(gotKey) != 32 || body["amount_minor"] != float64(49900) {
		t.Fatalf("bad request: %q %q %q %v", gotAuth, gotActor, gotKey, body)
	}
	if _, err := c.CreateIntent(0, "", ""); err == nil {
		t.Fatal("zero amount must be rejected before any request")
	}
}

func TestErrorsCarryCode(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(409)
		_, _ = w.Write([]byte(`{"error":{"code":"payment_outcome_unresolved","message":"resolve first"}}`))
	}))
	defer srv.Close()
	_, err := New("k", srv.URL).CreateAttempt("pi_1", "upi", "", "")
	e, ok := err.(*Error)
	if !ok || e.Status != 409 || e.Code != "payment_outcome_unresolved" {
		t.Fatalf("%v", err)
	}
}

func TestVerifyWebhook(t *testing.T) {
	body := []byte(`{"type":"payment.captured"}`)
	sign := func(ts int64, secret string) string {
		m := hmac.New(sha256.New, []byte(secret))
		m.Write([]byte(fmt.Sprintf("%d.", ts)))
		m.Write(body)
		return fmt.Sprintf("t=%d,v1=%s", ts, hex.EncodeToString(m.Sum(nil)))
	}
	now := time.Unix(1100, 0)
	if !VerifyWebhook("whsec", sign(1000, "whsec"), body, 5*time.Minute, now) {
		t.Fatal("valid signature rejected")
	}
	if VerifyWebhook("whsec", sign(1000, "whsec"), append(body, ' '), 5*time.Minute, now) ||
		VerifyWebhook("other", sign(1000, "whsec"), body, 5*time.Minute, now) ||
		VerifyWebhook("whsec", sign(1000, "whsec"), body, 5*time.Minute, time.Unix(9000, 0)) ||
		VerifyWebhook("whsec", "junk", body, 5*time.Minute, now) {
		t.Fatal("bad signature accepted")
	}
}

func TestBillingAndCollectionsPaths(t *testing.T) {
	var seen []string
	var usage map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = append(seen, r.Method+" "+r.URL.RequestURI())
		if r.URL.Path == "/v1/subscriptions/sub_1/usage" {
			_ = json.NewDecoder(r.Body).Decode(&usage)
		}
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	_, _ = c.Subscribe("cus_1", "plan_1", 2, "2026-11-01")
	_, _ = c.ChangePlan("sub_1", "plan_2")
	_, _ = c.RecordUsage("sub_1", 5, "u1")
	_, _ = c.CreditNote("inv_1", 100, "goodwill")
	_, _ = c.Ageing("2026-10-31")
	_, _ = c.PromiseToPay("inv_1", 500, "2026-11-05")
	_, _ = c.SetFxRate("83.2150", "")
	_, _ = c.FxExposure("2026-10-31")
	_, _ = c.ListPayments("succeeded", "ORD 1&x", 10)
	_, _ = c.CreateMandate("cus_1", 500000, "monthly", "2027-10-31", "m1")
	_, _ = c.MandateAction("mnd_1", "pause")
	_, _ = c.ScheduleDebit("mnd_1", 125000, "2026-11-30", "d1")
	if _, err := c.MandateAction("mnd_1", "delete"); err == nil {
		t.Fatal("an unknown action must be refused before any request")
	}
	if _, err := c.CreateMandate("cus_1", 0, "daily", "2027-01-01", ""); err == nil {
		t.Fatal("a zero limit must be refused before any request")
	}
	_, _ = c.SyncSettlements("sim", "2026-11-01")
	_, _ = c.SendTestEvent()
	_, _ = c.RunBilling("2027-01-15")
	_, _ = c.RunDunningAsOf("2027-01-15", "sms", 0, 0)
	if _, err := c.RunBilling("15/01/2027"); err == nil {
		t.Fatal("a malformed day must be refused before any request")
	}
	_, _ = c.SimulatePaymentEvent("att_1", "captured")
	if _, err := c.SimulatePaymentEvent("att_1", "refunded"); err == nil {
		t.Fatal("an unknown outcome must be refused before any request")
	}
	_, _ = c.SimulateBankCredit("stl_1", 5000)
	if _, err := c.SimulateBankCredit("stl_1", -1); err == nil {
		t.Fatal("a negative shortfall must be refused before any request")
	}
	_, _ = c.EinvoiceDraft("inv_1")
	_, _ = c.UpdateCustomer("cus_1", map[string]string{"pincode": "110001"})
	want := []string{"POST /v1/subscriptions", "POST /v1/subscriptions/sub_1/change_plan", "POST /v1/subscriptions/sub_1/usage",
		"POST /v1/invoices/inv_1/credit_notes", "GET /v1/collections/ageing?as_of=2026-10-31", "POST /v1/invoices/inv_1/promises",
		"POST /v1/fx/rates", "GET /v1/fx/exposure?as_of=2026-10-31", "GET /v1/payment_intents?limit=10&q=ORD+1%26x&status=succeeded", "POST /v1/mandates", "POST /v1/mandates/mnd_1/pause", "POST /v1/mandates/mnd_1/debits", "POST /v1/settlements/sync", "POST /v1/outbox/test", "POST /v1/billing/run", "POST /v1/collections/dunning/run", "POST /v1/payment_attempts/att_1/simulate_event", "POST /v1/settlements/stl_1/simulate_bank", "GET /v1/invoices/inv_1/einvoice", "PATCH /v1/customers/cus_1"}
	if fmt.Sprint(seen) != fmt.Sprint(want) {
		t.Fatalf("got %v", seen)
	}
	if usage["reference"] != "u1" || usage["quantity"] != float64(5) {
		t.Fatalf("usage body %v", usage)
	}
}

func TestPaymentLinksDunningAndRotationPaths(t *testing.T) {
	var seen []string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = append(seen, r.Method+" "+r.URL.Path)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	if _, err := c.CreatePaymentLink(0, "", "", 0); err == nil {
		t.Fatal("a link needs an amount or an invoice")
	}
	if _, err := c.CreatePaymentLink(100, "inv_1", "", 0); err == nil {
		t.Fatal("amount and invoice together must be rejected before any request")
	}
	_, _ = c.CreatePaymentLink(5000, "", "Term fee", 3)
	_, _ = c.CancelPaymentLink("plk_1")
	_, _ = c.RunDunning("sms", 0, 0)
	_, _ = c.RotateKey("key_1", 2)
	want := []string{"POST /v1/payment_links", "POST /v1/payment_links/plk_1/cancel", "POST /v1/collections/dunning/run", "POST /v1/api_keys/key_1/rotate"}
	if fmt.Sprint(seen) != fmt.Sprint(want) {
		t.Fatalf("got %v", seen)
	}
}

func TestReminderDraftPaths(t *testing.T) {
	var seen []string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = append(seen, r.Method+" "+r.URL.Path)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	_, _ = c.DraftReminder("inv_1", "sms", true)
	_, _ = c.ApproveDraft("rdf_1", "Edited")
	_, _ = c.RejectDraft("rdf_2")
	want := []string{"POST /v1/invoices/inv_1/reminders/draft", "POST /v1/collections/drafts/rdf_1/approve", "POST /v1/collections/drafts/rdf_2/reject"}
	if fmt.Sprint(seen) != fmt.Sprint(want) {
		t.Fatalf("got %v", seen)
	}
}

func TestMerchantOperationsPaths(t *testing.T) {
	var seen []string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = append(seen, r.Method+" "+r.URL.Path+"?"+r.URL.RawQuery)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	_, _ = c.PaymentTimeline("pi_1")
	_, _ = c.SaveProfile("Sri Clinic", "llp", "ABCDE1234F", "12 MG Road", "")
	_, _ = c.ReviewOnboarding("reject", "blurry PAN")
	_, _ = c.CreateCounter("br_1", "Till 1")
	_, _ = c.BranchSummary("br_1", "2026-10-01", "")
	_, _ = c.CloseDay("br_1", "2026-10-30", 500)
	_, _ = c.SignOffClosing("clo_1")
	want := []string{"GET /v1/payment_intents/pi_1/timeline?", "PUT /v1/onboarding?", "POST /v1/onboarding/review?", "POST /v1/counters?", "GET /v1/branches/br_1/summary?from=2026-10-01", "POST /v1/closings?", "POST /v1/closings/clo_1/signoff?"}
	if fmt.Sprint(seen) != fmt.Sprint(want) {
		t.Fatalf("got %v", seen)
	}
}

func TestRequestProfileChangeSendsTheReasonAndOmitsAnEmptyGSTIN(t *testing.T) {
	var path string
	var body map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		path = r.Method + " " + r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&body)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	_, _ = c.RequestProfileChange("Sri Clinic Pvt Ltd", "private_limited", "ABCPE1234F", "Pune 411001", "", "converted to a company")
	if path != "POST /v1/onboarding/profile_change" || body["reason"] != "converted to a company" || body["legal_name"] != "Sri Clinic Pvt Ltd" {
		t.Fatalf("got %s %v", path, body)
	}
	if _, has := body["gstin"]; has {
		t.Fatalf("an empty gstin should not be sent: %v", body)
	}
}

func TestCreateIntentAtSendsBranchAndCounter(t *testing.T) {
	var got map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&got)
		_, _ = w.Write([]byte(`{"id":"pi_1"}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	if _, err := c.CreateIntentAt(0, "", "br_1", "", ""); err == nil {
		t.Fatal("a non-positive amount must be rejected before any request")
	}
	if _, err := c.CreateIntentAt(500, "", "br_1", "ctr_1", "key-1"); err != nil {
		t.Fatal(err)
	}
	if got["branch_id"] != "br_1" || got["counter_id"] != "ctr_1" || got["currency"] != "INR" {
		t.Fatalf("got %v", got)
	}
}

func TestEveryEndpointMethodSendsTheRightVerbPathAndBody(t *testing.T) {
	var method, path string
	var body map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		method, path = r.Method, r.URL.RequestURI()
		body = nil
		_ = json.NewDecoder(r.Body).Decode(&body)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	c := New("k", srv.URL)
	rows := []struct {
		name string
		call func() (map[string]any, error)
		verb string
		path string
		body map[string]any
	}{
		{"GetAttempt", func() (map[string]any, error) { return c.GetAttempt("att_1") }, "GET", "/v1/payment_attempts/att_1", nil},
		{"RefreshAttempt", func() (map[string]any, error) { return c.RefreshAttempt("att_1") }, "POST", "/v1/payment_attempts/att_1/refresh", map[string]any{}},
		{"CancelAttempt", func() (map[string]any, error) { return c.CancelAttempt("att_1") }, "POST", "/v1/payment_attempts/att_1/cancel", map[string]any{}},
		{"CaptureAttempt", func() (map[string]any, error) { return c.CaptureAttempt("att_1") }, "POST", "/v1/payment_attempts/att_1/capture", map[string]any{}},
		{"ListRefunds", func() (map[string]any, error) { return c.ListRefunds(map[string]any{"status": "x1", "limit": 7}) }, "GET", "/v1/refunds?limit=7&status=x1", nil},
		{"ApproveRefund", func() (map[string]any, error) { return c.ApproveRefund("ref_1") }, "POST", "/v1/refunds/ref_1/approve", map[string]any{}},
		{"RejectRefund", func() (map[string]any, error) { return c.RejectRefund("ref_1") }, "POST", "/v1/refunds/ref_1/reject", map[string]any{}},
		{"RetryRefund", func() (map[string]any, error) { return c.RetryRefund("ref_1") }, "POST", "/v1/refunds/ref_1/retry", map[string]any{}},
		{"RefreshRefund", func() (map[string]any, error) { return c.RefreshRefund("ref_1") }, "POST", "/v1/refunds/ref_1/refresh", map[string]any{}},
		{"ListSettlements", func() (map[string]any, error) { return c.ListSettlements() }, "GET", "/v1/settlements", nil},
		{"ListCases", func() (map[string]any, error) { return c.ListCases(map[string]any{"status": "x1", "kind": "x1"}) }, "GET", "/v1/recon/cases?kind=x1&status=x1", nil},
		{"CaseSuggestions", func() (map[string]any, error) { return c.CaseSuggestions("cas_1") }, "GET", "/v1/recon/cases/cas_1/suggestions", nil},
		{"AssignCase", func() (map[string]any, error) { return c.AssignCase("cas_1", "x1") }, "POST", "/v1/recon/cases/cas_1/assign", map[string]any{"assignee": "x1"}},
		{"ResolveCase", func() (map[string]any, error) { return c.ResolveCase("cas_1", "x1") }, "POST", "/v1/recon/cases/cas_1/resolve", map[string]any{"resolution": "x1"}},
		{"ListPlans", func() (map[string]any, error) { return c.ListPlans() }, "GET", "/v1/plans", nil},
		{"ListSubscriptions", func() (map[string]any, error) { return c.ListSubscriptions() }, "GET", "/v1/subscriptions", nil},
		{"PauseSubscription", func() (map[string]any, error) { return c.PauseSubscription("sub_1") }, "POST", "/v1/subscriptions/sub_1/pause", map[string]any{}},
		{"ResumeSubscription", func() (map[string]any, error) { return c.ResumeSubscription("sub_1") }, "POST", "/v1/subscriptions/sub_1/resume", map[string]any{}},
		{"CancelSubscription", func() (map[string]any, error) { return c.CancelSubscription("sub_1") }, "POST", "/v1/subscriptions/sub_1/cancel", map[string]any{}},
		{"ListInvoices", func() (map[string]any, error) { return c.ListInvoices(map[string]any{"status": "x1"}) }, "GET", "/v1/invoices?status=x1", nil},
		{"GetInvoice", func() (map[string]any, error) { return c.GetInvoice("inv_1") }, "GET", "/v1/invoices/inv_1", nil},
		{"CreateInvoice", func() (map[string]any, error) {
			return c.CreateInvoice("x1", map[string]any{"amount_minor": 7, "customer_id": "x1", "due_date": "x1", "tax_minor": 7, "external_ref": "x1", "lines": []any{"a"}, "place_of_supply": "x1", "buyer_gstin": "x1", "reverse_charge": true})
		}, "POST", "/v1/invoices", map[string]any{"number": "x1", "amount_minor": 7, "customer_id": "x1", "due_date": "x1", "tax_minor": 7, "external_ref": "x1", "lines": []any{"a"}, "place_of_supply": "x1", "buyer_gstin": "x1", "reverse_charge": true}},
		{"VoidInvoice", func() (map[string]any, error) { return c.VoidInvoice("inv_1") }, "POST", "/v1/invoices/inv_1/void", map[string]any{}},
		{"InvoiceLines", func() (map[string]any, error) { return c.InvoiceLines("inv_1") }, "GET", "/v1/invoices/inv_1/lines", nil},
		{"ListReminders", func() (map[string]any, error) { return c.ListReminders("inv_1") }, "GET", "/v1/invoices/inv_1/reminders", nil},
		{"ListPromises", func() (map[string]any, error) { return c.ListPromises(map[string]any{"status": "x1"}) }, "GET", "/v1/collections/promises?status=x1", nil},
		{"CancelPromise", func() (map[string]any, error) { return c.CancelPromise("pro_1") }, "POST", "/v1/collections/promises/pro_1/cancel", map[string]any{}},
		{"CreateVendor", func() (map[string]any, error) {
			return c.CreateVendor("x1", map[string]any{"account_number": "x1", "ifsc": "x1", "vpa": "x1", "external_ref": "x1"})
		}, "POST", "/v1/vendors", map[string]any{"name": "x1", "account_number": "x1", "ifsc": "x1", "vpa": "x1", "external_ref": "x1"}},
		{"ListVendors", func() (map[string]any, error) { return c.ListVendors() }, "GET", "/v1/vendors", nil},
		{"VendorDetails", func() (map[string]any, error) { return c.VendorDetails("ven_1") }, "GET", "/v1/vendors/ven_1/details", nil},
		{"VendorBalance", func() (map[string]any, error) { return c.VendorBalance("ven_1") }, "GET", "/v1/vendors/ven_1/balance", nil},
		{"CreatePayout", func() (map[string]any, error) {
			return c.CreatePayout("x1", 7, "x1", map[string]any{"currency": "x1", "external_ref": "x1", "from_balance": true})
		}, "POST", "/v1/payouts", map[string]any{"vendor_id": "x1", "amount_minor": 7, "purpose": "x1", "currency": "x1", "external_ref": "x1", "from_balance": true}},
		{"CreatePayoutsBulk", func() (map[string]any, error) { return c.CreatePayoutsBulk([]any{"a"}) }, "POST", "/v1/payouts/bulk", map[string]any{"payouts": []any{"a"}}},
		{"ListPayouts", func() (map[string]any, error) { return c.ListPayouts(map[string]any{"status": "x1"}) }, "GET", "/v1/payouts?status=x1", nil},
		{"ApprovePayout", func() (map[string]any, error) { return c.ApprovePayout("pay_1") }, "POST", "/v1/payouts/pay_1/approve", map[string]any{}},
		{"RejectPayout", func() (map[string]any, error) { return c.RejectPayout("pay_1") }, "POST", "/v1/payouts/pay_1/reject", map[string]any{}},
		{"CancelPayout", func() (map[string]any, error) { return c.CancelPayout("pay_1") }, "POST", "/v1/payouts/pay_1/cancel", map[string]any{}},
		{"MarkPayoutPaid", func() (map[string]any, error) { return c.MarkPayoutPaid("pay_1", "x1") }, "POST", "/v1/payouts/pay_1/mark_paid", map[string]any{"bank_ref": "x1"}},
		{"ListCustomers", func() (map[string]any, error) { return c.ListCustomers(map[string]any{"external_ref": "x1"}) }, "GET", "/v1/customers?external_ref=x1", nil},
		{"GetCustomer", func() (map[string]any, error) { return c.GetCustomer("cus_1") }, "GET", "/v1/customers/cus_1", nil},
		{"CreateCustomer", func() (map[string]any, error) {
			return c.CreateCustomer("x1", map[string]any{"email": "x1", "phone": "x1", "external_ref": "x1", "gstin": "x1", "address1": "x1", "location": "x1", "pincode": "x1"})
		}, "POST", "/v1/customers", map[string]any{"name": "x1", "email": "x1", "phone": "x1", "external_ref": "x1", "gstin": "x1", "address1": "x1", "location": "x1", "pincode": "x1"}},
		{"RefreshMandate", func() (map[string]any, error) { return c.RefreshMandate("man_1") }, "POST", "/v1/mandates/man_1/refresh", map[string]any{}},
		{"PauseMandate", func() (map[string]any, error) { return c.PauseMandate("man_1") }, "POST", "/v1/mandates/man_1/pause", map[string]any{}},
		{"ResumeMandate", func() (map[string]any, error) { return c.ResumeMandate("man_1") }, "POST", "/v1/mandates/man_1/resume", map[string]any{}},
		{"RevokeMandate", func() (map[string]any, error) { return c.RevokeMandate("man_1") }, "POST", "/v1/mandates/man_1/revoke", map[string]any{}},
		{"RunMandates", func() (map[string]any, error) { return c.RunMandates() }, "POST", "/v1/mandates/run", map[string]any{}},
		{"GetSettings", func() (map[string]any, error) { return c.GetSettings() }, "GET", "/v1/settings", nil},
		{"CreateCoupon", func() (map[string]any, error) {
			return c.CreateCoupon("x1", "x1", 7, map[string]any{"max_uses": 7, "expires_at": "x1"})
		}, "POST", "/v1/coupons", map[string]any{"code": "x1", "kind": "x1", "value": 7, "max_uses": 7, "expires_at": "x1"}},
		{"ListCoupons", func() (map[string]any, error) { return c.ListCoupons() }, "GET", "/v1/coupons", nil},
		{"DeactivateCoupon", func() (map[string]any, error) { return c.DeactivateCoupon("cou_1") }, "POST", "/v1/coupons/cou_1/deactivate", map[string]any{}},
		{"ListMilestones", func() (map[string]any, error) { return c.ListMilestones() }, "GET", "/v1/milestones", nil},
		{"CreateMilestone", func() (map[string]any, error) {
			return c.CreateMilestone("x1", "x1", 7, map[string]any{"due_days": 7, "external_ref": "x1"})
		}, "POST", "/v1/milestones", map[string]any{"customer_id": "x1", "name": "x1", "amount_minor": 7, "due_days": 7, "external_ref": "x1"}},
		{"CancelMilestone", func() (map[string]any, error) { return c.CancelMilestone("mil_1") }, "POST", "/v1/milestones/mil_1/cancel", map[string]any{}},
		{"CreateInstallmentPlan", func() (map[string]any, error) {
			return c.CreateInstallmentPlan("x1", "x1", 7, 7, "x1", "x1", map[string]any{"every_unit": "x1", "every": 7, "currency": "x1", "external_ref": "x1"})
		}, "POST", "/v1/installment_plans", map[string]any{"customer_id": "x1", "name": "x1", "total_minor": 7, "instalments": 7, "first_due_date": "x1", "number_prefix": "x1", "every_unit": "x1", "every": 7, "currency": "x1", "external_ref": "x1"}},
		{"GetInstallmentPlan", func() (map[string]any, error) { return c.GetInstallmentPlan("pla_1") }, "GET", "/v1/installment_plans/pla_1", nil},
		{"OpenDispute", func() (map[string]any, error) {
			return c.OpenDispute("x1", "x1", map[string]any{"amount_minor": 7, "provider_ref": "x1", "respond_by": "x1"})
		}, "POST", "/v1/disputes", map[string]any{"attempt_id": "x1", "reason": "x1", "amount_minor": 7, "provider_ref": "x1", "respond_by": "x1"}},
		{"ListDisputes", func() (map[string]any, error) { return c.ListDisputes(map[string]any{"status": "x1"}) }, "GET", "/v1/disputes?status=x1", nil},
		{"AddDisputeEvidence", func() (map[string]any, error) {
			return c.AddDisputeEvidence("dis_1", "x1", map[string]any{"url": "x1"})
		}, "POST", "/v1/disputes/dis_1/evidence", map[string]any{"note": "x1", "url": "x1"}},
		{"ResolveDispute", func() (map[string]any, error) { return c.ResolveDispute("dis_1", "x1") }, "POST", "/v1/disputes/dis_1/resolve", map[string]any{"outcome": "x1"}},
		{"ListBranches", func() (map[string]any, error) { return c.ListBranches() }, "GET", "/v1/branches", nil},
		{"CreateBranch", func() (map[string]any, error) { return c.CreateBranch("x1") }, "POST", "/v1/branches", map[string]any{"name": "x1"}},
		{"VerifyAudit", func() (map[string]any, error) { return c.VerifyAudit() }, "GET", "/v1/audit_events/verify", nil},
		{"SealAudit", func() (map[string]any, error) { return c.SealAudit() }, "POST", "/v1/audit_events/seal", map[string]any{}},
		{"ListKeys", func() (map[string]any, error) { return c.ListKeys() }, "GET", "/v1/api_keys", nil},
		{"CreateKey", func() (map[string]any, error) {
			return c.CreateKey("x1", []any{"a"}, map[string]any{"branch_id": "x1", "expires_at": "x1", "allowed_cidrs": []any{"a"}})
		}, "POST", "/v1/api_keys", map[string]any{"name": "x1", "scopes": []any{"a"}, "branch_id": "x1", "expires_at": "x1", "allowed_cidrs": []any{"a"}}},
		{"RevokeKey", func() (map[string]any, error) { return c.RevokeKey("key_1") }, "POST", "/v1/api_keys/key_1/revoke", map[string]any{}},
		{"ListProviders", func() (map[string]any, error) { return c.ListProviders() }, "GET", "/v1/provider_accounts", nil},
		{"CheckProvider", func() (map[string]any, error) { return c.CheckProvider("pro_1") }, "POST", "/v1/provider_accounts/pro_1/check", map[string]any{}},
		{"SetProviderEnabled", func() (map[string]any, error) { return c.SetProviderEnabled("pro_1", true) }, "POST", "/v1/provider_accounts/pro_1/enabled", map[string]any{"enabled": true}},
		{"LiveAccessStatus", func() (map[string]any, error) { return c.LiveAccessStatus() }, "GET", "/v1/live_access", nil},
		{"RequestLiveAccess", func() (map[string]any, error) { return c.RequestLiveAccess() }, "POST", "/v1/live_access", map[string]any{}},
		{"EventCatalogue", func() (map[string]any, error) { return c.EventCatalogue() }, "GET", "/v1/events/catalogue", nil},
		{"ListOutbox", func() (map[string]any, error) { return c.ListOutbox(map[string]any{"status": "x1"}) }, "GET", "/v1/outbox?status=x1", nil},
		{"ReplayEvent", func() (map[string]any, error) { return c.ReplayEvent("eve_1") }, "POST", "/v1/outbox/eve_1/replay", map[string]any{}},
		{"GetIntent", func() (map[string]any, error) { return c.GetIntent("int_1") }, "GET", "/v1/payment_intents/int_1", nil},
		{"ExplainSettlement", func() (map[string]any, error) { return c.ExplainSettlement("set_1") }, "GET", "/v1/settlements/set_1/explain", nil},
		{"CreatePlan", func() (map[string]any, error) {
			return c.CreatePlan("x1", 7, "x1", map[string]any{"interval_count": 7, "usage_unit_minor": 7, "due_days": 7, "currency": "x1"})
		}, "POST", "/v1/plans", map[string]any{"name": "x1", "amount_minor": 7, "interval_unit": "x1", "interval_count": 7, "usage_unit_minor": 7, "due_days": 7, "currency": "x1"}},
		{"Statement", func() (map[string]any, error) { return c.Statement("cus_1") }, "GET", "/v1/customers/cus_1/statement", nil},
		{"CompleteMilestone", func() (map[string]any, error) { return c.CompleteMilestone("mil_1") }, "POST", "/v1/milestones/mil_1/complete", map[string]any{}},
		{"FxRates", func() (map[string]any, error) { return c.FxRates() }, "GET", "/v1/fx/rates", nil},
		{"FxConvert", func() (map[string]any, error) { return c.FxConvert(7, "x1", "x1", map[string]any{"as_of": "x1"}) }, "GET", "/v1/fx/convert?amount_minor=7&as_of=x1&from=x1&to=x1", nil},
		{"ListMandates", func() (map[string]any, error) {
			return c.ListMandates(map[string]any{"status": "x1", "customer_id": "x1"})
		}, "GET", "/v1/mandates?customer_id=x1&status=x1", nil},
		{"ListMandateDebits", func() (map[string]any, error) { return c.ListMandateDebits("man_1") }, "GET", "/v1/mandates/man_1/debits", nil},
		{"ListPaymentLinks", func() (map[string]any, error) { return c.ListPaymentLinks() }, "GET", "/v1/payment_links", nil},
		{"Remind", func() (map[string]any, error) { return c.Remind("inv_1", "x1", map[string]any{"message": "x1"}) }, "POST", "/v1/invoices/inv_1/reminders", map[string]any{"channel": "x1", "message": "x1"}},
		{"ReminderDrafts", func() (map[string]any, error) { return c.ReminderDrafts(map[string]any{"status": "x1"}) }, "GET", "/v1/collections/drafts?status=x1", nil},
		{"ListCounters", func() (map[string]any, error) { return c.ListCounters(map[string]any{"branch_id": "x1"}) }, "GET", "/v1/counters?branch_id=x1", nil},
		{"Closings", func() (map[string]any, error) { return c.Closings(map[string]any{"branch_id": "x1", "status": "x1"}) }, "GET", "/v1/closings?branch_id=x1&status=x1", nil},
		{"RefundAnomalies", func() (map[string]any, error) { return c.RefundAnomalies() }, "GET", "/v1/insights/refund_anomalies", nil},
	}
	for _, r := range rows {
		if _, err := r.call(); err != nil {
			t.Fatalf("%s: %v", r.name, err)
		}
		if method != r.verb || path != r.path {
			t.Fatalf("%s: sent %s %s, want %s %s", r.name, method, path, r.verb, r.path)
		}
		if r.verb == "POST" {
			want, _ := json.Marshal(r.body)
			got, _ := json.Marshal(body)
			if string(want) != string(got) {
				t.Fatalf("%s: body %s, want %s", r.name, got, want)
			}
		}
	}
	if len(rows) != 89 {
		t.Fatalf("expected 89 rows, got %d", len(rows))
	}
}

func TestUpdateSettingsSendsOnlyThePatch(t *testing.T) {
	var got map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&got)
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	if _, err := New("k", srv.URL).UpdateSettings(map[string]any{"env": "x"}); err != nil || len(got) != 1 || got["env"] != "x" {
		t.Fatalf("%v %v", err, got)
	}
}

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
	want := []string{"POST /v1/subscriptions", "POST /v1/subscriptions/sub_1/change_plan", "POST /v1/subscriptions/sub_1/usage",
		"POST /v1/invoices/inv_1/credit_notes", "GET /v1/collections/ageing?as_of=2026-10-31", "POST /v1/invoices/inv_1/promises"}
	if fmt.Sprint(seen) != fmt.Sprint(want) {
		t.Fatalf("got %v", seen)
	}
	if usage["reference"] != "u1" || usage["quantity"] != float64(5) {
		t.Fatalf("usage body %v", usage)
	}
}

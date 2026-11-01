// Package zyvor is a minimal Go client for Zyvor Payments. Amounts are integer minor units (paise).
package zyvor

import (
	"bytes"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// Error is returned for any non-2xx API response.
type Error struct {
	Status  int
	Code    string
	Message string
}

func (e *Error) Error() string { return fmt.Sprintf("%d %s: %s", e.Status, e.Code, e.Message) }

type Client struct {
	APIKey  string
	BaseURL string
	// Actor attributes writes to a person (audit trail, approval separation of duties).
	Actor string
	HTTP  *http.Client
}

func New(apiKey, baseURL string) *Client {
	if baseURL == "" {
		baseURL = "http://localhost:8080"
	}
	return &Client{APIKey: apiKey, BaseURL: strings.TrimRight(baseURL, "/"), HTTP: &http.Client{Timeout: 30 * time.Second}}
}

func newKey() string {
	b := make([]byte, 16)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)
}

func (c *Client) do(method, path string, body any, idem string, out any) error {
	var rdr io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		rdr = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, c.BaseURL+path, rdr)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.APIKey)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if idem != "" {
		req.Header.Set("Idempotency-Key", idem)
	}
	if c.Actor != "" {
		req.Header.Set("Zyvor-Actor", c.Actor)
	}
	res, err := c.HTTP.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	raw, _ := io.ReadAll(res.Body)
	if res.StatusCode >= 300 {
		var e struct {
			Error struct{ Code, Message string }
		}
		_ = json.Unmarshal(raw, &e)
		return &Error{res.StatusCode, e.Error.Code, e.Error.Message}
	}
	if out != nil {
		return json.Unmarshal(raw, out)
	}
	return nil
}

type Intent struct {
	ID           string `json:"id"`
	AmountMinor  int64  `json:"amount_minor"`
	Currency     string `json:"currency"`
	Status       string `json:"status"`
	ClientSecret string `json:"client_secret,omitempty"`
}

type Attempt struct {
	ID       string `json:"id"`
	Provider string `json:"provider"`
	Method   string `json:"method"`
	// Status "unknown" means the provider's outcome is unknown; no new attempt can start until it resolves.
	Status string `json:"status"`
}

type Refund struct {
	ID          string `json:"id"`
	AmountMinor int64  `json:"amount_minor"`
	Status      string `json:"status"`
}

// CreateIntent creates a payment intent. Pass the same idempotencyKey when retrying; "" generates one.
func (c *Client) CreateIntent(amountMinor int64, currency, idempotencyKey string) (*Intent, error) {
	if amountMinor <= 0 {
		return nil, fmt.Errorf("amountMinor must be a positive integer (paise)")
	}
	if currency == "" {
		currency = "INR"
	}
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	var out Intent
	err := c.do("POST", "/v1/payment_intents", map[string]any{"amount_minor": amountMinor, "currency": currency}, idempotencyKey, &out)
	return &out, err
}

// CreateIntentAt is CreateIntent attributed to a branch and/or a counter within it (either may be empty).
func (c *Client) CreateIntentAt(amountMinor int64, currency, branchID, counterID, idempotencyKey string) (*Intent, error) {
	if amountMinor <= 0 {
		return nil, fmt.Errorf("amountMinor must be a positive integer (paise)")
	}
	if currency == "" {
		currency = "INR"
	}
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	body := map[string]any{"amount_minor": amountMinor, "currency": currency}
	if branchID != "" {
		body["branch_id"] = branchID
	}
	if counterID != "" {
		body["counter_id"] = counterID
	}
	var out Intent
	err := c.do("POST", "/v1/payment_intents", body, idempotencyKey, &out)
	return &out, err
}

func (c *Client) CreateAttempt(intentID, method, vpa, idempotencyKey string) (*Attempt, error) {
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	var out Attempt
	err := c.do("POST", "/v1/payment_intents/"+intentID+"/attempts", map[string]any{"method": method, "vpa": vpa}, idempotencyKey, &out)
	return &out, err
}

// Refund refunds amountMinor (0 = everything still refundable).
func (c *Client) Refund(intentID string, amountMinor int64, idempotencyKey string) (*Refund, error) {
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	body := map[string]any{}
	if amountMinor > 0 {
		body["amount_minor"] = amountMinor
	}
	var out Refund
	err := c.do("POST", "/v1/payment_intents/"+intentID+"/refunds", body, idempotencyKey, &out)
	return &out, err
}

// Subscribe starts a subscription. startDate is YYYY-MM-DD ("" = today).
func (c *Client) Subscribe(customerID, planID string, quantity int, startDate string) (map[string]any, error) {
	body := map[string]any{"customer_id": customerID, "plan_id": planID}
	if quantity > 0 {
		body["quantity"] = quantity
	}
	if startDate != "" {
		body["start_date"] = startDate
	}
	var out map[string]any
	err := c.do("POST", "/v1/subscriptions", body, "", &out)
	return out, err
}

// ChangePlan upgrades are prorated for the rest of the period; downgrades apply from the next invoice.
func (c *Client) ChangePlan(subscriptionID, planID string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/subscriptions/"+subscriptionID+"/change_plan", map[string]any{"plan_id": planID}, "", &out)
	return out, err
}

// RecordUsage is retry-safe when reference is set: the same reference is recorded once.
func (c *Client) RecordUsage(subscriptionID string, quantity int64, reference string) (map[string]any, error) {
	body := map[string]any{"quantity": quantity}
	if reference != "" {
		body["reference"] = reference
	}
	var out map[string]any
	err := c.do("POST", "/v1/subscriptions/"+subscriptionID+"/usage", body, "", &out)
	return out, err
}

func (c *Client) CreditNote(invoiceID string, amountMinor int64, reason string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/invoices/"+invoiceID+"/credit_notes", map[string]any{"amount_minor": amountMinor, "reason": reason}, "", &out)
	return out, err
}

// Ageing reports unpaid invoices by days past due. asOf is YYYY-MM-DD ("" = today).
func (c *Client) Ageing(asOf string) (map[string]any, error) {
	path := "/v1/collections/ageing"
	if asOf != "" {
		path += "?as_of=" + asOf
	}
	var out map[string]any
	err := c.do("GET", path, nil, "", &out)
	return out, err
}

// SetFxRate records the INR-per-dollar rate you use for a day, e.g. "83.2150" (asOf "" = today). One per day; never edited.
func (c *Client) SetFxRate(rate, asOf string) (map[string]any, error) {
	body := map[string]any{"currency": "USD", "rate": rate}
	if asOf != "" {
		body["as_of"] = asOf
	}
	var out map[string]any
	err := c.do("POST", "/v1/fx/rates", body, "", &out)
	return out, err
}

// UpdateCustomer corrects a customer. Keys: name, email, phone, gstin, address1, location, pincode. A key left out
// stays; an empty string clears it.
func (c *Client) UpdateCustomer(customerID string, fields map[string]string) (map[string]any, error) {
	body := map[string]any{}
	for k, v := range fields {
		body[k] = v
	}
	var out map[string]any
	err := c.do("PATCH", "/v1/customers/"+customerID, body, "", &out)
	return out, err
}

// EinvoiceDraft returns a DRAFT e-invoice (IRP schema 1.1) for a B2B tax invoice. Not submitted, no IRN, not checked against the portal.
func (c *Client) EinvoiceDraft(invoiceID string) (map[string]any, error) {
	var out map[string]any
	err := c.do("GET", "/v1/invoices/"+invoiceID+"/einvoice", nil, "", &out)
	return out, err
}

// CreateMandate asks the payer to approve a UPI AutoPay mandate. frequency is daily, weekly, fortnightly, monthly,
// bimonthly, quarterly, half_yearly, yearly or as_presented; endDate is YYYY-MM-DD. idempotencyKey "" makes a new one.
func (c *Client) CreateMandate(customerID string, maxAmountMinor int64, frequency, endDate, idempotencyKey string) (map[string]any, error) {
	if maxAmountMinor <= 0 {
		return nil, fmt.Errorf("maxAmountMinor must be a positive integer (paise)")
	}
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	var out map[string]any
	err := c.do("POST", "/v1/mandates", map[string]any{"customer_id": customerID, "max_amount_minor": maxAmountMinor, "frequency": frequency, "end_date": endDate}, idempotencyKey, &out)
	return out, err
}

// GetMandate returns a mandate with its recent debits.
func (c *Client) GetMandate(id string) (map[string]any, error) {
	var out map[string]any
	err := c.do("GET", "/v1/mandates/"+id, nil, "", &out)
	return out, err
}

// SimulatePaymentEvent (test organisations only) settles a pending simulated payment as "captured" or "failed",
// like a provider's late update.
func (c *Client) SimulatePaymentEvent(attemptID, outcome string) (map[string]any, error) {
	if outcome != "captured" && outcome != "failed" {
		return nil, fmt.Errorf("outcome must be captured or failed")
	}
	var out map[string]any
	err := c.do("POST", "/v1/payment_attempts/"+attemptID+"/simulate_event", map[string]any{"outcome": outcome}, "", &out)
	return out, err
}

// SendTestEvent queues a webhook.test event to the webhook URL set in Settings (admin key).
func (c *Client) SendTestEvent() (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/outbox/test", map[string]any{}, "", &out)
	return out, err
}

// SyncSettlements pulls a provider's settlement file for a day (YYYY-MM-DD). On a test organisation, provider "sim"
// builds one from the simulated payments.
func (c *Client) SyncSettlements(provider, date string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/settlements/sync", map[string]any{"provider": provider, "date": date}, "", &out)
	return out, err
}

// SimulateBankCredit (test organisations only) posts a simulated bank credit for a simulated settlement,
// shortMinor less than due when positive.
func (c *Client) SimulateBankCredit(id string, shortMinor int64) (map[string]any, error) {
	if shortMinor < 0 {
		return nil, fmt.Errorf("shortMinor cannot be negative")
	}
	var out map[string]any
	err := c.do("POST", "/v1/settlements/"+id+"/simulate_bank", map[string]any{"short_minor": shortMinor}, "", &out)
	return out, err
}

// MandateAction is "refresh" (ask the provider where it stands), "pause", "resume" or "revoke".
func (c *Client) MandateAction(id, action string) (map[string]any, error) {
	switch action {
	case "refresh", "pause", "resume", "revoke":
	default:
		return nil, fmt.Errorf("action must be refresh, pause, resume or revoke")
	}
	var out map[string]any
	err := c.do("POST", "/v1/mandates/"+id+"/"+action, map[string]any{}, "", &out)
	return out, err
}

// ScheduleDebit schedules a debit for dueDate (YYYY-MM-DD, India time), far enough ahead for the payer's notice.
func (c *Client) ScheduleDebit(mandateID string, amountMinor int64, dueDate, idempotencyKey string) (map[string]any, error) {
	if amountMinor <= 0 {
		return nil, fmt.Errorf("amountMinor must be a positive integer (paise)")
	}
	if idempotencyKey == "" {
		idempotencyKey = newKey()
	}
	var out map[string]any
	err := c.do("POST", "/v1/mandates/"+mandateID+"/debits", map[string]any{"amount_minor": amountMinor, "due_date": dueDate}, idempotencyKey, &out)
	return out, err
}

// ListPayments returns payments newest first. status and q are optional; q matches a payment id exactly, or part of
// its reference or description. limit 0 uses the server default.
func (c *Client) ListPayments(status, q string, limit int) (map[string]any, error) {
	v := url.Values{}
	if status != "" {
		v.Set("status", status)
	}
	if q != "" {
		v.Set("q", q)
	}
	if limit > 0 {
		v.Set("limit", fmt.Sprint(limit))
	}
	path := "/v1/payment_intents"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	var out map[string]any
	err := c.do("GET", path, nil, "", &out)
	return out, err
}

// FxExposure values open dollar invoices at the rate when raised and at asOf ("" = today). Reporting only.
func (c *Client) FxExposure(asOf string) (map[string]any, error) {
	path := "/v1/fx/exposure"
	if asOf != "" {
		path += "?as_of=" + asOf
	}
	var out map[string]any
	err := c.do("GET", path, nil, "", &out)
	return out, err
}

func (c *Client) PromiseToPay(invoiceID string, promisedMinor int64, promisedDate string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/invoices/"+invoiceID+"/promises", map[string]any{"promised_minor": promisedMinor, "promised_date": promisedDate}, "", &out)
	return out, err
}

// DraftReminder makes a reminder for a person to review; useAI asks the optional assistant to word it.
func (c *Client) DraftReminder(invoiceID, channel string, useAI bool) (map[string]any, error) {
	body := map[string]any{}
	if channel != "" {
		body["channel"] = channel
	}
	if useAI {
		body["ai"] = true
	}
	var out map[string]any
	err := c.do("POST", "/v1/invoices/"+invoiceID+"/reminders/draft", body, "", &out)
	return out, err
}

// ApproveDraft logs the reminder and emits invoice.reminder. A non-empty message replaces the drafted text.
func (c *Client) ApproveDraft(draftID, message string) (map[string]any, error) {
	body := map[string]any{}
	if message != "" {
		body["message"] = message
	}
	var out map[string]any
	err := c.do("POST", "/v1/collections/drafts/"+draftID+"/approve", body, "", &out)
	return out, err
}

func (c *Client) RejectDraft(draftID string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/collections/drafts/"+draftID+"/reject", map[string]any{}, "", &out)
	return out, err
}

// RunDunning reminds every overdue invoice with no live promise and no recent reminder. Safe to repeat.
// Empty channel and zero days use the server defaults.
func (c *Client) RunDunning(channel string, minDaysOverdue, repeatAfterDays int) (map[string]any, error) {
	body := map[string]any{}
	if channel != "" {
		body["channel"] = channel
	}
	if minDaysOverdue > 0 {
		body["min_days_overdue"] = minDaysOverdue
	}
	if repeatAfterDays > 0 {
		body["repeat_after_days"] = repeatAfterDays
	}
	var out map[string]any
	err := c.do("POST", "/v1/collections/dunning/run", body, "", &out)
	return out, err
}

// RunDunningAsOf is RunDunning as if it were the given day (YYYY-MM-DD); test organisations only.
func (c *Client) RunDunningAsOf(asOf, channel string, minDaysOverdue, repeatAfterDays int) (map[string]any, error) {
	if _, err := time.Parse("2006-01-02", asOf); err != nil {
		return nil, fmt.Errorf("asOf must be YYYY-MM-DD")
	}
	body := map[string]any{"as_of": asOf}
	if channel != "" {
		body["channel"] = channel
	}
	if minDaysOverdue > 0 {
		body["min_days_overdue"] = minDaysOverdue
	}
	if repeatAfterDays > 0 {
		body["repeat_after_days"] = repeatAfterDays
	}
	var out map[string]any
	err := c.do("POST", "/v1/collections/dunning/run", body, "", &out)
	return out, err
}

// RunBilling invoices every subscription that is due (safe to repeat). A non-empty asOf (YYYY-MM-DD) runs as if it
// were that day; test organisations only.
func (c *Client) RunBilling(asOf string) (map[string]any, error) {
	body := map[string]any{}
	if asOf != "" {
		if _, err := time.Parse("2006-01-02", asOf); err != nil {
			return nil, fmt.Errorf("asOf must be YYYY-MM-DD")
		}
		body["as_of"] = asOf
	}
	var out map[string]any
	err := c.do("POST", "/v1/billing/run", body, "", &out)
	return out, err
}

// CreatePaymentLink makes one URL many payers can open. Pass amountMinor or invoiceID (the other zero/empty).
func (c *Client) CreatePaymentLink(amountMinor int64, invoiceID, description string, maxUses int) (map[string]any, error) {
	if (amountMinor > 0) == (invoiceID != "") {
		return nil, fmt.Errorf("give exactly one of amountMinor or invoiceID")
	}
	body := map[string]any{}
	if amountMinor > 0 {
		body["amount_minor"] = amountMinor
	}
	if invoiceID != "" {
		body["invoice_id"] = invoiceID
	}
	if description != "" {
		body["description"] = description
	}
	if maxUses > 0 {
		body["max_uses"] = maxUses
	}
	var out map[string]any
	err := c.do("POST", "/v1/payment_links", body, "", &out)
	return out, err
}

func (c *Client) CancelPaymentLink(id string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/payment_links/"+id+"/cancel", map[string]any{}, "", &out)
	return out, err
}

// RotateKey mints a replacement; the old key keeps working for overlapHours (0 = server default of 24).
func (c *Client) RotateKey(keyID string, overlapHours int) (map[string]any, error) {
	body := map[string]any{}
	if overlapHours > 0 {
		body["overlap_hours"] = overlapHours
	}
	var out map[string]any
	err := c.do("POST", "/v1/api_keys/"+keyID+"/rotate", body, "", &out)
	return out, err
}

// call is the shared shape of the merchant-operations methods below.
func (c *Client) call(method, path string, body any) (map[string]any, error) {
	if body == nil {
		body = map[string]any{}
	}
	var out map[string]any
	err := c.do(method, path, body, "", &out)
	return out, err
}

// PaymentTimeline is one ordered story of the payment with a plain sentence on where it stands.
func (c *Client) PaymentTimeline(intentID string) (map[string]any, error) {
	return c.call("GET", "/v1/payment_intents/"+intentID+"/timeline", nil)
}

// Onboarding returns the merchant's onboarding status, documents and bank accounts (PAN masked).
func (c *Client) Onboarding() (map[string]any, error) { return c.call("GET", "/v1/onboarding", nil) }

// SaveProfile stores the merchant profile (draft or rejected onboarding only). gstin may be empty.
func (c *Client) SaveProfile(legalName, businessType, pan, address, gstin string) (map[string]any, error) {
	b := map[string]any{"legal_name": legalName, "business_type": businessType, "pan": pan, "address": address}
	if gstin != "" {
		b["gstin"] = gstin
	}
	return c.call("PUT", "/v1/onboarding", b)
}

// RequestProfileChange asks Zyvor to change business details that were already approved. Nothing changes, and live
// payouts are held, until a Zyvor reviewer decides; one request can wait at a time. The PAN is checked and only its
// last four digits are kept. gstin may be empty.
func (c *Client) RequestProfileChange(legalName, businessType, pan, address, gstin, reason string) (map[string]any, error) {
	b := map[string]any{"legal_name": legalName, "business_type": businessType, "pan": pan, "address": address, "reason": reason}
	if gstin != "" {
		b["gstin"] = gstin
	}
	return c.call("POST", "/v1/onboarding/profile_change", b)
}

// AddOnboardingDocument records where a document is kept; Zyvor stores only the reference.
func (c *Client) AddOnboardingDocument(kind, reference string) (map[string]any, error) {
	return c.call("POST", "/v1/onboarding/documents", map[string]any{"kind": kind, "reference": reference})
}

// AddBankAccount stores the holder, IFSC and the last four digits of the account number.
func (c *Client) AddBankAccount(holderName, ifsc, accountNumber string) (map[string]any, error) {
	return c.call("POST", "/v1/onboarding/bank_accounts", map[string]any{"holder_name": holderName, "ifsc": ifsc, "account_number": accountNumber})
}

// VerifyBankAccount must be done by a different person from whoever added the account.
func (c *Client) VerifyBankAccount(id string) (map[string]any, error) {
	return c.call("POST", "/v1/onboarding/bank_accounts/"+id+"/verify", nil)
}

// SubmitOnboarding sends the profile for review (needs a PAN document and a bank account).
func (c *Client) SubmitOnboarding() (map[string]any, error) {
	return c.call("POST", "/v1/onboarding/submit", nil)
}

// ReviewOnboarding approves or rejects ("approve" or "reject"; a note is required to reject).
func (c *Client) ReviewOnboarding(decision, note string) (map[string]any, error) {
	b := map[string]any{"decision": decision}
	if note != "" {
		b["note"] = note
	}
	return c.call("POST", "/v1/onboarding/review", b)
}

// CreateCounter adds a till, desk or stall to a branch.
func (c *Client) CreateCounter(branchID, name string) (map[string]any, error) {
	return c.call("POST", "/v1/counters", map[string]any{"branch_id": branchID, "name": name})
}

// DeactivateCounter switches a counter off and cancels its QR links.
func (c *Client) DeactivateCounter(id string) (map[string]any, error) {
	return c.call("POST", "/v1/counters/"+id+"/deactivate", nil)
}

// BranchSummary is captured money per IST day, counter and method, net of refunds. Dates are YYYY-MM-DD or empty.
func (c *Client) BranchSummary(branchID, from, to string) (map[string]any, error) {
	q := ""
	for _, kv := range [][2]string{{"from", from}, {"to", to}} {
		if kv[1] != "" {
			if q != "" {
				q += "&"
			}
			q += kv[0] + "=" + kv[1]
		}
	}
	if q != "" {
		q = "?" + q
	}
	return c.call("GET", "/v1/branches/"+branchID+"/summary"+q, nil)
}

// AddCash records cash taken ("sale"), handed back ("refund") or put in the till ("float"). Entries are never edited.
func (c *Client) AddCash(branchID, kind string, amountMinor int64) (map[string]any, error) {
	return c.call("POST", "/v1/cash_entries", map[string]any{"branch_id": branchID, "kind": kind, "amount_minor": amountMinor})
}

// CloseDay compares expected cash and digital totals with the counted cash for an IST day.
func (c *Client) CloseDay(branchID, day string, countedCashMinor int64) (map[string]any, error) {
	return c.call("POST", "/v1/closings", map[string]any{"branch_id": branchID, "day": day, "counted_cash_minor": countedCashMinor})
}

// SignOffClosing must be done by a different person from whoever counted.
func (c *Client) SignOffClosing(id string) (map[string]any, error) {
	return c.call("POST", "/v1/closings/"+id+"/signoff", nil)
}

// Cashflow is what is owed by due week. A schedule, not a prediction.
func (c *Client) Cashflow() (map[string]any, error) {
	return c.call("GET", "/v1/insights/cashflow", nil)
}

// VerifyWebhook checks a `Zyvor-Signature: t=<unix>,v1=<hex>` header against the RAW body.
// Delivery is at-least-once: also de-duplicate on the event id.
func VerifyWebhook(secret, header string, rawBody []byte, tolerance time.Duration, now time.Time) bool {
	var t int64
	var sig []byte
	haveT := false
	for _, p := range strings.Split(header, ",") {
		k, v, ok := strings.Cut(p, "=")
		if !ok {
			continue
		}
		switch k {
		case "t":
			n, err := strconv.ParseInt(v, 10, 64)
			if err != nil {
				return false
			}
			t, haveT = n, true
		case "v1":
			b, err := hex.DecodeString(v)
			if err != nil {
				return false
			}
			sig = b
		}
	}
	if !haveT || sig == nil {
		return false
	}
	if d := now.Sub(time.Unix(t, 0)); d > tolerance || d < -tolerance {
		return false
	}
	m := hmac.New(sha256.New, []byte(secret))
	m.Write([]byte(strconv.FormatInt(t, 10) + "."))
	m.Write(rawBody)
	return hmac.Equal(m.Sum(nil), sig)
}

// ---- more of the API, one method each (the Python SDK has the same set) ----

// GetAttempt: One payment attempt.
func (c *Client) GetAttempt(attemptId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/payment_attempts/" + attemptId
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// RefreshAttempt: Ask the provider where an attempt stands now.
func (c *Client) RefreshAttempt(attemptId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payment_attempts/"+attemptId+"/refresh", body)
}

// CancelAttempt: Cancel an attempt that has not been paid.
func (c *Client) CancelAttempt(attemptId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payment_attempts/"+attemptId+"/cancel", body)
}

// CaptureAttempt: Capture an authorised attempt (manual capture).
func (c *Client) CaptureAttempt(attemptId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payment_attempts/"+attemptId+"/capture", body)
}

// ListRefunds: Refunds, optionally by status (for example pending_approval).
func (c *Client) ListRefunds(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/refunds"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ApproveRefund: Approve a refund waiting for a second person.
func (c *Client) ApproveRefund(refundId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/refunds/"+refundId+"/approve", body)
}

// RejectRefund: Reject a refund waiting for approval.
func (c *Client) RejectRefund(refundId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/refunds/"+refundId+"/reject", body)
}

// RetryRefund: Retry a failed refund.
func (c *Client) RetryRefund(refundId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/refunds/"+refundId+"/retry", body)
}

// RefreshRefund: Ask the provider where a refund stands.
func (c *Client) RefreshRefund(refundId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/refunds/"+refundId+"/refresh", body)
}

// ListSettlements: Settlements imported from providers.
func (c *Client) ListSettlements() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/settlements"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListCases: Reconciliation cases.
func (c *Client) ListCases(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/recon/cases"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CaseSuggestions: Evidence and next steps for a case (read only).
func (c *Client) CaseSuggestions(caseId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/recon/cases/" + caseId + "/suggestions"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// AssignCase: Give a case to someone.
func (c *Client) AssignCase(caseId string, assignee string) (map[string]any, error) {
	body := map[string]any{}
	body["assignee"] = assignee
	return c.call("POST", "/v1/recon/cases/"+caseId+"/assign", body)
}

// ResolveCase: Close a case with what was done.
func (c *Client) ResolveCase(caseId string, resolution string) (map[string]any, error) {
	body := map[string]any{}
	body["resolution"] = resolution
	return c.call("POST", "/v1/recon/cases/"+caseId+"/resolve", body)
}

// ListPlans: Billing plans.
func (c *Client) ListPlans() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/plans"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListSubscriptions: Subscriptions.
func (c *Client) ListSubscriptions() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/subscriptions"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// PauseSubscription: Pause a subscription.
func (c *Client) PauseSubscription(subscriptionId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/subscriptions/"+subscriptionId+"/pause", body)
}

// ResumeSubscription: Resume a paused subscription.
func (c *Client) ResumeSubscription(subscriptionId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/subscriptions/"+subscriptionId+"/resume", body)
}

// CancelSubscription: Cancel a subscription.
func (c *Client) CancelSubscription(subscriptionId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/subscriptions/"+subscriptionId+"/cancel", body)
}

// ListInvoices: Invoices, optionally by status.
func (c *Client) ListInvoices(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/invoices"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// GetInvoice: One invoice.
func (c *Client) GetInvoice(invoiceId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/invoices/" + invoiceId
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateInvoice: Raise an invoice: give amount_minor, or GST lines and the server computes tax and total.
func (c *Client) CreateInvoice(number string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["number"] = number
	return c.call("POST", "/v1/invoices", body)
}

// VoidInvoice: Void an invoice nothing has been paid or credited against.
func (c *Client) VoidInvoice(invoiceId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/invoices/"+invoiceId+"/void", body)
}

// InvoiceLines: An invoice's GST lines.
func (c *Client) InvoiceLines(invoiceId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/invoices/" + invoiceId + "/lines"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListReminders: Reminders sent for an invoice.
func (c *Client) ListReminders(invoiceId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/invoices/" + invoiceId + "/reminders"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListPromises: Promises to pay, optionally by status (open, kept, broken, canceled).
func (c *Client) ListPromises(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/collections/promises"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CancelPromise: Cancel a promise to pay.
func (c *Client) CancelPromise(promiseId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/collections/promises/"+promiseId+"/cancel", body)
}

// CreateVendor: Add someone you pay out to.
func (c *Client) CreateVendor(name string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["name"] = name
	return c.call("POST", "/v1/vendors", body)
}

// ListVendors: Vendors (account numbers masked).
func (c *Client) ListVendors() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/vendors"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// VendorDetails: Full bank details of a vendor; needs payouts:approve and is audited.
func (c *Client) VendorDetails(vendorId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/vendors/" + vendorId + "/details"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// VendorBalance: What a vendor is owed from split payments and how much is free to pay out.
func (c *Client) VendorBalance(vendorId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/vendors/" + vendorId + "/balance"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreatePayout: Request a payout. Repeating the same external_ref returns the original.
func (c *Client) CreatePayout(vendorId string, amountMinor int64, purpose string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["vendor_id"] = vendorId
	body["amount_minor"] = amountMinor
	body["purpose"] = purpose
	return c.call("POST", "/v1/payouts", body)
}

// CreatePayoutsBulk: Up to 200 payouts, all created or none; each item has vendor_id, amount_minor, purpose and optionally external_ref, from_balance.
func (c *Client) CreatePayoutsBulk(payouts []any) (map[string]any, error) {
	body := map[string]any{}
	body["payouts"] = payouts
	return c.call("POST", "/v1/payouts/bulk", body)
}

// ListPayouts: Payouts, optionally by status.
func (c *Client) ListPayouts(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/payouts"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ApprovePayout: Approve a payout (a different person from the requester).
func (c *Client) ApprovePayout(payoutId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payouts/"+payoutId+"/approve", body)
}

// RejectPayout: Reject a payout waiting for approval.
func (c *Client) RejectPayout(payoutId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payouts/"+payoutId+"/reject", body)
}

// CancelPayout: Cancel a payout that has not been paid.
func (c *Client) CancelPayout(payoutId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/payouts/"+payoutId+"/cancel", body)
}

// MarkPayoutPaid: Record the bank reference once the transfer is made.
func (c *Client) MarkPayoutPaid(payoutId string, bankRef string) (map[string]any, error) {
	body := map[string]any{}
	body["bank_ref"] = bankRef
	return c.call("POST", "/v1/payouts/"+payoutId+"/mark_paid", body)
}

// ListCustomers: Customers, optionally by your own reference.
func (c *Client) ListCustomers(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/customers"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// GetCustomer: One customer.
func (c *Client) GetCustomer(customerId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/customers/" + customerId
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateCustomer: Add a customer.
func (c *Client) CreateCustomer(name string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["name"] = name
	return c.call("POST", "/v1/customers", body)
}

// RefreshMandate: Ask the provider where a mandate stands.
func (c *Client) RefreshMandate(mandateId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/mandates/"+mandateId+"/refresh", body)
}

// PauseMandate: Pause a mandate.
func (c *Client) PauseMandate(mandateId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/mandates/"+mandateId+"/pause", body)
}

// ResumeMandate: Resume a paused mandate.
func (c *Client) ResumeMandate(mandateId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/mandates/"+mandateId+"/resume", body)
}

// RevokeMandate: Revoke a mandate for good.
func (c *Client) RevokeMandate(mandateId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/mandates/"+mandateId+"/revoke", body)
}

// RunMandates: Present every mandate debit that is due.
func (c *Client) RunMandates() (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/mandates/run", body)
}

// GetSettings: Organisation settings.
func (c *Client) GetSettings() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/settings"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateCoupon: A coupon: kind percent takes value in basis points (1000 = 10%), flat in minor units.
func (c *Client) CreateCoupon(code string, kind string, value int64, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["code"] = code
	body["kind"] = kind
	body["value"] = value
	return c.call("POST", "/v1/coupons", body)
}

// ListCoupons: Coupons.
func (c *Client) ListCoupons() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/coupons"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// DeactivateCoupon: Stop a coupon being redeemed.
func (c *Client) DeactivateCoupon(couponId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/coupons/"+couponId+"/deactivate", body)
}

// ListMilestones: Milestones.
func (c *Client) ListMilestones() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/milestones"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateMilestone: A milestone that raises an invoice when completed.
func (c *Client) CreateMilestone(customerId string, name string, amountMinor int64, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["customer_id"] = customerId
	body["name"] = name
	body["amount_minor"] = amountMinor
	return c.call("POST", "/v1/milestones", body)
}

// CancelMilestone: Cancel a milestone that is not complete.
func (c *Client) CancelMilestone(milestoneId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/milestones/"+milestoneId+"/cancel", body)
}

// CreateInstallmentPlan: Split an agreed total into dated invoices; idempotent by external_ref.
func (c *Client) CreateInstallmentPlan(customerId string, name string, totalMinor int64, instalments int64, firstDueDate string, numberPrefix string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["customer_id"] = customerId
	body["name"] = name
	body["total_minor"] = totalMinor
	body["instalments"] = instalments
	body["first_due_date"] = firstDueDate
	body["number_prefix"] = numberPrefix
	return c.call("POST", "/v1/installment_plans", body)
}

// GetInstallmentPlan: One instalment plan.
func (c *Client) GetInstallmentPlan(planId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/installment_plans/" + planId
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// OpenDispute: Open a dispute; it holds the amount so it cannot also be refunded.
func (c *Client) OpenDispute(attemptId string, reason string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["attempt_id"] = attemptId
	body["reason"] = reason
	return c.call("POST", "/v1/disputes", body)
}

// ListDisputes: Disputes, optionally by status (open, under_review, won, lost).
func (c *Client) ListDisputes(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/disputes"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// AddDisputeEvidence: Attach evidence to a dispute.
func (c *Client) AddDisputeEvidence(disputeId string, note string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["note"] = note
	return c.call("POST", "/v1/disputes/"+disputeId+"/evidence", body)
}

// ResolveDispute: Record the outcome, won or lost.
func (c *Client) ResolveDispute(disputeId string, outcome string) (map[string]any, error) {
	body := map[string]any{}
	body["outcome"] = outcome
	return c.call("POST", "/v1/disputes/"+disputeId+"/resolve", body)
}

// ListBranches: Branches.
func (c *Client) ListBranches() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/branches"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateBranch: Add a branch.
func (c *Client) CreateBranch(name string) (map[string]any, error) {
	body := map[string]any{}
	body["name"] = name
	return c.call("POST", "/v1/branches", body)
}

// VerifyAudit: Check the audit trail's seals are unbroken.
func (c *Client) VerifyAudit() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/audit_events/verify"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// SealAudit: Seal the audit events so far.
func (c *Client) SealAudit() (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/audit_events/seal", body)
}

// ListKeys: API keys (never the secret).
func (c *Client) ListKeys() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/api_keys"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreateKey: Mint an API key; the secret is shown once.
func (c *Client) CreateKey(name string, scopes []any, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["name"] = name
	body["scopes"] = scopes
	return c.call("POST", "/v1/api_keys", body)
}

// RevokeKey: Revoke an API key.
func (c *Client) RevokeKey(keyId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/api_keys/"+keyId+"/revoke", body)
}

// ListProviders: Connected payment providers.
func (c *Client) ListProviders() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/provider_accounts"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CheckProvider: Test a provider's credentials.
func (c *Client) CheckProvider(provider string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/provider_accounts/"+provider+"/check", body)
}

// SetProviderEnabled: Turn a provider on or off for routing.
func (c *Client) SetProviderEnabled(provider string, enabled bool) (map[string]any, error) {
	body := map[string]any{}
	body["enabled"] = enabled
	return c.call("POST", "/v1/provider_accounts/"+provider+"/enabled", body)
}

// LiveAccessStatus: Where your request for live mode stands.
func (c *Client) LiveAccessStatus() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/live_access"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// RequestLiveAccess: Ask Zyvor to open live mode (the onboarding review must be approved).
func (c *Client) RequestLiveAccess() (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/live_access", body)
}

// EventCatalogue: Every webhook event type and what it means.
func (c *Client) EventCatalogue() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/events/catalogue"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListOutbox: Webhook events and their delivery state.
func (c *Client) ListOutbox(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/outbox"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ReplayEvent: Queue a webhook event again.
func (c *Client) ReplayEvent(eventId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/outbox/"+eventId+"/replay", body)
}

// GetIntent: One payment with its attempts.
func (c *Client) GetIntent(intentId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/payment_intents/" + intentId
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ExplainSettlement: A settlement explained in words and amounts.
func (c *Client) ExplainSettlement(settlementId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/settlements/" + settlementId + "/explain"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CreatePlan: A billing plan.
func (c *Client) CreatePlan(name string, amountMinor int64, intervalUnit string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["name"] = name
	body["amount_minor"] = amountMinor
	body["interval_unit"] = intervalUnit
	return c.call("POST", "/v1/plans", body)
}

// Statement: A customer's statement.
func (c *Client) Statement(customerId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/customers/" + customerId + "/statement"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// CompleteMilestone: Complete a milestone; it raises its invoice.
func (c *Client) CompleteMilestone(milestoneId string) (map[string]any, error) {
	body := map[string]any{}
	return c.call("POST", "/v1/milestones/"+milestoneId+"/complete", body)
}

// FxRates: Dollar rates you have set.
func (c *Client) FxRates() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/fx/rates"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// FxConvert: Convert an amount at the rate for a day.
func (c *Client) FxConvert(amountMinor int64, fromCurrency string, toCurrency string, opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	v.Set("amount_minor", fmt.Sprint(amountMinor))
	v.Set("from", fmt.Sprint(fromCurrency))
	v.Set("to", fmt.Sprint(toCurrency))
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/fx/convert"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListMandates: Mandates.
func (c *Client) ListMandates(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/mandates"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListMandateDebits: A mandate's debits.
func (c *Client) ListMandateDebits(mandateId string) (map[string]any, error) {
	v := url.Values{}
	path := "/v1/mandates/" + mandateId + "/debits"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListPaymentLinks: Payment links.
func (c *Client) ListPaymentLinks() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/payment_links"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// Remind: Record a reminder you sent for an invoice.
func (c *Client) Remind(invoiceId string, channel string, opts map[string]any) (map[string]any, error) {
	body := map[string]any{}
	for k, x := range opts {
		body[k] = x
	}
	body["channel"] = channel
	return c.call("POST", "/v1/invoices/"+invoiceId+"/reminders", body)
}

// ReminderDrafts: Reminder drafts waiting for a person.
func (c *Client) ReminderDrafts(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/collections/drafts"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// ListCounters: Counters, optionally for one branch.
func (c *Client) ListCounters(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/counters"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// Closings: Day closings.
func (c *Client) Closings(opts map[string]any) (map[string]any, error) {
	v := url.Values{}
	for k, x := range opts {
		v.Set(k, fmt.Sprint(x))
	}
	path := "/v1/closings"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// RefundAnomalies: Unusual refund activity (flags only; nothing is blocked).
func (c *Client) RefundAnomalies() (map[string]any, error) {
	v := url.Values{}
	path := "/v1/insights/refund_anomalies"
	if len(v) > 0 {
		path += "?" + v.Encode()
	}
	return c.call("GET", path, nil)
}

// UpdateSettings changes organisation settings; send only the fields to change (see GetSettings for the shape).
func (c *Client) UpdateSettings(patch map[string]any) (map[string]any, error) {
	return c.call("POST", "/v1/settings", patch)
}

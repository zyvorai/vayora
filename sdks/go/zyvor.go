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

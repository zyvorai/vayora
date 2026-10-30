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

func (c *Client) PromiseToPay(invoiceID string, promisedMinor int64, promisedDate string) (map[string]any, error) {
	var out map[string]any
	err := c.do("POST", "/v1/invoices/"+invoiceID+"/promises", map[string]any{"promised_minor": promisedMinor, "promised_date": promisedDate}, "", &out)
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

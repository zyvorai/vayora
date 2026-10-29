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

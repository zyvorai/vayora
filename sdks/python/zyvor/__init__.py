"""Zyvor Payments Python SDK (stdlib only). Amounts are integer minor units (paise)."""
from __future__ import annotations

import hashlib
import hmac
import json
import time
import urllib.error
import urllib.request
import uuid
from typing import Any, Optional

__all__ = ["Zyvor", "ZyvorError", "verify_webhook"]


class ZyvorError(Exception):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(f"{status} {code}: {message}")
        self.status, self.code, self.message = status, code, message


class Zyvor:
    def __init__(self, api_key: str, base_url: str = "http://localhost:8080", actor: Optional[str] = None, opener=None):
        self.api_key, self.base, self.actor = api_key, base_url.rstrip("/"), actor
        self._open = opener or urllib.request.urlopen

    def _req(self, method: str, path: str, body: Any = None, idem: Optional[str] = None) -> Any:
        headers = {"authorization": f"Bearer {self.api_key}"}
        data = None
        if body is not None:
            data, headers["content-type"] = json.dumps(body).encode(), "application/json"
        if idem:
            headers["idempotency-key"] = idem
        if self.actor:
            headers["zyvor-actor"] = self.actor
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=headers)
        try:
            with self._open(req, timeout=30) as res:
                return json.loads(res.read() or b"null")
        except urllib.error.HTTPError as e:
            payload = json.loads(e.read() or b"{}").get("error", {})
            raise ZyvorError(e.code, payload.get("code", "unknown"), payload.get("message", e.reason)) from None

    def create_intent(self, amount_minor: int, currency: str = "INR", idempotency_key: Optional[str] = None, **extra) -> dict:
        if not isinstance(amount_minor, int) or isinstance(amount_minor, bool) or amount_minor <= 0:
            raise ValueError("amount_minor must be a positive integer (paise)")
        return self._req("POST", "/v1/payment_intents", {"amount_minor": amount_minor, "currency": currency, **extra}, idempotency_key or str(uuid.uuid4()))

    def get_intent(self, intent_id: str) -> dict:
        return self._req("GET", f"/v1/payment_intents/{intent_id}")

    def create_attempt(self, intent_id: str, method: str, vpa: Optional[str] = None, idempotency_key: Optional[str] = None) -> dict:
        return self._req("POST", f"/v1/payment_intents/{intent_id}/attempts", {"method": method, "vpa": vpa}, idempotency_key or str(uuid.uuid4()))

    def refund(self, intent_id: str, amount_minor: Optional[int] = None, idempotency_key: Optional[str] = None, **extra) -> dict:
        body = {k: v for k, v in {"amount_minor": amount_minor, **extra}.items() if v is not None}
        return self._req("POST", f"/v1/payment_intents/{intent_id}/refunds", body, idempotency_key or str(uuid.uuid4()))

    def explain_settlement(self, settlement_id: str) -> dict:
        return self._req("GET", f"/v1/settlements/{settlement_id}/explain")


    # ---- billing ----
    def create_plan(self, name: str, amount_minor: int, interval_unit: str, **extra) -> dict:
        return self._req("POST", "/v1/plans", {"name": name, "amount_minor": amount_minor, "interval_unit": interval_unit, **extra})

    def subscribe(self, customer_id: str, plan_id: str, quantity: int = 1, start_date: Optional[str] = None) -> dict:
        body = {k: v for k, v in {"customer_id": customer_id, "plan_id": plan_id, "quantity": quantity, "start_date": start_date}.items() if v is not None}
        return self._req("POST", "/v1/subscriptions", body)

    def change_plan(self, subscription_id: str, plan_id: str, quantity: Optional[int] = None) -> dict:
        """Upgrades are prorated for the rest of the period; downgrades apply from the next invoice."""
        body = {k: v for k, v in {"plan_id": plan_id, "quantity": quantity}.items() if v is not None}
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/change_plan", body)

    def record_usage(self, subscription_id: str, quantity: int, reference: Optional[str] = None) -> dict:
        """Retry-safe when `reference` is set: the same reference is recorded once."""
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/usage", {k: v for k, v in {"quantity": quantity, "reference": reference}.items() if v is not None})

    def run_billing(self) -> dict:
        return self._req("POST", "/v1/billing/run")

    def credit_note(self, invoice_id: str, amount_minor: int, reason: str) -> dict:
        return self._req("POST", f"/v1/invoices/{invoice_id}/credit_notes", {"amount_minor": amount_minor, "reason": reason})

    def statement(self, customer_id: str) -> dict:
        return self._req("GET", f"/v1/customers/{customer_id}/statement")

    def complete_milestone(self, milestone_id: str) -> dict:
        """Accept the milestone and invoice it; calling twice returns the same invoice."""
        return self._req("POST", f"/v1/milestones/{milestone_id}/complete")

    # ---- collections ----
    def ageing(self, as_of: Optional[str] = None) -> dict:
        return self._req("GET", "/v1/collections/ageing" + (f"?as_of={as_of}" if as_of else ""))

    def update_customer(self, customer_id: str, **fields: str) -> dict:
        """Correct a customer: name, email, phone, gstin, address1, location, pincode. Left-out fields stay; "" clears."""
        return self._req("PATCH", f"/v1/customers/{customer_id}", fields)

    def einvoice_draft(self, invoice_id: str) -> dict:
        """DRAFT e-invoice JSON (IRP schema 1.1) for a B2B tax invoice. Not submitted, no IRN, not checked against the portal."""
        return self._req("GET", f"/v1/invoices/{invoice_id}/einvoice")

    # ---- exchange rates (reporting only) ----
    def set_fx_rate(self, rate: str, currency: str = "USD", as_of: Optional[str] = None, note: Optional[str] = None) -> dict:
        """Record the INR-per-unit rate you use for a day, e.g. "83.2150". One per day; never edited."""
        return self._req("POST", "/v1/fx/rates", {k: v for k, v in {"currency": currency, "rate": rate, "as_of": as_of, "note": note}.items() if v is not None})

    def fx_rates(self) -> dict:
        return self._req("GET", "/v1/fx/rates")

    def fx_convert(self, amount_minor: int, from_currency: str, to_currency: str, as_of: Optional[str] = None) -> dict:
        return self._req("GET", f"/v1/fx/convert?amount_minor={amount_minor}&from={from_currency}&to={to_currency}" + (f"&as_of={as_of}" if as_of else ""))

    def fx_exposure(self, as_of: Optional[str] = None) -> dict:
        """Open dollar invoices at the rate when raised vs the report date. Reporting only; nothing is posted."""
        return self._req("GET", "/v1/fx/exposure" + (f"?as_of={as_of}" if as_of else ""))

    def promise_to_pay(self, invoice_id: str, promised_minor: int, promised_date: str, note: Optional[str] = None) -> dict:
        return self._req("POST", f"/v1/invoices/{invoice_id}/promises", {k: v for k, v in {"promised_minor": promised_minor, "promised_date": promised_date, "note": note}.items() if v is not None})

    def remind(self, invoice_id: str, channel: str, message: Optional[str] = None) -> dict:
        return self._req("POST", f"/v1/invoices/{invoice_id}/reminders", {k: v for k, v in {"channel": channel, "message": message}.items() if v is not None})

    def run_dunning(self, channel: Optional[str] = None, min_days_overdue: Optional[int] = None, repeat_after_days: Optional[int] = None) -> dict:
        """Remind every overdue invoice with no live promise and no recent reminder. Safe to repeat."""
        return self._req("POST", "/v1/collections/dunning/run", {k: v for k, v in {"channel": channel, "min_days_overdue": min_days_overdue, "repeat_after_days": repeat_after_days}.items() if v is not None})

    def draft_reminder(self, invoice_id: str, channel: Optional[str] = None, tone: Optional[str] = None, ai: bool = False) -> dict:
        """A reminder for a person to review. `ai=True` asks the optional assistant to word it."""
        return self._req("POST", f"/v1/invoices/{invoice_id}/reminders/draft", {k: v for k, v in {"channel": channel, "tone": tone, "ai": ai or None}.items() if v is not None})

    def reminder_drafts(self, status: Optional[str] = None) -> dict:
        return self._req("GET", "/v1/collections/drafts" + (f"?status={status}" if status else ""))

    def approve_draft(self, draft_id: str, message: Optional[str] = None) -> dict:
        """Approving logs the reminder and emits `invoice.reminder`; pass `message` to edit the text first."""
        return self._req("POST", f"/v1/collections/drafts/{draft_id}/approve", {"message": message} if message else {})

    def reject_draft(self, draft_id: str) -> dict:
        return self._req("POST", f"/v1/collections/drafts/{draft_id}/reject")

    # ---- payment links and keys ----
    def create_payment_link(self, amount_minor: Optional[int] = None, invoice_id: Optional[str] = None, **extra) -> dict:
        """One URL, many payers. Give `amount_minor` or `invoice_id`; extras: description, max_uses, expires_at, customer_id, currency."""
        body = {k: v for k, v in {"amount_minor": amount_minor, "invoice_id": invoice_id, **extra}.items() if v is not None}
        return self._req("POST", "/v1/payment_links", body)

    def list_payment_links(self) -> dict:
        return self._req("GET", "/v1/payment_links")

    def cancel_payment_link(self, link_id: str) -> dict:
        return self._req("POST", f"/v1/payment_links/{link_id}/cancel")

    # --- merchant operations -------------------------------------------------------------------
    def payment_timeline(self, intent_id: str) -> dict:
        """One ordered story of the payment, with a plain sentence on where it stands."""
        return self._req("GET", f"/v1/payment_intents/{intent_id}/timeline")

    def onboarding(self) -> dict:
        return self._req("GET", "/v1/onboarding")

    def save_profile(self, legal_name: str, business_type: str, pan: str, address: str, gstin: Optional[str] = None) -> dict:
        return self._req("PUT", "/v1/onboarding", {"legal_name": legal_name, "business_type": business_type, "pan": pan, "address": address, "gstin": gstin})

    def add_onboarding_document(self, kind: str, reference: str) -> dict:
        return self._req("POST", "/v1/onboarding/documents", {"kind": kind, "reference": reference})

    def add_bank_account(self, holder_name: str, ifsc: str, account_number: str) -> dict:
        return self._req("POST", "/v1/onboarding/bank_accounts", {"holder_name": holder_name, "ifsc": ifsc, "account_number": account_number})

    def verify_bank_account(self, account_id: str) -> dict:
        """A different person from whoever added it."""
        return self._req("POST", f"/v1/onboarding/bank_accounts/{account_id}/verify")

    def submit_onboarding(self) -> dict:
        return self._req("POST", "/v1/onboarding/submit")

    def review_onboarding(self, decision: str, note: Optional[str] = None) -> dict:
        """`approve` or `reject` (a note is required to reject); a different person from the submitter."""
        return self._req("POST", "/v1/onboarding/review", {"decision": decision, "note": note})

    def create_counter(self, branch_id: str, name: str) -> dict:
        return self._req("POST", "/v1/counters", {"branch_id": branch_id, "name": name})

    def list_counters(self, branch_id: Optional[str] = None) -> dict:
        return self._req("GET", "/v1/counters" + (f"?branch_id={branch_id}" if branch_id else ""))

    def deactivate_counter(self, counter_id: str) -> dict:
        """Also cancels the counter's QR links."""
        return self._req("POST", f"/v1/counters/{counter_id}/deactivate")

    def branch_summary(self, branch_id: str, from_day: Optional[str] = None, to_day: Optional[str] = None) -> dict:
        q = "&".join(f"{k}={v}" for k, v in (("from", from_day), ("to", to_day)) if v)
        return self._req("GET", f"/v1/branches/{branch_id}/summary" + (f"?{q}" if q else ""))

    def add_cash(self, branch_id: str, kind: str, amount_minor: int, counter_id: Optional[str] = None, note: Optional[str] = None) -> dict:
        """`sale`, `refund` or `float`. Never edited: correct a mistake with an opposite entry."""
        return self._req("POST", "/v1/cash_entries", {"branch_id": branch_id, "counter_id": counter_id, "kind": kind, "amount_minor": amount_minor, "note": note})

    def close_day(self, branch_id: str, day: str, counted_cash_minor: int, note: Optional[str] = None) -> dict:
        return self._req("POST", "/v1/closings", {"branch_id": branch_id, "day": day, "counted_cash_minor": counted_cash_minor, "note": note})

    def closings(self, branch_id: Optional[str] = None, status: Optional[str] = None) -> dict:
        q = "&".join(f"{k}={v}" for k, v in (("branch_id", branch_id), ("status", status)) if v)
        return self._req("GET", "/v1/closings" + (f"?{q}" if q else ""))

    def sign_off_closing(self, closing_id: str) -> dict:
        """A different person from whoever counted."""
        return self._req("POST", f"/v1/closings/{closing_id}/signoff")

    def cashflow(self, weeks: Optional[int] = None) -> dict:
        """What is owed by due week. A schedule, not a prediction."""
        return self._req("GET", "/v1/insights/cashflow" + (f"?weeks={weeks}" if weeks else ""))

    def refund_anomalies(self) -> dict:
        return self._req("GET", "/v1/insights/refund_anomalies")

    def rotate_key(self, key_id: str, overlap_hours: Optional[int] = None) -> dict:
        """The old key keeps working for `overlap_hours` (default 24) so deployments can switch over."""
        return self._req("POST", f"/v1/api_keys/{key_id}/rotate", {"overlap_hours": overlap_hours} if overlap_hours is not None else {})


def verify_webhook(secret: str, header: str, raw_body: bytes, tolerance_seconds: int = 300, now: Optional[int] = None) -> bool:
    """Verify `Zyvor-Signature: t=<unix>,v1=<hex>` against the RAW body. De-duplicate on event id too."""
    parts = dict(p.split("=", 1) for p in header.split(",") if "=" in p)
    try:
        t = int(parts["t"])
        given = bytes.fromhex(parts["v1"])
    except (KeyError, ValueError):
        return False
    if abs((now if now is not None else int(time.time())) - t) > tolerance_seconds:
        return False
    expected = hmac.new(secret.encode(), f"{t}.".encode() + raw_body, hashlib.sha256).digest()
    return hmac.compare_digest(expected, given)

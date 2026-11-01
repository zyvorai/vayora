"""Zyvor Payments Python SDK (stdlib only). Amounts are integer minor units (paise)."""
from __future__ import annotations

import hashlib
import hmac
import json
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from typing import Any, Optional

from . import types

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

    def get_intent(self, intent_id: str) -> types.GetPaymentIntentsByIdResponse:
        return self._req("GET", f"/v1/payment_intents/{intent_id}")

    def create_attempt(self, intent_id: str, method: str, vpa: Optional[str] = None, idempotency_key: Optional[str] = None) -> types.PostPaymentIntentsByIdAttemptsResponse:
        return self._req("POST", f"/v1/payment_intents/{intent_id}/attempts", {"method": method, "vpa": vpa}, idempotency_key or str(uuid.uuid4()))

    def refund(self, intent_id: str, amount_minor: Optional[int] = None, idempotency_key: Optional[str] = None, **extra) -> dict:
        body = {k: v for k, v in {"amount_minor": amount_minor, **extra}.items() if v is not None}
        return self._req("POST", f"/v1/payment_intents/{intent_id}/refunds", body, idempotency_key or str(uuid.uuid4()))

    def explain_settlement(self, settlement_id: str) -> dict:
        return self._req("GET", f"/v1/settlements/{settlement_id}/explain")

    def simulate_payment_event(self, attempt_id: str, outcome: str) -> types.PostOutboxTestResponse:
        """Test organisations only: settle a pending simulated payment as "captured" or "failed", like a provider's late update."""
        if outcome not in ("captured", "failed"):
            raise ValueError("outcome must be captured or failed")
        return self._req("POST", f"/v1/payment_attempts/{attempt_id}/simulate_event", {"outcome": outcome})

    def send_test_event(self) -> types.PostOutboxTestResponse:
        """Queue a webhook.test event to the webhook URL set in Settings (admin key)."""
        return self._req("POST", "/v1/outbox/test", {})

    def sync_settlements(self, provider: str, date: str) -> types.PostSettlementsSyncResponse:
        """Pull a provider's settlement file for a day (date is YYYY-MM-DD). On a test organisation, provider "sim" builds one from simulated payments."""
        return self._req("POST", "/v1/settlements/sync", {"provider": provider, "date": date})

    def simulate_bank_credit(self, settlement_id: str, short_minor: int = 0) -> types.PostSubscriptionsByIdUsageResponse:
        """Test organisations only: post a simulated bank credit for a simulated settlement, optionally short_minor less than due."""
        if short_minor < 0:
            raise ValueError("short_minor cannot be negative")
        return self._req("POST", f"/v1/settlements/{settlement_id}/simulate_bank", {"short_minor": short_minor})


    # ---- billing ----
    def create_plan(self, name: str, amount_minor: int, interval_unit: str, **extra) -> types.PostPlansResponse:
        return self._req("POST", "/v1/plans", {"name": name, "amount_minor": amount_minor, "interval_unit": interval_unit, **extra})

    def subscribe(self, customer_id: str, plan_id: str, quantity: int = 1, start_date: Optional[str] = None) -> dict:
        body = {k: v for k, v in {"customer_id": customer_id, "plan_id": plan_id, "quantity": quantity, "start_date": start_date}.items() if v is not None}
        return self._req("POST", "/v1/subscriptions", body)

    def change_plan(self, subscription_id: str, plan_id: str, quantity: Optional[int] = None) -> types.PostSubscriptionsByIdUsageResponse:
        """Upgrades are prorated for the rest of the period; downgrades apply from the next invoice."""
        body = {k: v for k, v in {"plan_id": plan_id, "quantity": quantity}.items() if v is not None}
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/change_plan", body)

    def record_usage(self, subscription_id: str, quantity: int, reference: Optional[str] = None) -> types.PostSubscriptionsByIdUsageResponse:
        """Retry-safe when `reference` is set: the same reference is recorded once."""
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/usage", {k: v for k, v in {"quantity": quantity, "reference": reference}.items() if v is not None})

    def run_billing(self, as_of: Optional[str] = None) -> types.PostBillingRunResponse:
        """Invoice every subscription that is due. as_of (YYYY-MM-DD) runs as if it were that day; test mode only."""
        return self._req("POST", "/v1/billing/run", {"as_of": as_of} if as_of else {})

    def credit_note(self, invoice_id: str, amount_minor: int, reason: str) -> types.PostInvoicesByIdCreditNotesResponse:
        return self._req("POST", f"/v1/invoices/{invoice_id}/credit_notes", {"amount_minor": amount_minor, "reason": reason})

    def statement(self, customer_id: str) -> types.GetCustomersByIdStatementResponse:
        return self._req("GET", f"/v1/customers/{customer_id}/statement")

    def complete_milestone(self, milestone_id: str) -> types.PostMilestonesByIdCompleteResponse:
        """Accept the milestone and invoice it; calling twice returns the same invoice."""
        return self._req("POST", f"/v1/milestones/{milestone_id}/complete")

    # ---- collections ----
    def ageing(self, as_of: Optional[str] = None) -> types.GetCollectionsAgeingResponse:
        return self._req("GET", "/v1/collections/ageing" + (f"?as_of={as_of}" if as_of else ""))

    def update_customer(self, customer_id: str, **fields: str) -> dict:
        """Correct a customer: name, email, phone, gstin, address1, location, pincode. Left-out fields stay; "" clears."""
        return self._req("PATCH", f"/v1/customers/{customer_id}", fields)

    def einvoice_draft(self, invoice_id: str) -> types.GetInvoicesByIdEinvoiceResponse:
        """DRAFT e-invoice JSON (IRP schema 1.1) for a B2B tax invoice. Not submitted, no IRN, not checked against the portal."""
        return self._req("GET", f"/v1/invoices/{invoice_id}/einvoice")

    # ---- exchange rates (reporting only) ----
    def set_fx_rate(self, rate: str, currency: str = "USD", as_of: Optional[str] = None, note: Optional[str] = None) -> types.PostFxRatesResponse:
        """Record the INR-per-unit rate you use for a day, e.g. "83.2150". One per day; never edited."""
        return self._req("POST", "/v1/fx/rates", {k: v for k, v in {"currency": currency, "rate": rate, "as_of": as_of, "note": note}.items() if v is not None})

    def fx_rates(self) -> types.GetFxRatesResponse:
        return self._req("GET", "/v1/fx/rates")

    def fx_convert(self, amount_minor: int, from_currency: str, to_currency: str, as_of: Optional[str] = None) -> types.GetFxConvertResponse:
        return self._req("GET", f"/v1/fx/convert?amount_minor={amount_minor}&from={from_currency}&to={to_currency}" + (f"&as_of={as_of}" if as_of else ""))

    def fx_exposure(self, as_of: Optional[str] = None) -> types.GetFxExposureResponse:
        """Open dollar invoices at the rate when raised vs the report date. Reporting only; nothing is posted."""
        return self._req("GET", "/v1/fx/exposure" + (f"?as_of={as_of}" if as_of else ""))

    def promise_to_pay(self, invoice_id: str, promised_minor: int, promised_date: str, note: Optional[str] = None) -> types.PostInvoicesByIdPromisesResponse:
        return self._req("POST", f"/v1/invoices/{invoice_id}/promises", {k: v for k, v in {"promised_minor": promised_minor, "promised_date": promised_date, "note": note}.items() if v is not None})

    def remind(self, invoice_id: str, channel: str, message: Optional[str] = None) -> types.PostInvoicesByIdRemindersResponse:
        return self._req("POST", f"/v1/invoices/{invoice_id}/reminders", {k: v for k, v in {"channel": channel, "message": message}.items() if v is not None})

    def run_dunning(self, channel: Optional[str] = None, min_days_overdue: Optional[int] = None, repeat_after_days: Optional[int] = None, as_of: Optional[str] = None) -> types.PostCollectionsDunningRunResponse:
        """Remind every overdue invoice with no live promise and no recent reminder. Safe to repeat."""
        return self._req("POST", "/v1/collections/dunning/run", {k: v for k, v in {"channel": channel, "min_days_overdue": min_days_overdue, "repeat_after_days": repeat_after_days, "as_of": as_of}.items() if v is not None})

    def draft_reminder(self, invoice_id: str, channel: Optional[str] = None, tone: Optional[str] = None, ai: bool = False) -> types.PostInvoicesByIdRemindersDraftResponse:
        """A reminder for a person to review. `ai=True` asks the optional assistant to word it."""
        return self._req("POST", f"/v1/invoices/{invoice_id}/reminders/draft", {k: v for k, v in {"channel": channel, "tone": tone, "ai": ai or None}.items() if v is not None})

    def reminder_drafts(self, status: Optional[str] = None) -> types.GetCollectionsDraftsResponse:
        return self._req("GET", "/v1/collections/drafts" + (f"?status={status}" if status else ""))

    def approve_draft(self, draft_id: str, message: Optional[str] = None) -> types.PostCollectionsDraftsByIdApproveResponse:
        """Approving logs the reminder and emits `invoice.reminder`; pass `message` to edit the text first."""
        return self._req("POST", f"/v1/collections/drafts/{draft_id}/approve", {"message": message} if message else {})

    def reject_draft(self, draft_id: str) -> types.PostCollectionsDraftsByIdRejectResponse:
        return self._req("POST", f"/v1/collections/drafts/{draft_id}/reject")

    # ---- payment links and keys ----
    def create_payment_link(self, amount_minor: Optional[int] = None, invoice_id: Optional[str] = None, **extra) -> types.GetPaymentIntentsByIdTimelineResponse:
        """One URL, many payers. Give `amount_minor` or `invoice_id`; extras: description, max_uses, expires_at, customer_id, currency."""
        body = {k: v for k, v in {"amount_minor": amount_minor, "invoice_id": invoice_id, **extra}.items() if v is not None}
        return self._req("POST", "/v1/payment_links", body)

    def list_payment_links(self) -> types.GetPaymentLinksResponse:
        return self._req("GET", "/v1/payment_links")

    def cancel_payment_link(self, link_id: str) -> types.PostPaymentLinksByIdCancelResponse:
        return self._req("POST", f"/v1/payment_links/{link_id}/cancel")

    def list_payments(self, status: Optional[str] = None, q: Optional[str] = None, limit: Optional[int] = None, starting_after: Optional[str] = None) -> types.GetPaymentIntentsByIdTimelineResponse:
        """Newest first. `q` matches a payment id exactly, or part of its reference or description."""
        params = {k: v for k, v in {"status": status, "q": q, "limit": limit, "starting_after": starting_after}.items() if v not in (None, "")}
        return self._req("GET", "/v1/payment_intents" + ("?" + urllib.parse.urlencode(params) if params else ""))

    # --- UPI AutoPay mandates ------------------------------------------------------------------
    def create_mandate(self, customer_id: str, max_amount_minor: int, frequency: str, end_date: str, idempotency_key: Optional[str] = None, **extra) -> types.GetPaymentIntentsByIdTimelineResponse:
        """The payer approves once. `frequency`: daily, weekly, fortnightly, monthly, bimonthly, quarterly, half_yearly, yearly or as_presented. Extras: start_date, provider, external_ref."""
        if not isinstance(max_amount_minor, int) or isinstance(max_amount_minor, bool) or max_amount_minor <= 0:
            raise ValueError("max_amount_minor must be a positive integer (paise)")
        body = {k: v for k, v in {"customer_id": customer_id, "max_amount_minor": max_amount_minor, "frequency": frequency, "end_date": end_date, **extra}.items() if v is not None}
        return self._req("POST", "/v1/mandates", body, idempotency_key or str(uuid.uuid4()))

    def list_mandates(self, status: Optional[str] = None, customer_id: Optional[str] = None) -> dict:
        params = {k: v for k, v in {"status": status, "customer_id": customer_id}.items() if v}
        return self._req("GET", "/v1/mandates" + ("?" + urllib.parse.urlencode(params) if params else ""))

    def get_mandate(self, mandate_id: str) -> types.GetMandatesByIdResponse:
        return self._req("GET", f"/v1/mandates/{mandate_id}")

    def mandate_action(self, mandate_id: str, action: str) -> types.GetPaymentIntentsByIdTimelineResponse:
        """`action`: refresh (ask the provider), pause, resume or revoke."""
        if action not in ("refresh", "pause", "resume", "revoke"):
            raise ValueError("action must be refresh, pause, resume or revoke")
        return self._req("POST", f"/v1/mandates/{mandate_id}/{action}", {})

    def schedule_debit(self, mandate_id: str, amount_minor: int, due_date: str, invoice_id: Optional[str] = None, idempotency_key: Optional[str] = None) -> types.GetPaymentIntentsByIdTimelineResponse:
        """`due_date` is an India-time day far enough ahead for the payer's notice."""
        if not isinstance(amount_minor, int) or isinstance(amount_minor, bool) or amount_minor <= 0:
            raise ValueError("amount_minor must be a positive integer (paise)")
        body = {k: v for k, v in {"amount_minor": amount_minor, "due_date": due_date, "invoice_id": invoice_id}.items() if v is not None}
        return self._req("POST", f"/v1/mandates/{mandate_id}/debits", body, idempotency_key or str(uuid.uuid4()))

    def list_mandate_debits(self, mandate_id: str) -> types.GetMandatesByIdDebitsResponse:
        return self._req("GET", f"/v1/mandates/{mandate_id}/debits")

    # --- merchant operations -------------------------------------------------------------------
    def payment_timeline(self, intent_id: str) -> types.GetPaymentIntentsByIdTimelineResponse:
        """One ordered story of the payment, with a plain sentence on where it stands."""
        return self._req("GET", f"/v1/payment_intents/{intent_id}/timeline")

    def onboarding(self) -> types.GetOnboardingResponse:
        return self._req("GET", "/v1/onboarding")

    def save_profile(self, legal_name: str, business_type: str, pan: str, address: str, gstin: Optional[str] = None) -> types.PutOnboardingResponse:
        return self._req("PUT", "/v1/onboarding", {"legal_name": legal_name, "business_type": business_type, "pan": pan, "address": address, "gstin": gstin})

    def request_profile_change(self, legal_name: str, business_type: str, pan: str, address: str, reason: str, gstin: Optional[str] = None) -> types.PostOnboardingProfileChangeResponse:
        """After approval the details are locked. This asks Zyvor to change them: nothing changes, and live payouts are held,
        until a Zyvor reviewer decides. One request can wait at a time. The PAN is checked and only its last four digits are kept."""
        return self._req("POST", "/v1/onboarding/profile_change", {"legal_name": legal_name, "business_type": business_type, "pan": pan, "address": address, "gstin": gstin, "reason": reason})

    def add_onboarding_document(self, kind: str, reference: str) -> types.PostOnboardingDocumentsResponse:
        return self._req("POST", "/v1/onboarding/documents", {"kind": kind, "reference": reference})

    def add_bank_account(self, holder_name: str, ifsc: str, account_number: str) -> types.PostOnboardingBankAccountsResponse:
        return self._req("POST", "/v1/onboarding/bank_accounts", {"holder_name": holder_name, "ifsc": ifsc, "account_number": account_number})

    def verify_bank_account(self, account_id: str) -> types.PostOnboardingBankAccountsByIdVerifyResponse:
        """A different person from whoever added it."""
        return self._req("POST", f"/v1/onboarding/bank_accounts/{account_id}/verify")

    def submit_onboarding(self) -> types.PostOnboardingSubmitResponse:
        return self._req("POST", "/v1/onboarding/submit")

    def review_onboarding(self, decision: str, note: Optional[str] = None) -> types.PostOnboardingReviewResponse:
        """`approve` or `reject` (a note is required to reject); a different person from the submitter."""
        return self._req("POST", "/v1/onboarding/review", {"decision": decision, "note": note})

    def create_counter(self, branch_id: str, name: str) -> types.PostCountersResponse:
        return self._req("POST", "/v1/counters", {"branch_id": branch_id, "name": name})

    def list_counters(self, branch_id: Optional[str] = None) -> types.GetCountersResponse:
        return self._req("GET", "/v1/counters" + (f"?branch_id={branch_id}" if branch_id else ""))

    def deactivate_counter(self, counter_id: str) -> types.PostCountersByIdDeactivateResponse:
        """Also cancels the counter's QR links."""
        return self._req("POST", f"/v1/counters/{counter_id}/deactivate")

    def branch_summary(self, branch_id: str, from_day: Optional[str] = None, to_day: Optional[str] = None) -> dict:
        q = "&".join(f"{k}={v}" for k, v in (("from", from_day), ("to", to_day)) if v)
        return self._req("GET", f"/v1/branches/{branch_id}/summary" + (f"?{q}" if q else ""))

    def add_cash(self, branch_id: str, kind: str, amount_minor: int, counter_id: Optional[str] = None, note: Optional[str] = None) -> types.PostCashEntriesResponse:
        """`sale`, `refund` or `float`. Never edited: correct a mistake with an opposite entry."""
        return self._req("POST", "/v1/cash_entries", {"branch_id": branch_id, "counter_id": counter_id, "kind": kind, "amount_minor": amount_minor, "note": note})

    def close_day(self, branch_id: str, day: str, counted_cash_minor: int, note: Optional[str] = None) -> types.PostClosingsResponse:
        return self._req("POST", "/v1/closings", {"branch_id": branch_id, "day": day, "counted_cash_minor": counted_cash_minor, "note": note})

    def closings(self, branch_id: Optional[str] = None, status: Optional[str] = None) -> dict:
        q = "&".join(f"{k}={v}" for k, v in (("branch_id", branch_id), ("status", status)) if v)
        return self._req("GET", "/v1/closings" + (f"?{q}" if q else ""))

    def sign_off_closing(self, closing_id: str) -> types.PostClosingsByIdSignoffResponse:
        """A different person from whoever counted."""
        return self._req("POST", f"/v1/closings/{closing_id}/signoff")

    def cashflow(self, weeks: Optional[int] = None) -> types.GetInsightsCashflowResponse:
        """What is owed by due week. A schedule, not a prediction."""
        return self._req("GET", "/v1/insights/cashflow" + (f"?weeks={weeks}" if weeks else ""))

    def refund_anomalies(self) -> types.GetInsightsRefundAnomaliesResponse:
        return self._req("GET", "/v1/insights/refund_anomalies")

    def rotate_key(self, key_id: str, overlap_hours: Optional[int] = None) -> types.PostApiKeysByIdRotateResponse:
        """The old key keeps working for `overlap_hours` (default 24) so deployments can switch over."""
        return self._req("POST", f"/v1/api_keys/{key_id}/rotate", {"overlap_hours": overlap_hours} if overlap_hours is not None else {})

    # ---- more of the API, one method each (generated from one table so the Python and Go SDKs agree) ----
    def get_attempt(self, attempt_id: str) -> types.GetPaymentAttemptsByIdResponse:
        """One payment attempt."""
        return self._req("GET", f"/v1/payment_attempts/{attempt_id}")

    def refresh_attempt(self, attempt_id: str) -> types.PostPaymentAttemptsByIdRefreshResponse:
        """Ask the provider where an attempt stands now."""
        return self._req("POST", f"/v1/payment_attempts/{attempt_id}/refresh", {})

    def cancel_attempt(self, attempt_id: str) -> types.PostPaymentAttemptsByIdCancelResponse:
        """Cancel an attempt that has not been paid."""
        return self._req("POST", f"/v1/payment_attempts/{attempt_id}/cancel", {})

    def capture_attempt(self, attempt_id: str) -> types.PostPaymentAttemptsByIdCaptureResponse:
        """Capture an authorised attempt (manual capture)."""
        return self._req("POST", f"/v1/payment_attempts/{attempt_id}/capture", {})

    def list_refunds(self, *, status: Optional[str] = None, limit: Optional[int] = None) -> types.PostRefundsByIdApproveResponse:
        """Refunds, optionally by status (for example pending_approval)."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status, "limit": limit}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/refunds" + ("?" + q if q else ""))

    def approve_refund(self, refund_id: str) -> types.PostRefundsByIdApproveResponse:
        """Approve a refund waiting for a second person."""
        return self._req("POST", f"/v1/refunds/{refund_id}/approve", {})

    def reject_refund(self, refund_id: str) -> types.PostRefundsByIdRejectResponse:
        """Reject a refund waiting for approval."""
        return self._req("POST", f"/v1/refunds/{refund_id}/reject", {})

    def retry_refund(self, refund_id: str) -> types.PostRefundsByIdRetryResponse:
        """Retry a failed refund."""
        return self._req("POST", f"/v1/refunds/{refund_id}/retry", {})

    def refresh_refund(self, refund_id: str) -> types.PostRefundsByIdRefreshResponse:
        """Ask the provider where a refund stands."""
        return self._req("POST", f"/v1/refunds/{refund_id}/refresh", {})

    def list_settlements(self) -> types.GetSettlementsResponse:
        """Settlements imported from providers."""
        return self._req("GET", "/v1/settlements")

    def list_cases(self, *, status: Optional[str] = None, kind: Optional[str] = None) -> types.GetReconCasesByIdSuggestionsResponse:
        """Reconciliation cases."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status, "kind": kind}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/recon/cases" + ("?" + q if q else ""))

    def case_suggestions(self, case_id: str) -> types.GetReconCasesByIdSuggestionsResponse:
        """Evidence and next steps for a case (read only)."""
        return self._req("GET", f"/v1/recon/cases/{case_id}/suggestions")

    def assign_case(self, case_id: str, assignee: str) -> types.GetPlansResponse:
        """Give a case to someone."""
        body = {k: v for k, v in {"assignee": assignee}.items() if v is not None}
        return self._req("POST", f"/v1/recon/cases/{case_id}/assign", body)

    def resolve_case(self, case_id: str, resolution: str) -> types.GetPlansResponse:
        """Close a case with what was done."""
        body = {k: v for k, v in {"resolution": resolution}.items() if v is not None}
        return self._req("POST", f"/v1/recon/cases/{case_id}/resolve", body)

    def list_plans(self) -> types.GetPlansResponse:
        """Billing plans."""
        return self._req("GET", "/v1/plans")

    def list_subscriptions(self) -> types.GetSubscriptionsResponse:
        """Subscriptions."""
        return self._req("GET", "/v1/subscriptions")

    def pause_subscription(self, subscription_id: str) -> Any:
        """Pause a subscription."""
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/pause", {})

    def resume_subscription(self, subscription_id: str) -> Any:
        """Resume a paused subscription."""
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/resume", {})

    def cancel_subscription(self, subscription_id: str) -> Any:
        """Cancel a subscription."""
        return self._req("POST", f"/v1/subscriptions/{subscription_id}/cancel", {})

    def list_invoices(self, *, status: Optional[str] = None) -> types.GetInvoicesByIdResponse:
        """Invoices, optionally by status."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/invoices" + ("?" + q if q else ""))

    def get_invoice(self, invoice_id: str) -> types.GetInvoicesByIdResponse:
        """One invoice."""
        return self._req("GET", f"/v1/invoices/{invoice_id}")

    def create_invoice(self, number: str, *, amount_minor: Optional[int] = None, customer_id: Optional[str] = None, due_date: Optional[str] = None, tax_minor: Optional[int] = None, external_ref: Optional[str] = None, lines: Optional[list] = None, place_of_supply: Optional[str] = None, buyer_gstin: Optional[str] = None, reverse_charge: Optional[bool] = None) -> types.PostInvoicesByIdVoidResponse:
        """Raise an invoice: give amount_minor, or GST lines and the server computes tax and total."""
        body = {k: v for k, v in {"number": number, "amount_minor": amount_minor, "customer_id": customer_id, "due_date": due_date, "tax_minor": tax_minor, "external_ref": external_ref, "lines": lines, "place_of_supply": place_of_supply, "buyer_gstin": buyer_gstin, "reverse_charge": reverse_charge}.items() if v is not None}
        return self._req("POST", "/v1/invoices", body)

    def void_invoice(self, invoice_id: str) -> types.PostInvoicesByIdVoidResponse:
        """Void an invoice nothing has been paid or credited against."""
        return self._req("POST", f"/v1/invoices/{invoice_id}/void", {})

    def invoice_lines(self, invoice_id: str) -> types.GetInvoicesByIdLinesResponse:
        """An invoice's GST lines."""
        return self._req("GET", f"/v1/invoices/{invoice_id}/lines")

    def list_reminders(self, invoice_id: str) -> types.GetInvoicesByIdRemindersResponse:
        """Reminders sent for an invoice."""
        return self._req("GET", f"/v1/invoices/{invoice_id}/reminders")

    def list_promises(self, *, status: Optional[str] = None) -> types.PostCollectionsPromisesByIdCancelResponse:
        """Promises to pay, optionally by status (open, kept, broken, canceled)."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/collections/promises" + ("?" + q if q else ""))

    def cancel_promise(self, promise_id: str) -> types.PostCollectionsPromisesByIdCancelResponse:
        """Cancel a promise to pay."""
        return self._req("POST", f"/v1/collections/promises/{promise_id}/cancel", {})

    def create_vendor(self, name: str, *, account_number: Optional[str] = None, ifsc: Optional[str] = None, vpa: Optional[str] = None, external_ref: Optional[str] = None) -> types.GetVendorsResponse:
        """Add someone you pay out to."""
        body = {k: v for k, v in {"name": name, "account_number": account_number, "ifsc": ifsc, "vpa": vpa, "external_ref": external_ref}.items() if v is not None}
        return self._req("POST", "/v1/vendors", body)

    def list_vendors(self) -> types.GetVendorsResponse:
        """Vendors (account numbers masked)."""
        return self._req("GET", "/v1/vendors")

    def vendor_details(self, vendor_id: str) -> types.GetVendorsByIdDetailsResponse:
        """Full bank details of a vendor; needs payouts:approve and is audited."""
        return self._req("GET", f"/v1/vendors/{vendor_id}/details")

    def vendor_balance(self, vendor_id: str) -> types.GetVendorsByIdBalanceResponse:
        """What a vendor is owed from split payments and how much is free to pay out."""
        return self._req("GET", f"/v1/vendors/{vendor_id}/balance")

    def create_payout(self, vendor_id: str, amount_minor: int, purpose: str, *, currency: Optional[str] = None, external_ref: Optional[str] = None, from_balance: Optional[bool] = None) -> Any:
        """Request a payout. Repeating the same external_ref returns the original."""
        body = {k: v for k, v in {"vendor_id": vendor_id, "amount_minor": amount_minor, "purpose": purpose, "currency": currency, "external_ref": external_ref, "from_balance": from_balance}.items() if v is not None}
        return self._req("POST", "/v1/payouts", body)

    def create_payouts_bulk(self, payouts: list) -> Any:
        """Up to 200 payouts, all created or none; each item has vendor_id, amount_minor, purpose and optionally external_ref, from_balance."""
        body = {k: v for k, v in {"payouts": payouts}.items() if v is not None}
        return self._req("POST", "/v1/payouts/bulk", body)

    def list_payouts(self, *, status: Optional[str] = None) -> Any:
        """Payouts, optionally by status."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/payouts" + ("?" + q if q else ""))

    def approve_payout(self, payout_id: str) -> Any:
        """Approve a payout (a different person from the requester)."""
        return self._req("POST", f"/v1/payouts/{payout_id}/approve", {})

    def reject_payout(self, payout_id: str) -> Any:
        """Reject a payout waiting for approval."""
        return self._req("POST", f"/v1/payouts/{payout_id}/reject", {})

    def cancel_payout(self, payout_id: str) -> Any:
        """Cancel a payout that has not been paid."""
        return self._req("POST", f"/v1/payouts/{payout_id}/cancel", {})

    def mark_payout_paid(self, payout_id: str, bank_ref: str) -> types.GetCustomersByIdResponse:
        """Record the bank reference once the transfer is made."""
        body = {k: v for k, v in {"bank_ref": bank_ref}.items() if v is not None}
        return self._req("POST", f"/v1/payouts/{payout_id}/mark_paid", body)

    def list_customers(self, *, external_ref: Optional[str] = None) -> types.GetCustomersByIdResponse:
        """Customers, optionally by your own reference."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"external_ref": external_ref}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/customers" + ("?" + q if q else ""))

    def get_customer(self, customer_id: str) -> types.GetCustomersByIdResponse:
        """One customer."""
        return self._req("GET", f"/v1/customers/{customer_id}")

    def create_customer(self, name: str, *, email: Optional[str] = None, phone: Optional[str] = None, external_ref: Optional[str] = None, gstin: Optional[str] = None, address1: Optional[str] = None, location: Optional[str] = None, pincode: Optional[str] = None) -> types.PostMandatesByIdRefreshResponse:
        """Add a customer."""
        body = {k: v for k, v in {"name": name, "email": email, "phone": phone, "external_ref": external_ref, "gstin": gstin, "address1": address1, "location": location, "pincode": pincode}.items() if v is not None}
        return self._req("POST", "/v1/customers", body)

    def refresh_mandate(self, mandate_id: str) -> types.PostMandatesByIdRefreshResponse:
        """Ask the provider where a mandate stands."""
        return self._req("POST", f"/v1/mandates/{mandate_id}/refresh", {})

    def pause_mandate(self, mandate_id: str) -> types.PostMandatesByIdPauseResponse:
        """Pause a mandate."""
        return self._req("POST", f"/v1/mandates/{mandate_id}/pause", {})

    def resume_mandate(self, mandate_id: str) -> types.PostMandatesByIdResumeResponse:
        """Resume a paused mandate."""
        return self._req("POST", f"/v1/mandates/{mandate_id}/resume", {})

    def revoke_mandate(self, mandate_id: str) -> types.PostMandatesByIdRevokeResponse:
        """Revoke a mandate for good."""
        return self._req("POST", f"/v1/mandates/{mandate_id}/revoke", {})

    def run_mandates(self) -> Any:
        """Present every mandate debit that is due."""
        return self._req("POST", "/v1/mandates/run", {})

    def get_settings(self) -> types.GetSettingsResponse:
        """Organisation settings."""
        return self._req("GET", "/v1/settings")

    def create_coupon(self, code: str, kind: str, value: int, *, max_uses: Optional[int] = None, expires_at: Optional[str] = None) -> types.GetCouponsResponse:
        """A coupon: kind percent takes value in basis points (1000 = 10%), flat in minor units."""
        body = {k: v for k, v in {"code": code, "kind": kind, "value": value, "max_uses": max_uses, "expires_at": expires_at}.items() if v is not None}
        return self._req("POST", "/v1/coupons", body)

    def list_coupons(self) -> types.GetCouponsResponse:
        """Coupons."""
        return self._req("GET", "/v1/coupons")

    def deactivate_coupon(self, coupon_id: str) -> types.PostCouponsByIdDeactivateResponse:
        """Stop a coupon being redeemed."""
        return self._req("POST", f"/v1/coupons/{coupon_id}/deactivate", {})

    def list_milestones(self) -> types.GetMilestonesResponse:
        """Milestones."""
        return self._req("GET", "/v1/milestones")

    def create_milestone(self, customer_id: str, name: str, amount_minor: int, *, due_days: Optional[int] = None, external_ref: Optional[str] = None) -> types.PostMilestonesByIdCancelResponse:
        """A milestone that raises an invoice when completed."""
        body = {k: v for k, v in {"customer_id": customer_id, "name": name, "amount_minor": amount_minor, "due_days": due_days, "external_ref": external_ref}.items() if v is not None}
        return self._req("POST", "/v1/milestones", body)

    def cancel_milestone(self, milestone_id: str) -> types.PostMilestonesByIdCancelResponse:
        """Cancel a milestone that is not complete."""
        return self._req("POST", f"/v1/milestones/{milestone_id}/cancel", {})

    def create_installment_plan(self, customer_id: str, name: str, total_minor: int, instalments: int, first_due_date: str, number_prefix: str, *, every_unit: Optional[str] = None, every: Optional[int] = None, currency: Optional[str] = None, external_ref: Optional[str] = None) -> Any:
        """Split an agreed total into dated invoices; idempotent by external_ref."""
        body = {k: v for k, v in {"customer_id": customer_id, "name": name, "total_minor": total_minor, "instalments": instalments, "first_due_date": first_due_date, "number_prefix": number_prefix, "every_unit": every_unit, "every": every, "currency": currency, "external_ref": external_ref}.items() if v is not None}
        return self._req("POST", "/v1/installment_plans", body)

    def get_installment_plan(self, plan_id: str) -> Any:
        """One instalment plan."""
        return self._req("GET", f"/v1/installment_plans/{plan_id}")

    def open_dispute(self, attempt_id: str, reason: str, *, amount_minor: Optional[int] = None, provider_ref: Optional[str] = None, respond_by: Optional[str] = None) -> types.GetBranchesResponse:
        """Open a dispute; it holds the amount so it cannot also be refunded."""
        body = {k: v for k, v in {"attempt_id": attempt_id, "reason": reason, "amount_minor": amount_minor, "provider_ref": provider_ref, "respond_by": respond_by}.items() if v is not None}
        return self._req("POST", "/v1/disputes", body)

    def list_disputes(self, *, status: Optional[str] = None) -> types.GetBranchesResponse:
        """Disputes, optionally by status (open, under_review, won, lost)."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/disputes" + ("?" + q if q else ""))

    def add_dispute_evidence(self, dispute_id: str, note: str, *, url: Optional[str] = None) -> types.GetBranchesResponse:
        """Attach evidence to a dispute."""
        body = {k: v for k, v in {"note": note, "url": url}.items() if v is not None}
        return self._req("POST", f"/v1/disputes/{dispute_id}/evidence", body)

    def resolve_dispute(self, dispute_id: str, outcome: str) -> types.GetBranchesResponse:
        """Record the outcome, won or lost."""
        body = {k: v for k, v in {"outcome": outcome}.items() if v is not None}
        return self._req("POST", f"/v1/disputes/{dispute_id}/resolve", body)

    def list_branches(self) -> types.GetBranchesResponse:
        """Branches."""
        return self._req("GET", "/v1/branches")

    def create_branch(self, name: str) -> types.GetAuditEventsVerifyResponse:
        """Add a branch."""
        body = {k: v for k, v in {"name": name}.items() if v is not None}
        return self._req("POST", "/v1/branches", body)

    def verify_audit(self) -> types.GetAuditEventsVerifyResponse:
        """Check the audit trail's seals are unbroken."""
        return self._req("GET", "/v1/audit_events/verify")

    def seal_audit(self) -> types.PostAuditEventsSealResponse:
        """Seal the audit events so far."""
        return self._req("POST", "/v1/audit_events/seal", {})

    def list_keys(self) -> types.GetApiKeysResponse:
        """API keys (never the secret)."""
        return self._req("GET", "/v1/api_keys")

    def create_key(self, name: str, scopes: list, *, branch_id: Optional[str] = None, expires_at: Optional[str] = None, allowed_cidrs: Optional[list] = None) -> types.PostApiKeysByIdRevokeResponse:
        """Mint an API key; the secret is shown once."""
        body = {k: v for k, v in {"name": name, "scopes": scopes, "branch_id": branch_id, "expires_at": expires_at, "allowed_cidrs": allowed_cidrs}.items() if v is not None}
        return self._req("POST", "/v1/api_keys", body)

    def revoke_key(self, key_id: str) -> types.PostApiKeysByIdRevokeResponse:
        """Revoke an API key."""
        return self._req("POST", f"/v1/api_keys/{key_id}/revoke", {})

    def list_providers(self) -> types.GetProviderAccountsResponse:
        """Connected payment providers."""
        return self._req("GET", "/v1/provider_accounts")

    def check_provider(self, provider: str) -> types.PostProviderAccountsByProviderCheckResponse:
        """Test a provider's credentials."""
        return self._req("POST", f"/v1/provider_accounts/{provider}/check", {})

    def set_provider_enabled(self, provider: str, enabled: bool) -> Any:
        """Turn a provider on or off for routing."""
        body = {k: v for k, v in {"enabled": enabled}.items() if v is not None}
        return self._req("POST", f"/v1/provider_accounts/{provider}/enabled", body)

    def live_access_status(self) -> Any:
        """Where your request for live mode stands."""
        return self._req("GET", "/v1/live_access")

    def request_live_access(self) -> types.PostLiveAccessResponse:
        """Ask Zyvor to open live mode (the onboarding review must be approved)."""
        return self._req("POST", "/v1/live_access", {})

    def event_catalogue(self) -> types.GetEventsCatalogueResponse:
        """Every webhook event type and what it means."""
        return self._req("GET", "/v1/events/catalogue")

    def list_outbox(self, *, status: Optional[str] = None) -> types.PostOutboxByIdReplayResponse:
        """Webhook events and their delivery state."""
        q = urllib.parse.urlencode(sorted({k: v for k, v in {"status": status}.items() if v not in (None, '')}.items()))
        return self._req("GET", "/v1/outbox" + ("?" + q if q else ""))

    def replay_event(self, event_id: str) -> types.PostOutboxByIdReplayResponse:
        """Queue a webhook event again."""
        return self._req("POST", f"/v1/outbox/{event_id}/replay", {})

    def update_settings(self, patch: dict) -> types.PostSettingsResponse:
        """Change organisation settings; send only the fields to change (see GET /v1/settings for the shape)."""
        return self._req("POST", "/v1/settings", patch)


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

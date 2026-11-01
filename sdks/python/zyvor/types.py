"""Response types, generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. Do not edit."""
from __future__ import annotations

from typing import Any, List, Optional, TypedDict


AttemptRow = TypedDict("AttemptRow", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

DebitRow = TypedDict("DebitRow", {
    "id": "str",
    "mandate_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "due_date": "str",
    "notified_at": "Optional[str]",
    "invoice_id": "Optional[str]",
    "state": "str",
    "attempt_id": "Optional[str]",
    "retry_no": "int",
    "failure_reason": "Optional[str]",
    "created_at": "str",
})

DeletePlatformRotaDaysOffByIdResponse = TypedDict("DeletePlatformRotaDaysOffByIdResponse", {
    "id": "str",
    "deleted": "bool",
})

GetAnalyticsCostResponse = TypedDict("GetAnalyticsCostResponse", {
    "by_provider_method": "List[Any]",
    "suggestions": "Any",
})

GetApiKeysResponse = TypedDict("GetApiKeysResponse", {
    "data": "List[GetApiKeysResponseDataItem]",
})

GetApiKeysResponseDataItem = TypedDict("GetApiKeysResponseDataItem", {
    "branch_id": "Optional[str]",
    "id": "str",
    "name": "str",
    "prefix": "str",
    "scopes": "List[str]",
    "created_at": "Any",
    "revoked_at": "Any",
    "expires_at": "Any",
    "last_used_at": "Any",
    "allowed_cidrs": "List[str]",
})

GetAuditEventsResponse = TypedDict("GetAuditEventsResponse", {
    "data": "List[GetAuditEventsResponseDataItem]",
})

GetAuditEventsResponseDataItem = TypedDict("GetAuditEventsResponseDataItem", {
    "id": "int",
    "actor": "str",
    "action": "str",
    "subject": "Optional[str]",
    "detail": "Any",
    "created_at": "Any",
})

GetAuditEventsVerifyResponse = TypedDict("GetAuditEventsVerifyResponse", {
    "ok": "bool",
    "seals_checked": "Any",
    "broken_seal": "Any",
    "events_range": "Any",
    "reason": "Any",
})

GetBranchesByIdSummaryResponse = TypedDict("GetBranchesByIdSummaryResponse", {
    "branch": "Any",
    "from": "Any",
    "to": "Any",
    "data": "List[Any]",
})

GetBranchesResponse = TypedDict("GetBranchesResponse", {
    "data": "List[GetBranchesResponseDataItem]",
})

GetBranchesResponseDataItem = TypedDict("GetBranchesResponseDataItem", {
    "id": "str",
    "name": "str",
})

GetCheckoutByTenantIdByIntentIdResponse = TypedDict("GetCheckoutByTenantIdByIntentIdResponse", {
    "merchant": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "description": "Optional[str]",
    "status": "str",
    "phase": "Any",
    "methods": "List[Any]",
    "upi_flow": "Any",
})

GetCheckoutLinksByTenantIdByTokenResponse = TypedDict("GetCheckoutLinksByTenantIdByTokenResponse", {
    "merchant": "Optional[str]",
    "description": "Optional[str]",
    "amount_minor": "int",
    "discount_minor": "int",
    "fee_minor": "int",
    "total_minor": "int",
    "coupon_applied": "Any",
    "currency": "str",
    "status": "str",
    "open_amount": "bool",
    "min_amount_minor": "Optional[int]",
    "max_amount_minor": "Optional[int]",
})

GetClosingsResponse = TypedDict("GetClosingsResponse", {
    "data": "List[Any]",
})

GetCollectionsAgeingResponse = TypedDict("GetCollectionsAgeingResponse", {
    "as_of": "Optional[str]",
    "data": "List[Any]",
    "total_outstanding_inr_minor": "Optional[int]",
    "usd_rate_date": "Optional[str]",
})

GetCollectionsDraftsResponse = TypedDict("GetCollectionsDraftsResponse", {
    "data": "List[Any]",
})

GetCollectionsPromisesResponse = TypedDict("GetCollectionsPromisesResponse", {
    "data": "List[Any]",
})

GetConsoleBranchesResponse = TypedDict("GetConsoleBranchesResponse", {
    "data": "List[GetConsoleBranchesResponseDataItem]",
})

GetConsoleBranchesResponseDataItem = TypedDict("GetConsoleBranchesResponseDataItem", {
    "id": "str",
    "name": "str",
})

GetConsoleInvitationsResponse = TypedDict("GetConsoleInvitationsResponse", {
    "data": "List[GetConsoleInvitationsResponseDataItem]",
})

GetConsoleInvitationsResponseDataItem = TypedDict("GetConsoleInvitationsResponseDataItem", {
    "id": "str",
    "email": "str",
    "role": "str",
    "expires_at": "Any",
    "invited_by": "Optional[str]",
})

GetConsoleUsersResponse = TypedDict("GetConsoleUsersResponse", {
    "data": "List[Any]",
})

GetCountersResponse = TypedDict("GetCountersResponse", {
    "data": "List[GetCountersResponseDataItem]",
})

GetCountersResponseDataItem = TypedDict("GetCountersResponseDataItem", {
    "id": "str",
    "branch_id": "str",
    "name": "str",
    "active": "bool",
})

GetCouponsResponse = TypedDict("GetCouponsResponse", {
    "data": "List[Any]",
})

GetCustomersByIdResponse = TypedDict("GetCustomersByIdResponse", {
    "id": "str",
    "name": "str",
    "email": "str",
    "phone": "Optional[str]",
    "external_ref": "Optional[str]",
    "gstin": "Optional[str]",
    "address1": "Optional[str]",
    "location": "Optional[str]",
    "pincode": "Optional[str]",
})

GetCustomersByIdStatementResponse = TypedDict("GetCustomersByIdStatementResponse", {
    "customer_id": "str",
    "name": "Optional[str]",
    "lines": "List[Any]",
    "balance_minor": "Any",
})

GetCustomersResponse = TypedDict("GetCustomersResponse", {
    "data": "List[GetCustomersResponseDataItem]",
})

GetCustomersResponseDataItem = TypedDict("GetCustomersResponseDataItem", {
    "id": "str",
    "name": "str",
    "email": "Optional[str]",
    "phone": "Optional[str]",
    "external_ref": "Optional[str]",
    "gstin": "Optional[str]",
    "address1": "Optional[str]",
    "location": "Optional[str]",
    "pincode": "Optional[str]",
})

GetDisputesResponse = TypedDict("GetDisputesResponse", {
    "data": "List[GetDisputesResponseDataItem]",
})

GetDisputesResponseDataItem = TypedDict("GetDisputesResponseDataItem", {
    "id": "str",
    "attempt_id": "str",
    "amount_minor": "int",
    "reason": "str",
    "status": "str",
    "respond_by": "Optional[str]",
    "evidence_count": "int",
})

GetEventsCatalogueResponse = TypedDict("GetEventsCatalogueResponse", {
    "data": "List[GetEventsCatalogueResponseDataItem]",
})

GetEventsCatalogueResponseDataItem = TypedDict("GetEventsCatalogueResponseDataItem", {
    "type": "Any",
    "description": "Optional[str]",
})

GetFxConvertResponse = TypedDict("GetFxConvertResponse", {
    "amount_minor": "int",
    "currency": "str",
    "rate": "Any",
})

GetFxExposureResponse = TypedDict("GetFxExposureResponse", {
    "as_of": "Optional[str]",
    "rate_date": "Optional[str]",
    "outstanding_usd_minor": "int",
    "complete": "Any",
    "booked_inr_minor": "int",
    "current_inr_minor": "int",
    "unrealised_inr_minor": "int",
    "note": "str",
    "data": "Any",
})

GetFxRatesResponse = TypedDict("GetFxRatesResponse", {
    "data": "List[Any]",
})

GetGstGstr1Response = TypedDict("GetGstGstr1Response", {
    "note": "str",
    "period": "Any",
    "gstin": "Optional[str]",
    "legal_name": "Optional[str]",
    "b2b": "List[GetGstGstr1ResponseB2bItem]",
    "b2cl": "Any",
    "b2cs": "Any",
    "cdnr": "List[Any]",
    "cdnur": "List[Any]",
    "hsn": "Any",
    "totals": "Any",
})

GetGstGstr1ResponseB2bItem = TypedDict("GetGstGstr1ResponseB2bItem", {
    "buyer_gstin": "int",
    "invoices": "Any",
})

GetInboxResponse = TypedDict("GetInboxResponse", {
    "data": "List[GetInboxResponseDataItem]",
})

GetInboxResponseDataItem = TypedDict("GetInboxResponseDataItem", {
    "id": "int",
    "provider": "str",
    "event_id": "str",
    "kind": "str",
    "outcome": "str",
    "received_at": "Any",
})

GetInsightsCashflowResponse = TypedDict("GetInsightsCashflowResponse", {
    "as_of": "Optional[str]",
    "currency": "str",
    "weeks": "List[GetInsightsCashflowResponseWeeksItem]",
    "overdue_minor": "int",
    "due_later_minor": "int",
    "no_due_date_minor": "int",
    "promised_minor": "int",
    "payouts_waiting_minor": "int",
    "daily_receipts_run_rate_minor": "int",
    "note": "str",
})

GetInsightsCashflowResponseWeeksItem = TypedDict("GetInsightsCashflowResponseWeeksItem", {
    "starts": "Any",
    "due_minor": "int",
})

GetInsightsRefundAnomaliesResponse = TypedDict("GetInsightsRefundAnomaliesResponse", {
    "refunds_last_7_days": "int",
    "weekly_average_before": "Any",
    "flags": "List[GetInsightsRefundAnomaliesResponseFlagsItem]",
    "note": "str",
})

GetInsightsRefundAnomaliesResponseFlagsItem = TypedDict("GetInsightsRefundAnomaliesResponseFlagsItem", {
    "code": "str",
    "message": "Optional[str]",
})

GetInstallmentPlansByIdResponse = TypedDict("GetInstallmentPlansByIdResponse", {
    "id": "str",
    "customer_id": "Optional[str]",
    "name": "str",
    "total_minor": "int",
    "currency": "str",
    "instalments": "Any",
    "external_ref": "Optional[str]",
    "settled_minor": "int",
    "remaining_minor": "int",
    "invoices": "List[Any]",
})

GetInvoicesByIdEinvoiceResponse = TypedDict("GetInvoicesByIdEinvoiceResponse", {
    "draft": "bool",
    "note": "str",
    "Version": "str",
    "TranDtls": "Any",
    "DocDtls": "Any",
    "SellerDtls": "Any",
    "BuyerDtls": "Any",
    "ItemList": "List[Any]",
    "ValDtls": "Any",
})

GetInvoicesByIdLinesResponse = TypedDict("GetInvoicesByIdLinesResponse", {
    "invoice_id": "str",
    "number": "str",
    "place_of_supply": "Optional[str]",
    "buyer_gstin": "Optional[str]",
    "total_minor": "int",
    "tax_minor": "int",
    "lines": "List[Any]",
})

GetInvoicesByIdRemindersResponse = TypedDict("GetInvoicesByIdRemindersResponse", {
    "data": "List[Any]",
})

GetInvoicesByIdResponse = TypedDict("GetInvoicesByIdResponse", {
    "id": "str",
    "number": "str",
    "currency": "str",
    "amount_minor": "int",
    "tax_minor": "int",
    "external_ref": "Optional[str]",
    "status": "str",
    "due_date": "Optional[str]",
    "customer_id": "Optional[str]",
    "place_of_supply": "Optional[str]",
    "buyer_gstin": "Optional[str]",
    "reverse_charge": "bool",
    "issued_on": "str",
    "paid_minor": "int",
    "credited_minor": "int",
    "outstanding_minor": "int",
    "allocations": "List[GetInvoicesByIdResponseAllocationsItem]",
})

GetInvoicesByIdResponseAllocationsItem = TypedDict("GetInvoicesByIdResponseAllocationsItem", {
    "intent_id": "str",
    "amount_minor": "int",
    "reference": "str",
})

GetInvoicesResponse = TypedDict("GetInvoicesResponse", {
    "data": "List[GetInvoicesResponseDataItem]",
})

GetInvoicesResponseDataItem = TypedDict("GetInvoicesResponseDataItem", {
    "id": "str",
    "number": "str",
    "amount_minor": "int",
    "paid_minor": "int",
    "outstanding_minor": "int",
    "status": "str",
    "due_date": "Optional[str]",
})

GetMandatesByIdDebitsResponse = TypedDict("GetMandatesByIdDebitsResponse", {
    "data": "List[DebitRow]",
})

GetMandatesByIdResponse = TypedDict("GetMandatesByIdResponse", {
    "mandate": "Any",
    "debits": "List[DebitRow]",
})

GetMandatesResponse = TypedDict("GetMandatesResponse", {
    "data": "List[MandateRow]",
})

GetMilestonesResponse = TypedDict("GetMilestonesResponse", {
    "data": "List[GetMilestonesResponseDataItem]",
})

GetMilestonesResponseDataItem = TypedDict("GetMilestonesResponseDataItem", {
    "id": "str",
    "customer_id": "str",
    "name": "str",
    "amount_minor": "int",
    "status": "str",
    "invoice_id": "Optional[str]",
})

GetOnboardingResponse = TypedDict("GetOnboardingResponse", {
    "automated_checks": "Any",
    "review_reasons": "List[str]",
    "checks": "List[GetOnboardingResponseChecksItem]",
    "status": "str",
    "profile": "List[GetOnboardingResponseProfileItem]",
    "documents": "List[GetOnboardingResponseDocumentsItem]",
    "bank_accounts": "List[GetOnboardingResponseBankAccountsItem]",
})

GetOnboardingResponseBankAccountsItem = TypedDict("GetOnboardingResponseBankAccountsItem", {
    "id": "str",
    "holder_name": "Any",
    "ifsc": "Any",
    "account_last4": "Optional[str]",
    "verified": "bool",
})

GetOnboardingResponseChecksItem = TypedDict("GetOnboardingResponseChecksItem", {
    "kind": "str",
    "subject": "Any",
    "outcome": "str",
    "detail": "Any",
})

GetOnboardingResponseDocumentsItem = TypedDict("GetOnboardingResponseDocumentsItem", {
    "id": "str",
    "kind": "str",
    "reference": "Any",
    "status": "str",
})

GetOnboardingResponseProfileItem = TypedDict("GetOnboardingResponseProfileItem", {
    "legal_name": "Optional[str]",
    "business_type": "Any",
    "pan": "str",
    "gstin": "Optional[str]",
    "address": "Any",
    "submitted_by": "Any",
    "reviewed_by": "Any",
    "review_note": "Any",
})

GetOutboxResponse = TypedDict("GetOutboxResponse", {
    "data": "List[GetOutboxResponseDataItem]",
})

GetOutboxResponseDataItem = TypedDict("GetOutboxResponseDataItem", {
    "id": "str",
    "type": "str",
    "status": "str",
    "attempts": "int",
    "last_error": "Optional[str]",
    "payload": "Any",
    "created_at": "Any",
})

GetPaymentAttemptsByIdResponse = TypedDict("GetPaymentAttemptsByIdResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

GetPaymentIntentsByIdResponse = TypedDict("GetPaymentIntentsByIdResponse", {
    "intent": "IntentRow",
    "attempts": "List[AttemptRow]",
})

GetPaymentIntentsByIdTimelineResponse = TypedDict("GetPaymentIntentsByIdTimelineResponse", {
    "payment_id": "str",
    "status": "str",
    "standing": "Any",
    "events": "Any",
})

GetPaymentIntentsResponse = TypedDict("GetPaymentIntentsResponse", {
    "data": "List[IntentRow]",
    "has_more": "bool",
})

GetPaymentLinksResponse = TypedDict("GetPaymentLinksResponse", {
    "data": "List[Any]",
})

GetPayoutsByIdResponse = TypedDict("GetPayoutsByIdResponse", {
    "id": "str",
    "vendor_id": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "purpose": "Any",
    "external_ref": "Optional[str]",
    "requested_by": "Any",
    "approved_by": "Any",
    "paid_by": "Any",
    "bank_ref": "Any",
    "created_at": "Optional[str]",
})

GetPayoutsResponse = TypedDict("GetPayoutsResponse", {
    "data": "Any",
})

GetPlansResponse = TypedDict("GetPlansResponse", {
    "data": "List[GetPlansResponseDataItem]",
})

GetPlansResponseDataItem = TypedDict("GetPlansResponseDataItem", {
    "id": "str",
    "name": "str",
    "currency": "str",
    "amount_minor": "int",
    "interval_unit": "str",
    "interval_count": "int",
    "usage_unit_minor": "int",
    "active": "bool",
})

GetPlatformChangesResponse = TypedDict("GetPlatformChangesResponse", {
    "data": "List[GetPlatformChangesResponseDataItem]",
})

GetPlatformChangesResponseDataItem = TypedDict("GetPlatformChangesResponseDataItem", {
    "id": "str",
    "organisation_id": "Optional[str]",
    "organisation": "Any",
    "kind": "str",
    "summary": "Any",
    "status": "str",
    "requested_by": "Any",
    "requested_at": "Optional[str]",
    "note": "Optional[str]",
    "due_at": "Optional[str]",
    "overdue": "Any",
    "sla_hours": "Any",
    "stage": "str",
    "first_approved_by": "Any",
    "first_approved_by_me": "Any",
    "needs_second_because": "Optional[str]",
    "escalation_note": "Any",
    "assigned_to": "Any",
    "assigned_to_name": "Any",
    "assigned_to_me": "Any",
})

GetPlatformMeResponse = TypedDict("GetPlatformMeResponse", {
    "id": "str",
    "name": "str",
    "email": "Optional[str]",
})

GetPlatformReviewsByOrganisationIdResponse = TypedDict("GetPlatformReviewsByOrganisationIdResponse", {
    "kind": "str",
    "outcome": "str",
    "detail": "Any",
})

GetPlatformReviewsResponse = TypedDict("GetPlatformReviewsResponse", {
    "data": "Any",
})

GetPlatformRotaCoverageResponse = TypedDict("GetPlatformRotaCoverageResponse", {
    "configured": "bool",
    "days": "Any",
})

GetPlatformRotaResponse = TypedDict("GetPlatformRotaResponse", {
    "configured": "bool",
    "i_am_lead": "Any",
    "utc_offset_minutes": "Any",
    "timezone": "Any",
    "local_now": "str",
    "local_date": "str",
    "on_duty_now": "Any",
    "holiday_today": "Any",
    "reviewers": "List[Any]",
    "days_off": "List[GetPlatformRotaResponseDaysOffItem]",
})

GetPlatformRotaResponseDaysOffItem = TypedDict("GetPlatformRotaResponseDaysOffItem", {
    "id": "str",
    "reviewer_id": "Optional[str]",
    "reviewer": "Optional[str]",
    "desk": "Any",
    "mine": "Any",
    "removable": "Any",
    "first_day": "str",
    "last_day": "str",
    "note": "str",
})

GetProviderAccountsResponse = TypedDict("GetProviderAccountsResponse", {
    "data": "Any",
})

GetReconCasesByIdSuggestionsResponse = TypedDict("GetReconCasesByIdSuggestionsResponse", {
    "case_id": "str",
    "kind": "str",
    "summary": "str",
    "evidence": "List[Any]",
    "next_steps": "List[str]",
    "note": "str",
})

GetReconCasesResponse = TypedDict("GetReconCasesResponse", {
    "data": "List[Any]",
})

GetRefundsByIdResponse = TypedDict("GetRefundsByIdResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

GetRefundsResponse = TypedDict("GetRefundsResponse", {
    "data": "List[RefundRow]",
    "has_more": "bool",
})

GetSettingsResponse = TypedDict("GetSettingsResponse", {
    "refund_approval_threshold_minor": "int",
    "routing": "Any",
    "outbound_webhook_url": "Optional[str]",
    "env": "str",
    "dunning": "Any",
    "gst": "Any",
    "payout_approval_threshold_minor": "int",
    "risk": "Any",
    "webhook_events": "Any",
})

GetSettlementsByIdExplainResponse = TypedDict("GetSettlementsByIdExplainResponse", {
    "settlement": "Any",
    "payments": "List[Any]",
    "refunds": "List[Any]",
    "adjustments": "List[Any]",
    "fees": "Any",
    "arithmetic": "Any",
    "expected_minor": "int",
    "received_minor": "int",
    "difference_minor": "int",
    "difference_breakdown": "Any",
    "bank_match": "Any",
    "cases": "List[Any]",
    "narrative": "bool",
})

GetSettlementsResponse = TypedDict("GetSettlementsResponse", {
    "data": "List[Any]",
})

GetSubscriptionsResponse = TypedDict("GetSubscriptionsResponse", {
    "data": "List[GetSubscriptionsResponseDataItem]",
})

GetSubscriptionsResponseDataItem = TypedDict("GetSubscriptionsResponseDataItem", {
    "id": "str",
    "customer_id": "str",
    "plan_id": "str",
    "quantity": "int",
    "status": "str",
    "next_bill_date": "str",
})

GetVendorsByIdBalanceResponse = TypedDict("GetVendorsByIdBalanceResponse", {
    "vendor_id": "str",
    "balances": "Any",
    "awaiting_capture_minor": "int",
})

GetVendorsByIdDetailsResponse = TypedDict("GetVendorsByIdDetailsResponse", {
    "id": "str",
    "details": "Any",
})

GetVendorsResponse = TypedDict("GetVendorsResponse", {
    "data": "List[Any]",
})

IntentRow = TypedDict("IntentRow", {
    "id": "str",
    "customer_id": "Optional[str]",
    "invoice_id": "Optional[str]",
    "branch_id": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "capture_method": "str",
    "allowed_providers": "List[str]",
    "external_ref": "Optional[str]",
    "description": "Optional[str]",
    "metadata": "Any",
    "created_at": "str",
    "updated_at": "str",
})

MandateRow = TypedDict("MandateRow", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PatchCustomersByIdResponse = TypedDict("PatchCustomersByIdResponse", {
    "id": "str",
    "name": "str",
    "email": "str",
    "phone": "Optional[str]",
    "external_ref": "Optional[str]",
    "gstin": "Optional[str]",
    "address1": "Optional[str]",
    "location": "Optional[str]",
    "pincode": "Optional[str]",
})

PostApiKeysByIdRevokeResponse = TypedDict("PostApiKeysByIdRevokeResponse", {
    "id": "str",
    "revoked": "bool",
})

PostApiKeysByIdRotateResponse = TypedDict("PostApiKeysByIdRotateResponse", {
    "id": "str",
    "key": "str",
    "prefix": "str",
    "scopes": "List[str]",
    "rotated_from": "str",
    "old_key_expires_in_hours": "int",
    "note": "str",
})

PostApiKeysResponse = TypedDict("PostApiKeysResponse", {
    "id": "str",
    "key": "str",
    "prefix": "str",
    "scopes": "List[str]",
    "expires_at": "Any",
    "allowed_cidrs": "List[str]",
    "note": "str",
})

PostAuditEventsSealResponse = TypedDict("PostAuditEventsSealResponse", {
    "sealed_events": "Any",
})

PostBankStatementsImportResponse = TypedDict("PostBankStatementsImportResponse", {
    "added": "int",
    "settlements_rechecked": "int",
})

PostBillingRunResponse = TypedDict("PostBillingRunResponse", {
    "invoices_created": "int",
})

PostBranchesResponse = TypedDict("PostBranchesResponse", {
    "id": "str",
    "name": "str",
})

PostCashEntriesResponse = TypedDict("PostCashEntriesResponse", {
    "id": "str",
})

PostCheckoutByTenantIdByIntentIdAttemptsResponse = TypedDict("PostCheckoutByTenantIdByIntentIdAttemptsResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostCheckoutLinksByTenantIdByTokenStartResponse = TypedDict("PostCheckoutLinksByTenantIdByTokenStartResponse", {
    "intent_id": "str",
    "client_secret": "str",
    "amount_minor": "int",
    "currency": "str",
    "path": "str",
})

PostClosingsByIdSignoffResponse = TypedDict("PostClosingsByIdSignoffResponse", {
    "id": "str",
    "status": "str",
})

PostClosingsResponse = TypedDict("PostClosingsResponse", {
    "id": "str",
    "expected_cash_minor": "int",
    "counted_cash_minor": "int",
    "difference_minor": "int",
    "digital": "List[Any]",
    "status": "str",
})

PostCollectionsDraftsByIdApproveResponse = TypedDict("PostCollectionsDraftsByIdApproveResponse", {
    "id": "str",
    "status": "str",
    "reminder_id": "str",
})

PostCollectionsDraftsByIdRejectResponse = TypedDict("PostCollectionsDraftsByIdRejectResponse", {
    "id": "str",
    "status": "str",
})

PostCollectionsDunningRunResponse = TypedDict("PostCollectionsDunningRunResponse", {
    "reminded": "int",
    "drafts_created": "int",
    "draft_ids": "List[str]",
})

PostCollectionsPromisesByIdCancelResponse = TypedDict("PostCollectionsPromisesByIdCancelResponse", {
    "id": "str",
    "status": "str",
})

PostConsoleAuthenticateResponse = TypedDict("PostConsoleAuthenticateResponse", {
    "user": "Any",
    "mfa_enrolled": "Any",
})

PostConsoleInvitationsByIdRevokeResponse = TypedDict("PostConsoleInvitationsByIdRevokeResponse", {
    "id": "str",
    "revoked": "bool",
})

PostConsoleInvitationsResponse = TypedDict("PostConsoleInvitationsResponse", {
    "id": "str",
    "email": "str",
    "role": "str",
    "expires_at": "Any",
})

PostConsoleLogoutResponse = TypedDict("PostConsoleLogoutResponse", {
    "ok": "bool",
    "ended": "Any",
})

PostConsoleSwitchEnvResponse = TypedDict("PostConsoleSwitchEnvResponse", {
    "user": "Any",
    "session_key": "Any",
    "tenant_id": "str",
    "env": "str",
})

PostConsoleUsersByIdDisableResponse = TypedDict("PostConsoleUsersByIdDisableResponse", {
    "id": "str",
    "ok": "bool",
})

PostConsoleUsersByIdEnableResponse = TypedDict("PostConsoleUsersByIdEnableResponse", {
    "id": "str",
    "ok": "bool",
})

PostConsoleUsersByIdMfaBeginResponse = TypedDict("PostConsoleUsersByIdMfaBeginResponse", {
    "secret": "Any",
    "otpauth_uri": "str",
})

PostConsoleUsersByIdMfaConfirmResponse = TypedDict("PostConsoleUsersByIdMfaConfirmResponse", {
    "recovery_codes": "List[str]",
    "note": "str",
})

PostConsoleUsersByIdMfaResetResponse = TypedDict("PostConsoleUsersByIdMfaResetResponse", {
    "id": "str",
    "ok": "bool",
})

PostConsoleUsersByIdPasswordResponse = TypedDict("PostConsoleUsersByIdPasswordResponse", {
    "id": "str",
    "ok": "bool",
})

PostConsoleUsersResponse = TypedDict("PostConsoleUsersResponse", {
    "id": "str",
    "username": "Any",
    "role": "str",
    "branch_id": "Optional[str]",
})

PostCountersByIdDeactivateResponse = TypedDict("PostCountersByIdDeactivateResponse", {
    "id": "str",
    "active": "bool",
})

PostCountersResponse = TypedDict("PostCountersResponse", {
    "id": "str",
    "branch_id": "str",
    "name": "str",
})

PostCouponsByIdDeactivateResponse = TypedDict("PostCouponsByIdDeactivateResponse", {
    "id": "str",
    "status": "str",
})

PostCouponsResponse = TypedDict("PostCouponsResponse", {
    "id": "str",
    "code": "str",
})

PostCustomersResponse = TypedDict("PostCustomersResponse", {
    "id": "str",
    "name": "str",
    "email": "Optional[str]",
    "phone": "Optional[str]",
    "external_ref": "Optional[str]",
    "gstin": "Optional[str]",
})

PostDisputesByIdEvidenceResponse = TypedDict("PostDisputesByIdEvidenceResponse", {
    "id": "str",
    "status": "str",
})

PostDisputesByIdResolveResponse = TypedDict("PostDisputesByIdResolveResponse", {
    "id": "str",
    "status": "str",
})

PostDisputesResponse = TypedDict("PostDisputesResponse", {
    "id": "str",
    "status": "str",
    "amount_minor": "int",
})

PostFxRatesResponse = TypedDict("PostFxRatesResponse", {
    "id": "str",
    "currency": "str",
    "rate": "Any",
    "as_of": "Optional[str]",
})

PostInstallmentPlansResponse = TypedDict("PostInstallmentPlansResponse", {
    "id": "str",
    "customer_id": "Optional[str]",
    "name": "str",
    "total_minor": "int",
    "currency": "str",
    "instalments": "Any",
    "external_ref": "Optional[str]",
    "settled_minor": "int",
    "remaining_minor": "int",
    "invoices": "List[Any]",
})

PostInvitationsAcceptResponse = TypedDict("PostInvitationsAcceptResponse", {
    "email": "str",
    "joined": "bool",
})

PostInvitationsLookupResponse = TypedDict("PostInvitationsLookupResponse", {
    "email": "str",
    "organisation": "Any",
    "role": "str",
})

PostInvoicesByIdCreditNotesResponse = TypedDict("PostInvoicesByIdCreditNotesResponse", {
    "id": "str",
    "invoice_id": "str",
    "amount_minor": "int",
})

PostInvoicesByIdPromisesResponse = TypedDict("PostInvoicesByIdPromisesResponse", {
    "id": "str",
    "invoice_id": "str",
    "promised_minor": "int",
    "promised_date": "str",
    "status": "str",
})

PostInvoicesByIdRemindersDraftResponse = TypedDict("PostInvoicesByIdRemindersDraftResponse", {
    "id": "str",
    "invoice_id": "str",
    "channel": "str",
    "message": "Optional[str]",
    "source": "str",
    "status": "str",
})

PostInvoicesByIdRemindersResponse = TypedDict("PostInvoicesByIdRemindersResponse", {
    "id": "str",
    "invoice_id": "str",
    "channel": "str",
})

PostInvoicesByIdVoidResponse = TypedDict("PostInvoicesByIdVoidResponse", {
    "id": "str",
    "status": "str",
})

PostInvoicesResponse = TypedDict("PostInvoicesResponse", {
    "id": "str",
    "number": "str",
    "amount_minor": "int",
    "tax_minor": "int",
    "status": "str",
})

PostLiveAccessResponse = TypedDict("PostLiveAccessResponse", {
    "status": "str",
})

PostMandatesByIdDebitsResponse = TypedDict("PostMandatesByIdDebitsResponse", {
    "id": "str",
    "mandate_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "due_date": "str",
    "notified_at": "Optional[str]",
    "invoice_id": "Optional[str]",
    "state": "str",
    "attempt_id": "Optional[str]",
    "retry_no": "int",
    "failure_reason": "Optional[str]",
    "created_at": "str",
})

PostMandatesByIdPauseResponse = TypedDict("PostMandatesByIdPauseResponse", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostMandatesByIdRefreshResponse = TypedDict("PostMandatesByIdRefreshResponse", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostMandatesByIdResumeResponse = TypedDict("PostMandatesByIdResumeResponse", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostMandatesByIdRevokeResponse = TypedDict("PostMandatesByIdRevokeResponse", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostMandatesResponse = TypedDict("PostMandatesResponse", {
    "id": "str",
    "customer_id": "str",
    "currency": "str",
    "max_amount_minor": "int",
    "frequency": "str",
    "start_date": "str",
    "end_date": "str",
    "status": "str",
    "provider": "str",
    "provider_mandate_ref": "Optional[str]",
    "authorization": "Any",
    "failure_reason": "Optional[str]",
    "revoked_by": "Optional[str]",
    "external_ref": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostMilestonesByIdCancelResponse = TypedDict("PostMilestonesByIdCancelResponse", {
    "id": "str",
    "status": "str",
})

PostMilestonesByIdCompleteResponse = TypedDict("PostMilestonesByIdCompleteResponse", {
    "id": "str",
    "status": "str",
    "invoice_id": "Optional[str]",
})

PostMilestonesResponse = TypedDict("PostMilestonesResponse", {
    "id": "str",
    "status": "str",
})

PostOnboardingBankAccountsByIdVerifyResponse = TypedDict("PostOnboardingBankAccountsByIdVerifyResponse", {
    "id": "str",
    "verified": "bool",
})

PostOnboardingBankAccountsResponse = TypedDict("PostOnboardingBankAccountsResponse", {
    "id": "str",
    "account_last4": "Optional[str]",
    "check": "List[PostOnboardingBankAccountsResponseCheckItem]",
})

PostOnboardingBankAccountsResponseCheckItem = TypedDict("PostOnboardingBankAccountsResponseCheckItem", {
    "outcome": "str",
    "detail": "Any",
})

PostOnboardingDocumentsResponse = TypedDict("PostOnboardingDocumentsResponse", {
    "id": "str",
})

PostOnboardingProfileChangeResponse = TypedDict("PostOnboardingProfileChangeResponse", {
    "id": "str",
    "status": "str",
    "summary": "str",
})

PostOnboardingReviewResponse = TypedDict("PostOnboardingReviewResponse", {
    "status": "str",
})

PostOnboardingSubmitResponse = TypedDict("PostOnboardingSubmitResponse", {
    "status": "str",
    "automatic": "bool",
    "reasons": "Any",
})

PostOutboxByIdReplayResponse = TypedDict("PostOutboxByIdReplayResponse", {
    "id": "str",
    "status": "str",
})

PostOutboxTestResponse = TypedDict("PostOutboxTestResponse", {
    "id": "str",
    "status": "str",
})

PostPasswordResetConfirmResponse = TypedDict("PostPasswordResetConfirmResponse", {
    "ok": "bool",
})

PostPasswordResetRequestResponse = TypedDict("PostPasswordResetRequestResponse", {
    "ok": "bool",
})

PostPaymentAttemptsByIdCancelResponse = TypedDict("PostPaymentAttemptsByIdCancelResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentAttemptsByIdCaptureResponse = TypedDict("PostPaymentAttemptsByIdCaptureResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentAttemptsByIdRefreshResponse = TypedDict("PostPaymentAttemptsByIdRefreshResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentAttemptsByIdSimulateEventResponse = TypedDict("PostPaymentAttemptsByIdSimulateEventResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentIntentsByIdAttemptsResponse = TypedDict("PostPaymentIntentsByIdAttemptsResponse", {
    "id": "str",
    "intent_id": "str",
    "provider": "str",
    "method": "str",
    "status": "str",
    "amount_minor": "int",
    "currency": "str",
    "provider_order_ref": "Optional[str]",
    "provider_payment_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "route": "Any",
    "client_action": "Any",
    "refunded_minor": "int",
    "reserved_refund_minor": "int",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentIntentsByIdRefundsResponse = TypedDict("PostPaymentIntentsByIdRefundsResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostPaymentIntentsResponse = TypedDict("PostPaymentIntentsResponse", {
    "id": "str",
    "customer_id": "Optional[str]",
    "invoice_id": "Optional[str]",
    "branch_id": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "capture_method": "str",
    "allowed_providers": "List[str]",
    "external_ref": "Optional[str]",
    "description": "Optional[str]",
    "metadata": "Any",
    "created_at": "str",
    "updated_at": "str",
    "client_secret": "str",
})

PostPaymentLinksByIdCancelResponse = TypedDict("PostPaymentLinksByIdCancelResponse", {
    "id": "str",
    "status": "str",
})

PostPaymentLinksResponse = TypedDict("PostPaymentLinksResponse", {
    "id": "str",
    "token": "Any",
    "currency": "str",
    "path": "str",
    "max_uses": "Optional[int]",
    "expires_at": "Optional[str]",
})

PostPayoutsBulkResponse = TypedDict("PostPayoutsBulkResponse", {
    "count": "int",
    "already_existed": "Any",
    "new_total_minor": "int",
    "data": "Any",
})

PostPayoutsByIdApproveResponse = TypedDict("PostPayoutsByIdApproveResponse", {
    "payout_id": "Optional[str]",
    "amount_minor": "int",
})

PostPayoutsByIdCancelResponse = TypedDict("PostPayoutsByIdCancelResponse", {
    "id": "str",
    "vendor_id": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "purpose": "Any",
    "external_ref": "Optional[str]",
    "requested_by": "Any",
    "approved_by": "Any",
    "paid_by": "Any",
    "bank_ref": "Any",
    "created_at": "Optional[str]",
})

PostPayoutsByIdMarkPaidResponse = TypedDict("PostPayoutsByIdMarkPaidResponse", {
    "id": "str",
    "vendor_id": "Optional[str]",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "purpose": "Any",
    "external_ref": "Optional[str]",
    "requested_by": "Any",
    "approved_by": "Any",
    "paid_by": "Any",
    "bank_ref": "Any",
    "created_at": "Optional[str]",
})

PostPayoutsByIdRejectResponse = TypedDict("PostPayoutsByIdRejectResponse", {
    "payout_id": "Optional[str]",
    "amount_minor": "int",
})

PostPayoutsResponse = TypedDict("PostPayoutsResponse", {
    "payout_id": "str",
    "amount_minor": "int",
    "currency": "str",
})

PostPlansResponse = TypedDict("PostPlansResponse", {
    "id": "str",
    "name": "str",
    "amount_minor": "int",
    "interval_unit": "str",
})

PostPlatformChangesByIdAssignResponse = TypedDict("PostPlatformChangesByIdAssignResponse", {
    "id": "str",
    "assigned_to": "Optional[str]",
    "assigned_to_name": "Optional[str]",
})

PostPlatformChangesByIdDecisionResponse = TypedDict("PostPlatformChangesByIdDecisionResponse", {
    "id": "str",
    "status": "str",
    "stage": "str",
    "needs_second_because": "Optional[str]",
})

PostPlatformChangesByIdEscalateResponse = TypedDict("PostPlatformChangesByIdEscalateResponse", {
    "id": "str",
    "status": "str",
    "escalated": "bool",
})

PostPlatformReviewsByOrganisationIdAssignResponse = TypedDict("PostPlatformReviewsByOrganisationIdAssignResponse", {
    "organisation_id": "str",
    "assigned_to": "Optional[str]",
    "assigned_to_name": "Optional[str]",
})

PostPlatformReviewsByOrganisationIdDecisionResponse = TypedDict("PostPlatformReviewsByOrganisationIdDecisionResponse", {
    "organisation_id": "str",
    "status": "str",
    "stage": "str",
    "needs_second_because": "Optional[str]",
})

PostPlatformReviewsByOrganisationIdEscalateResponse = TypedDict("PostPlatformReviewsByOrganisationIdEscalateResponse", {
    "organisation_id": "str",
    "status": "str",
    "escalated": "bool",
})

PostPlatformReviewsByOrganisationIdRestoreResponse = TypedDict("PostPlatformReviewsByOrganisationIdRestoreResponse", {
    "organisation_id": "Optional[str]",
    "status": "str",
})

PostPlatformReviewsByOrganisationIdRevokeResponse = TypedDict("PostPlatformReviewsByOrganisationIdRevokeResponse", {
    "organisation_id": "Optional[str]",
    "status": "str",
})

PostPlatformRotaDaysOffResponse = TypedDict("PostPlatformRotaDaysOffResponse", {
    "id": "str",
    "desk": "bool",
    "reviewer_id": "Optional[str]",
    "first_day": "str",
    "last_day": "str",
    "note": "str",
})

PostPlatformRotaRebalanceResponse = TypedDict("PostPlatformRotaRebalanceResponse", {
    "moved": "Any",
    "evened": "Any",
    "left": "Any",
})

PostProviderAccountsByProviderCheckResponse = TypedDict("PostProviderAccountsByProviderCheckResponse", {
    "provider": "str",
    "ok": "bool",
    "detail": "Any",
    "env": "str",
})

PostProviderAccountsByProviderEnabledResponse = TypedDict("PostProviderAccountsByProviderEnabledResponse", {
    "provider": "str",
    "enabled": "bool",
})

PostProviderAccountsResponse = TypedDict("PostProviderAccountsResponse", {
    "id": "str",
    "provider": "str",
    "updated": "Any",
    "webhook_url": "str",
})

PostReconCasesByIdAssignResponse = TypedDict("PostReconCasesByIdAssignResponse", {
    "id": "str",
    "status": "str",
    "assignee": "str",
})

PostReconCasesByIdResolveResponse = TypedDict("PostReconCasesByIdResolveResponse", {
    "id": "str",
    "status": "str",
})

PostReconScanResponse = TypedDict("PostReconScanResponse", {
    "new_cases": "int",
})

PostRefundsByIdApproveResponse = TypedDict("PostRefundsByIdApproveResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostRefundsByIdRefreshResponse = TypedDict("PostRefundsByIdRefreshResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostRefundsByIdRejectResponse = TypedDict("PostRefundsByIdRejectResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostRefundsByIdRetryResponse = TypedDict("PostRefundsByIdRetryResponse", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

PostSettingsResponse = TypedDict("PostSettingsResponse", {
    "ok": "bool",
})

PostSettlementsByIdSimulateBankResponse = TypedDict("PostSettlementsByIdSimulateBankResponse", {
    "added": "int",
    "settlements_rechecked": "int",
})

PostSettlementsImportResponse = TypedDict("PostSettlementsImportResponse", {
    "settlement": "Any",
    "payments": "List[Any]",
    "refunds": "List[Any]",
    "adjustments": "List[Any]",
    "fees": "Any",
    "arithmetic": "Any",
    "expected_minor": "int",
    "received_minor": "int",
    "difference_minor": "int",
    "difference_breakdown": "Any",
    "bank_match": "Any",
    "cases": "List[Any]",
    "narrative": "bool",
})

PostSettlementsSyncResponse = TypedDict("PostSettlementsSyncResponse", {
    "imported": "int",
})

PostSignupResendResponse = TypedDict("PostSignupResendResponse", {
    "ok": "bool",
})

PostSignupResponse = TypedDict("PostSignupResponse", {
    "organisation_id": "Optional[str]",
    "test_tenant": "Any",
    "test_key": "Any",
    "email": "str",
    "verification": "str",
})

PostSignupVerifyResponse = TypedDict("PostSignupVerifyResponse", {
    "organisation_id": "Optional[str]",
    "verified": "bool",
})

PostSimCirclePayResponse = TypedDict("PostSimCirclePayResponse", {
    "status": "str",
    "remaining_minor": "int",
})

PostSimCircleResponse = TypedDict("PostSimCircleResponse", {
    "circle_id": "Optional[str]",
})

PostSimCircleRevokeResponse = TypedDict("PostSimCircleRevokeResponse", {
    "status": "str",
})

PostSimMandateResponse = TypedDict("PostSimMandateResponse", {
    "status": "str",
})

PostSimMandateRevokeResponse = TypedDict("PostSimMandateRevokeResponse", {
    "status": "str",
})

PostSimPayResponse = TypedDict("PostSimPayResponse", {
    "status": "str",
})

PostSubscriptionsByIdCancelResponse = TypedDict("PostSubscriptionsByIdCancelResponse", {
    "id": "str",
    "status": "str",
})

PostSubscriptionsByIdChangePlanResponse = TypedDict("PostSubscriptionsByIdChangePlanResponse", {
    "id": "str",
    "plan_id": "str",
    "quantity": "int",
    "proration_minor": "bool",
    "proration_invoice_id": "Optional[str]",
    "remaining_days": "int",
    "period_days": "int",
})

PostSubscriptionsByIdMandateResponse = TypedDict("PostSubscriptionsByIdMandateResponse", {
    "id": "str",
    "mandate_id": "Optional[str]",
})

PostSubscriptionsByIdPauseResponse = TypedDict("PostSubscriptionsByIdPauseResponse", {
    "id": "str",
    "status": "str",
})

PostSubscriptionsByIdResumeResponse = TypedDict("PostSubscriptionsByIdResumeResponse", {
    "id": "str",
    "status": "str",
})

PostSubscriptionsByIdUsageResponse = TypedDict("PostSubscriptionsByIdUsageResponse", {
    "recorded": "int",
    "quantity": "int",
})

PostSubscriptionsResponse = TypedDict("PostSubscriptionsResponse", {
    "id": "str",
    "status": "str",
    "next_bill_date": "Optional[str]",
})

PostUsersResponse = TypedDict("PostUsersResponse", {
    "id": "str",
    "email": "str",
    "role": "str",
})

PostVendorsResponse = TypedDict("PostVendorsResponse", {
    "id": "str",
    "name": "str",
})

PostWebhooksByProviderByTenantIdResponse = TypedDict("PostWebhooksByProviderByTenantIdResponse", {
    "status": "str",
})

PutOnboardingResponse = TypedDict("PutOnboardingResponse", {
    "status": "str",
})

PutPlatformMeResponse = TypedDict("PutPlatformMeResponse", {
    "id": "str",
    "name": "str",
    "email": "Optional[str]",
})

PutPlatformRotaMeResponse = TypedDict("PutPlatformRotaMeResponse", {
    "shifts": "int",
    "on_duty": "Any",
})

PutPlatformRotaReviewersByIdResponse = TypedDict("PutPlatformRotaReviewersByIdResponse", {
    "reviewer": "str",
    "name": "str",
    "shifts": "int",
    "on_duty": "Any",
})

PutProviderAccountsByProviderResponse = TypedDict("PutProviderAccountsByProviderResponse", {
    "id": "str",
    "provider": "str",
    "enabled": "bool",
    "replaced": "Any",
})

RefundRow = TypedDict("RefundRow", {
    "id": "str",
    "attempt_id": "str",
    "intent_id": "str",
    "amount_minor": "int",
    "currency": "str",
    "status": "str",
    "reason": "Optional[str]",
    "external_ref": "Optional[str]",
    "requested_by": "str",
    "approved_by": "Optional[str]",
    "provider_refund_ref": "Optional[str]",
    "failure_reason": "Optional[str]",
    "created_at": "str",
    "updated_at": "str",
})

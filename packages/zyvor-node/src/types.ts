// Response types, generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. Do not edit.

export interface AttemptRow {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface DebitRow {
  id: string;
  mandate_id: string;
  amount_minor: number;
  currency: string;
  due_date: string;
  notified_at: string | null;
  invoice_id: string | null;
  state: string;
  attempt_id: string | null;
  retry_no: number;
  failure_reason: string | null;
  created_at: string;
}

export interface DeletePlatformRotaDaysOffByIdResponse {
  id: string;
  deleted: boolean;
}

export interface GetAnalyticsCostResponse {
  by_provider_method: unknown[];
  suggestions: unknown;
}

export interface GetApiKeysResponse {
  data: GetApiKeysResponseDataItem[];
}

export interface GetApiKeysResponseDataItem {
  branch_id: string | null;
  id: string;
  name: string;
  prefix: string;
  scopes: string[];
  created_at: unknown;
  revoked_at: unknown;
  expires_at: unknown;
  last_used_at: unknown;
  allowed_cidrs: string[];
}

export interface GetAuditEventsResponse {
  data: GetAuditEventsResponseDataItem[];
}

export interface GetAuditEventsResponseDataItem {
  id: number;
  actor: string;
  action: string;
  subject: string | null;
  detail: unknown;
  created_at: unknown;
}

export interface GetAuditEventsVerifyResponse {
  ok: boolean;
  seals_checked: unknown;
  broken_seal: unknown;
  events_range: unknown;
  reason: unknown;
}

export interface GetBranchesByIdSummaryResponse {
  branch: unknown;
  from: unknown;
  to: unknown;
  data: unknown[];
}

export interface GetBranchesResponse {
  data: GetBranchesResponseDataItem[];
}

export interface GetBranchesResponseDataItem {
  id: string;
  name: string;
}

export interface GetCheckoutByTenantIdByIntentIdResponse {
  merchant: string | null;
  amount_minor: number;
  currency: string;
  description: string | null;
  status: string;
  phase: unknown;
  methods: unknown[];
  upi_flow: unknown;
}

export interface GetCheckoutLinksByTenantIdByTokenResponse {
  merchant: string | null;
  description: string | null;
  amount_minor: number;
  discount_minor: number;
  fee_minor: number;
  total_minor: number;
  coupon_applied: unknown;
  currency: string;
  status: string;
  open_amount: boolean;
  min_amount_minor: number | null;
  max_amount_minor: number | null;
}

export interface GetClosingsResponse {
  data: unknown[];
}

export interface GetCollectionsAgeingResponse {
  as_of: string | null;
  data: unknown[];
  total_outstanding_inr_minor: number | null;
  usd_rate_date: string | null;
}

export interface GetCollectionsDraftsResponse {
  data: unknown[];
}

export interface GetCollectionsPromisesResponse {
  data: unknown[];
}

export interface GetConsoleBranchesResponse {
  data: GetConsoleBranchesResponseDataItem[];
}

export interface GetConsoleBranchesResponseDataItem {
  id: string;
  name: string;
}

export interface GetConsoleInvitationsResponse {
  data: GetConsoleInvitationsResponseDataItem[];
}

export interface GetConsoleInvitationsResponseDataItem {
  id: string;
  email: string;
  role: string;
  expires_at: unknown;
  invited_by: string | null;
}

export interface GetConsoleUsersResponse {
  data: unknown[];
}

export interface GetCountersResponse {
  data: GetCountersResponseDataItem[];
}

export interface GetCountersResponseDataItem {
  id: string;
  branch_id: string;
  name: string;
  active: boolean;
}

export interface GetCouponsResponse {
  data: unknown[];
}

export interface GetCustomersByIdResponse {
  id: string;
  name: string;
  email: string;
  phone: string | null;
  external_ref: string | null;
  gstin: string | null;
  address1: string | null;
  location: string | null;
  pincode: string | null;
}

export interface GetCustomersByIdStatementResponse {
  customer_id: string;
  name: string | null;
  lines: unknown[];
  balance_minor: unknown;
}

export interface GetCustomersResponse {
  data: GetCustomersResponseDataItem[];
}

export interface GetCustomersResponseDataItem {
  id: string;
  name: string;
  email: string | null;
  phone: string | null;
  external_ref: string | null;
  gstin: string | null;
  address1: string | null;
  location: string | null;
  pincode: string | null;
}

export interface GetDisputesResponse {
  data: GetDisputesResponseDataItem[];
}

export interface GetDisputesResponseDataItem {
  id: string;
  attempt_id: string;
  amount_minor: number;
  reason: string;
  status: string;
  respond_by: string | null;
  evidence_count: number;
}

export interface GetEventsCatalogueResponse {
  data: GetEventsCatalogueResponseDataItem[];
}

export interface GetEventsCatalogueResponseDataItem {
  type: unknown;
  description: string | null;
}

export interface GetFxConvertResponse {
  amount_minor: number;
  currency: string;
  rate: unknown;
}

export interface GetFxExposureResponse {
  as_of: string | null;
  rate_date: string | null;
  outstanding_usd_minor: number;
  complete: unknown;
  booked_inr_minor: number;
  current_inr_minor: number;
  unrealised_inr_minor: number;
  note: string;
  data: unknown;
}

export interface GetFxRatesResponse {
  data: unknown[];
}

export interface GetGstGstr1Response {
  note: string;
  period: unknown;
  gstin: string | null;
  legal_name: string | null;
  b2b: GetGstGstr1ResponseB2bItem[];
  b2cl: unknown;
  b2cs: unknown;
  cdnr: unknown[];
  cdnur: unknown[];
  hsn: unknown;
  totals: unknown;
}

export interface GetGstGstr1ResponseB2bItem {
  buyer_gstin: number;
  invoices: unknown;
}

export interface GetInboxResponse {
  data: GetInboxResponseDataItem[];
}

export interface GetInboxResponseDataItem {
  id: number;
  provider: string;
  event_id: string;
  kind: string;
  outcome: string;
  received_at: unknown;
}

export interface GetInsightsCashflowResponse {
  as_of: string | null;
  currency: string;
  weeks: GetInsightsCashflowResponseWeeksItem[];
  overdue_minor: number;
  due_later_minor: number;
  no_due_date_minor: number;
  promised_minor: number;
  payouts_waiting_minor: number;
  daily_receipts_run_rate_minor: number;
  note: string;
}

export interface GetInsightsCashflowResponseWeeksItem {
  starts: unknown;
  due_minor: number;
}

export interface GetInsightsRefundAnomaliesResponse {
  refunds_last_7_days: number;
  weekly_average_before: unknown;
  flags: GetInsightsRefundAnomaliesResponseFlagsItem[];
  note: string;
}

export interface GetInsightsRefundAnomaliesResponseFlagsItem {
  code: string;
  message: string | null;
}

export interface GetInstallmentPlansByIdResponse {
  id: string;
  customer_id: string | null;
  name: string;
  total_minor: number;
  currency: string;
  instalments: unknown;
  external_ref: string | null;
  settled_minor: number;
  remaining_minor: number;
  invoices: unknown[];
}

export interface GetInvoicesByIdEinvoiceResponse {
  draft: boolean;
  note: string;
  Version: string;
  TranDtls: unknown;
  DocDtls: unknown;
  SellerDtls: unknown;
  BuyerDtls: unknown;
  ItemList: unknown[];
  ValDtls: unknown;
}

export interface GetInvoicesByIdLinesResponse {
  invoice_id: string;
  number: string;
  place_of_supply: string | null;
  buyer_gstin: string | null;
  total_minor: number;
  tax_minor: number;
  lines: unknown[];
}

export interface GetInvoicesByIdRemindersResponse {
  data: unknown[];
}

export interface GetInvoicesByIdResponse {
  id: string;
  number: string;
  currency: string;
  amount_minor: number;
  tax_minor: number;
  external_ref: string | null;
  status: string;
  due_date: string | null;
  customer_id: string | null;
  place_of_supply: string | null;
  buyer_gstin: string | null;
  reverse_charge: boolean;
  issued_on: string;
  paid_minor: number;
  credited_minor: number;
  outstanding_minor: number;
  allocations: GetInvoicesByIdResponseAllocationsItem[];
}

export interface GetInvoicesByIdResponseAllocationsItem {
  intent_id: string;
  amount_minor: number;
  reference: string;
}

export interface GetInvoicesResponse {
  data: GetInvoicesResponseDataItem[];
}

export interface GetInvoicesResponseDataItem {
  id: string;
  number: string;
  amount_minor: number;
  paid_minor: number;
  outstanding_minor: number;
  status: string;
  due_date: string | null;
}

export interface GetMandatesByIdDebitsResponse {
  data: DebitRow[];
}

export interface GetMandatesByIdResponse {
  mandate: unknown;
  debits: DebitRow[];
}

export interface GetMandatesResponse {
  data: MandateRow[];
}

export interface GetMilestonesResponse {
  data: GetMilestonesResponseDataItem[];
}

export interface GetMilestonesResponseDataItem {
  id: string;
  customer_id: string;
  name: string;
  amount_minor: number;
  status: string;
  invoice_id: string | null;
}

export interface GetOnboardingResponse {
  automated_checks: unknown;
  review_reasons: string[];
  checks: GetOnboardingResponseChecksItem[];
  status: string;
  profile: GetOnboardingResponseProfileItem[];
  documents: GetOnboardingResponseDocumentsItem[];
  bank_accounts: GetOnboardingResponseBankAccountsItem[];
}

export interface GetOnboardingResponseBankAccountsItem {
  id: string;
  holder_name: unknown;
  ifsc: unknown;
  account_last4: string | null;
  verified: boolean;
}

export interface GetOnboardingResponseChecksItem {
  kind: string;
  subject: unknown;
  outcome: string;
  detail: unknown;
}

export interface GetOnboardingResponseDocumentsItem {
  id: string;
  kind: string;
  reference: unknown;
  status: string;
}

export interface GetOnboardingResponseProfileItem {
  legal_name: string | null;
  business_type: unknown;
  pan: string;
  gstin: string | null;
  address: unknown;
  submitted_by: unknown;
  reviewed_by: unknown;
  review_note: unknown;
}

export interface GetOutboxResponse {
  data: GetOutboxResponseDataItem[];
}

export interface GetOutboxResponseDataItem {
  id: string;
  type: string;
  status: string;
  attempts: number;
  last_error: string | null;
  payload: unknown;
  created_at: unknown;
}

export interface GetPaymentAttemptsByIdResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface GetPaymentIntentsByIdResponse {
  intent: IntentRow;
  attempts: AttemptRow[];
}

export interface GetPaymentIntentsByIdTimelineResponse {
  payment_id: string;
  status: string;
  standing: unknown;
  events: unknown;
}

export interface GetPaymentIntentsResponse {
  data: IntentRow[];
  has_more: boolean;
}

export interface GetPaymentLinksResponse {
  data: unknown[];
}

export interface GetPayoutsByIdResponse {
  id: string;
  vendor_id: string | null;
  amount_minor: number;
  currency: string;
  status: string;
  purpose: unknown;
  external_ref: string | null;
  requested_by: unknown;
  approved_by: unknown;
  paid_by: unknown;
  bank_ref: unknown;
  created_at: string | null;
}

export interface GetPayoutsResponse {
  data: unknown;
}

export interface GetPlansResponse {
  data: GetPlansResponseDataItem[];
}

export interface GetPlansResponseDataItem {
  id: string;
  name: string;
  currency: string;
  amount_minor: number;
  interval_unit: string;
  interval_count: number;
  usage_unit_minor: number;
  active: boolean;
}

export interface GetPlatformChangesResponse {
  data: GetPlatformChangesResponseDataItem[];
}

export interface GetPlatformChangesResponseDataItem {
  id: string;
  organisation_id: string | null;
  organisation: unknown;
  kind: string;
  summary: unknown;
  status: string;
  requested_by: unknown;
  requested_at: string | null;
  note: string | null;
  due_at: string | null;
  overdue: unknown;
  sla_hours: unknown;
  stage: string;
  first_approved_by: unknown;
  first_approved_by_me: unknown;
  needs_second_because: string | null;
  escalation_note: unknown;
  assigned_to: unknown;
  assigned_to_name: unknown;
  assigned_to_me: unknown;
}

export interface GetPlatformMeResponse {
  id: string;
  name: string;
  email: string | null;
}

export interface GetPlatformReviewsByOrganisationIdResponse {
  kind: string;
  outcome: string;
  detail: unknown;
}

export interface GetPlatformReviewsResponse {
  data: unknown;
}

export interface GetPlatformRotaCoverageResponse {
  configured: boolean;
  days: unknown;
}

export interface GetPlatformRotaResponse {
  configured: boolean;
  i_am_lead: unknown;
  utc_offset_minutes: unknown;
  timezone: unknown;
  local_now: string;
  local_date: string;
  on_duty_now: unknown;
  holiday_today: unknown;
  reviewers: unknown[];
  days_off: GetPlatformRotaResponseDaysOffItem[];
}

export interface GetPlatformRotaResponseDaysOffItem {
  id: string;
  reviewer_id: string | null;
  reviewer: string | null;
  desk: unknown;
  mine: unknown;
  removable: unknown;
  first_day: string;
  last_day: string;
  note: string;
}

export interface GetProviderAccountsResponse {
  data: unknown;
}

export interface GetReconCasesByIdSuggestionsResponse {
  case_id: string;
  kind: string;
  summary: string;
  evidence: unknown[];
  next_steps: string[];
  note: string;
}

export interface GetReconCasesResponse {
  data: unknown[];
}

export interface GetRefundsByIdResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface GetRefundsResponse {
  data: RefundRow[];
  has_more: boolean;
}

export interface GetSettingsResponse {
  refund_approval_threshold_minor: number;
  routing: unknown;
  outbound_webhook_url: string | null;
  env: string;
  dunning: unknown;
  gst: unknown;
  payout_approval_threshold_minor: number;
  risk: unknown;
  webhook_events: unknown;
}

export interface GetSettlementsByIdExplainResponse {
  settlement: unknown;
  payments: unknown[];
  refunds: unknown[];
  adjustments: unknown[];
  fees: unknown;
  arithmetic: unknown;
  expected_minor: number;
  received_minor: number;
  difference_minor: number;
  difference_breakdown: unknown;
  bank_match: unknown;
  cases: unknown[];
  narrative: boolean;
}

export interface GetSettlementsResponse {
  data: unknown[];
}

export interface GetSubscriptionsResponse {
  data: GetSubscriptionsResponseDataItem[];
}

export interface GetSubscriptionsResponseDataItem {
  id: string;
  customer_id: string;
  plan_id: string;
  quantity: number;
  status: string;
  next_bill_date: string;
}

export interface GetVendorsByIdBalanceResponse {
  vendor_id: string;
  balances: unknown;
  awaiting_capture_minor: number;
}

export interface GetVendorsByIdDetailsResponse {
  id: string;
  details: unknown;
}

export interface GetVendorsResponse {
  data: unknown[];
}

export interface IntentRow {
  id: string;
  customer_id: string | null;
  invoice_id: string | null;
  branch_id: string | null;
  amount_minor: number;
  currency: string;
  status: string;
  capture_method: string;
  allowed_providers: string[];
  external_ref: string | null;
  description: string | null;
  metadata: unknown;
  created_at: string;
  updated_at: string;
}

export interface MandateRow {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PatchCustomersByIdResponse {
  id: string;
  name: string;
  email: string;
  phone: string | null;
  external_ref: string | null;
  gstin: string | null;
  address1: string | null;
  location: string | null;
  pincode: string | null;
}

export interface PostApiKeysByIdRevokeResponse {
  id: string;
  revoked: boolean;
}

export interface PostApiKeysByIdRotateResponse {
  id: string;
  key: string;
  prefix: string;
  scopes: string[];
  rotated_from: string;
  old_key_expires_in_hours: number;
  note: string;
}

export interface PostApiKeysResponse {
  id: string;
  key: string;
  prefix: string;
  scopes: string[];
  expires_at: unknown;
  allowed_cidrs: string[];
  note: string;
}

export interface PostAuditEventsSealResponse {
  sealed_events: unknown;
}

export interface PostBankStatementsImportResponse {
  added: number;
  settlements_rechecked: number;
}

export interface PostBillingRunResponse {
  invoices_created: number;
}

export interface PostBranchesResponse {
  id: string;
  name: string;
}

export interface PostCashEntriesResponse {
  id: string;
}

export interface PostCheckoutByTenantIdByIntentIdAttemptsResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostCheckoutLinksByTenantIdByTokenStartResponse {
  intent_id: string;
  client_secret: string;
  amount_minor: number;
  currency: string;
  path: string;
}

export interface PostClosingsByIdSignoffResponse {
  id: string;
  status: string;
}

export interface PostClosingsResponse {
  id: string;
  expected_cash_minor: number;
  counted_cash_minor: number;
  difference_minor: number;
  digital: unknown[];
  status: string;
}

export interface PostCollectionsDraftsByIdApproveResponse {
  id: string;
  status: string;
  reminder_id: string;
}

export interface PostCollectionsDraftsByIdRejectResponse {
  id: string;
  status: string;
}

export interface PostCollectionsDunningRunResponse {
  reminded: number;
  drafts_created: number;
  draft_ids: string[];
}

export interface PostCollectionsPromisesByIdCancelResponse {
  id: string;
  status: string;
}

export interface PostConsoleAuthenticateResponse {
  user: unknown;
  mfa_enrolled: unknown;
}

export interface PostConsoleInvitationsByIdRevokeResponse {
  id: string;
  revoked: boolean;
}

export interface PostConsoleInvitationsResponse {
  id: string;
  email: string;
  role: string;
  expires_at: unknown;
}

export interface PostConsoleLogoutResponse {
  ok: boolean;
  ended: unknown;
}

export interface PostConsoleSwitchEnvResponse {
  user: unknown;
  session_key: unknown;
  tenant_id: string;
  env: string;
}

export interface PostConsoleUsersByIdDisableResponse {
  id: string;
  ok: boolean;
}

export interface PostConsoleUsersByIdEnableResponse {
  id: string;
  ok: boolean;
}

export interface PostConsoleUsersByIdMfaBeginResponse {
  secret: unknown;
  otpauth_uri: string;
}

export interface PostConsoleUsersByIdMfaConfirmResponse {
  recovery_codes: string[];
  note: string;
}

export interface PostConsoleUsersByIdMfaResetResponse {
  id: string;
  ok: boolean;
}

export interface PostConsoleUsersByIdPasswordResponse {
  id: string;
  ok: boolean;
}

export interface PostConsoleUsersResponse {
  id: string;
  username: unknown;
  role: string;
  branch_id: string | null;
}

export interface PostCountersByIdDeactivateResponse {
  id: string;
  active: boolean;
}

export interface PostCountersResponse {
  id: string;
  branch_id: string;
  name: string;
}

export interface PostCouponsByIdDeactivateResponse {
  id: string;
  status: string;
}

export interface PostCouponsResponse {
  id: string;
  code: string;
}

export interface PostCustomersResponse {
  id: string;
  name: string;
  email: string | null;
  phone: string | null;
  external_ref: string | null;
  gstin: string | null;
}

export interface PostDisputesByIdEvidenceResponse {
  id: string;
  status: string;
}

export interface PostDisputesByIdResolveResponse {
  id: string;
  status: string;
}

export interface PostDisputesResponse {
  id: string;
  status: string;
  amount_minor: number;
}

export interface PostFxRatesResponse {
  id: string;
  currency: string;
  rate: unknown;
  as_of: string | null;
}

export interface PostInstallmentPlansResponse {
  id: string;
  customer_id: string | null;
  name: string;
  total_minor: number;
  currency: string;
  instalments: unknown;
  external_ref: string | null;
  settled_minor: number;
  remaining_minor: number;
  invoices: unknown[];
}

export interface PostInvitationsAcceptResponse {
  email: string;
  joined: boolean;
}

export interface PostInvitationsLookupResponse {
  email: string;
  organisation: unknown;
  role: string;
}

export interface PostInvoicesByIdCreditNotesResponse {
  id: string;
  invoice_id: string;
  amount_minor: number;
}

export interface PostInvoicesByIdPromisesResponse {
  id: string;
  invoice_id: string;
  promised_minor: number;
  promised_date: string;
  status: string;
}

export interface PostInvoicesByIdRemindersDraftResponse {
  id: string;
  invoice_id: string;
  channel: string;
  message: string | null;
  source: string;
  status: string;
}

export interface PostInvoicesByIdRemindersResponse {
  id: string;
  invoice_id: string;
  channel: string;
}

export interface PostInvoicesByIdVoidResponse {
  id: string;
  status: string;
}

export interface PostInvoicesResponse {
  id: string;
  number: string;
  amount_minor: number;
  tax_minor: number;
  status: string;
}

export interface PostLiveAccessResponse {
  status: string;
}

export interface PostMandatesByIdDebitsResponse {
  id: string;
  mandate_id: string;
  amount_minor: number;
  currency: string;
  due_date: string;
  notified_at: string | null;
  invoice_id: string | null;
  state: string;
  attempt_id: string | null;
  retry_no: number;
  failure_reason: string | null;
  created_at: string;
}

export interface PostMandatesByIdPauseResponse {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostMandatesByIdRefreshResponse {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostMandatesByIdResumeResponse {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostMandatesByIdRevokeResponse {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostMandatesResponse {
  id: string;
  customer_id: string;
  currency: string;
  max_amount_minor: number;
  frequency: string;
  start_date: string;
  end_date: string;
  status: string;
  provider: string;
  provider_mandate_ref: string | null;
  authorization: unknown;
  failure_reason: string | null;
  revoked_by: string | null;
  external_ref: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostMilestonesByIdCancelResponse {
  id: string;
  status: string;
}

export interface PostMilestonesByIdCompleteResponse {
  id: string;
  status: string;
  invoice_id: string | null;
}

export interface PostMilestonesResponse {
  id: string;
  status: string;
}

export interface PostOnboardingBankAccountsByIdVerifyResponse {
  id: string;
  verified: boolean;
}

export interface PostOnboardingBankAccountsResponse {
  id: string;
  account_last4: string | null;
  check: PostOnboardingBankAccountsResponseCheckItem[];
}

export interface PostOnboardingBankAccountsResponseCheckItem {
  outcome: string;
  detail: unknown;
}

export interface PostOnboardingDocumentsResponse {
  id: string;
}

export interface PostOnboardingProfileChangeResponse {
  id: string;
  status: string;
  summary: string;
}

export interface PostOnboardingReviewResponse {
  status: string;
}

export interface PostOnboardingSubmitResponse {
  status: string;
  automatic: boolean;
  reasons: unknown;
}

export interface PostOutboxByIdReplayResponse {
  id: string;
  status: string;
}

export interface PostOutboxTestResponse {
  id: string;
  status: string;
}

export interface PostPasswordResetConfirmResponse {
  ok: boolean;
}

export interface PostPasswordResetRequestResponse {
  ok: boolean;
}

export interface PostPaymentAttemptsByIdCancelResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentAttemptsByIdCaptureResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentAttemptsByIdRefreshResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentAttemptsByIdSimulateEventResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentIntentsByIdAttemptsResponse {
  id: string;
  intent_id: string;
  provider: string;
  method: string;
  status: string;
  amount_minor: number;
  currency: string;
  provider_order_ref: string | null;
  provider_payment_ref: string | null;
  failure_reason: string | null;
  route: unknown;
  client_action: unknown;
  refunded_minor: number;
  reserved_refund_minor: number;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentIntentsByIdRefundsResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostPaymentIntentsResponse {
  id: string;
  customer_id: string | null;
  invoice_id: string | null;
  branch_id: string | null;
  amount_minor: number;
  currency: string;
  status: string;
  capture_method: string;
  allowed_providers: string[];
  external_ref: string | null;
  description: string | null;
  metadata: unknown;
  created_at: string;
  updated_at: string;
  client_secret: string;
}

export interface PostPaymentLinksByIdCancelResponse {
  id: string;
  status: string;
}

export interface PostPaymentLinksResponse {
  id: string;
  token: unknown;
  currency: string;
  path: string;
  max_uses: number | null;
  expires_at: string | null;
}

export interface PostPayoutsBulkResponse {
  count: number;
  already_existed: unknown;
  new_total_minor: number;
  data: unknown;
}

export interface PostPayoutsByIdApproveResponse {
  payout_id: string | null;
  amount_minor: number;
}

export interface PostPayoutsByIdCancelResponse {
  id: string;
  vendor_id: string | null;
  amount_minor: number;
  currency: string;
  status: string;
  purpose: unknown;
  external_ref: string | null;
  requested_by: unknown;
  approved_by: unknown;
  paid_by: unknown;
  bank_ref: unknown;
  created_at: string | null;
}

export interface PostPayoutsByIdMarkPaidResponse {
  id: string;
  vendor_id: string | null;
  amount_minor: number;
  currency: string;
  status: string;
  purpose: unknown;
  external_ref: string | null;
  requested_by: unknown;
  approved_by: unknown;
  paid_by: unknown;
  bank_ref: unknown;
  created_at: string | null;
}

export interface PostPayoutsByIdRejectResponse {
  payout_id: string | null;
  amount_minor: number;
}

export interface PostPayoutsResponse {
  payout_id: string;
  amount_minor: number;
  currency: string;
}

export interface PostPlansResponse {
  id: string;
  name: string;
  amount_minor: number;
  interval_unit: string;
}

export interface PostPlatformChangesByIdAssignResponse {
  id: string;
  assigned_to: string | null;
  assigned_to_name: string | null;
}

export interface PostPlatformChangesByIdDecisionResponse {
  id: string;
  status: string;
  stage: string;
  needs_second_because: string | null;
}

export interface PostPlatformChangesByIdEscalateResponse {
  id: string;
  status: string;
  escalated: boolean;
}

export interface PostPlatformReviewsByOrganisationIdAssignResponse {
  organisation_id: string;
  assigned_to: string | null;
  assigned_to_name: string | null;
}

export interface PostPlatformReviewsByOrganisationIdDecisionResponse {
  organisation_id: string;
  status: string;
  stage: string;
  needs_second_because: string | null;
}

export interface PostPlatformReviewsByOrganisationIdEscalateResponse {
  organisation_id: string;
  status: string;
  escalated: boolean;
}

export interface PostPlatformReviewsByOrganisationIdRestoreResponse {
  organisation_id: string | null;
  status: string;
}

export interface PostPlatformReviewsByOrganisationIdRevokeResponse {
  organisation_id: string | null;
  status: string;
}

export interface PostPlatformRotaDaysOffResponse {
  id: string;
  desk: boolean;
  reviewer_id: string | null;
  first_day: string;
  last_day: string;
  note: string;
}

export interface PostPlatformRotaRebalanceResponse {
  moved: unknown;
  evened: unknown;
  left: unknown;
}

export interface PostProviderAccountsByProviderCheckResponse {
  provider: string;
  ok: boolean;
  detail: unknown;
  env: string;
}

export interface PostProviderAccountsByProviderEnabledResponse {
  provider: string;
  enabled: boolean;
}

export interface PostProviderAccountsResponse {
  id: string;
  provider: string;
  updated: unknown;
  webhook_url: string;
}

export interface PostReconCasesByIdAssignResponse {
  id: string;
  status: string;
  assignee: string;
}

export interface PostReconCasesByIdResolveResponse {
  id: string;
  status: string;
}

export interface PostReconScanResponse {
  new_cases: number;
}

export interface PostRefundsByIdApproveResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostRefundsByIdRefreshResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostRefundsByIdRejectResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostRefundsByIdRetryResponse {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

export interface PostSettingsResponse {
  ok: boolean;
}

export interface PostSettlementsByIdSimulateBankResponse {
  added: number;
  settlements_rechecked: number;
}

export interface PostSettlementsImportResponse {
  settlement: unknown;
  payments: unknown[];
  refunds: unknown[];
  adjustments: unknown[];
  fees: unknown;
  arithmetic: unknown;
  expected_minor: number;
  received_minor: number;
  difference_minor: number;
  difference_breakdown: unknown;
  bank_match: unknown;
  cases: unknown[];
  narrative: boolean;
}

export interface PostSettlementsSyncResponse {
  imported: number;
}

export interface PostSignupResendResponse {
  ok: boolean;
}

export interface PostSignupResponse {
  organisation_id: string | null;
  test_tenant: unknown;
  test_key: unknown;
  email: string;
  verification: string;
}

export interface PostSignupVerifyResponse {
  organisation_id: string | null;
  verified: boolean;
}

export interface PostSimCirclePayResponse {
  status: string;
  remaining_minor: number;
}

export interface PostSimCircleResponse {
  circle_id: string | null;
}

export interface PostSimCircleRevokeResponse {
  status: string;
}

export interface PostSimMandateResponse {
  status: string;
}

export interface PostSimMandateRevokeResponse {
  status: string;
}

export interface PostSimPayResponse {
  status: string;
}

export interface PostSubscriptionsByIdCancelResponse {
  id: string;
  status: string;
}

export interface PostSubscriptionsByIdChangePlanResponse {
  id: string;
  plan_id: string;
  quantity: number;
  proration_minor: boolean;
  proration_invoice_id: string | null;
  remaining_days: number;
  period_days: number;
}

export interface PostSubscriptionsByIdMandateResponse {
  id: string;
  mandate_id: string | null;
}

export interface PostSubscriptionsByIdPauseResponse {
  id: string;
  status: string;
}

export interface PostSubscriptionsByIdResumeResponse {
  id: string;
  status: string;
}

export interface PostSubscriptionsByIdUsageResponse {
  recorded: number;
  quantity: number;
}

export interface PostSubscriptionsResponse {
  id: string;
  status: string;
  next_bill_date: string | null;
}

export interface PostUsersResponse {
  id: string;
  email: string;
  role: string;
}

export interface PostVendorsResponse {
  id: string;
  name: string;
}

export interface PostWebhooksByProviderByTenantIdResponse {
  status: string;
}

export interface PutOnboardingResponse {
  status: string;
}

export interface PutPlatformMeResponse {
  id: string;
  name: string;
  email: string | null;
}

export interface PutPlatformRotaMeResponse {
  shifts: number;
  on_duty: unknown;
}

export interface PutPlatformRotaReviewersByIdResponse {
  reviewer: string;
  name: string;
  shifts: number;
  on_duty: unknown;
}

export interface PutProviderAccountsByProviderResponse {
  id: string;
  provider: string;
  enabled: boolean;
  replaced: unknown;
}

export interface RefundRow {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  currency: string;
  status: string;
  reason: string | null;
  external_ref: string | null;
  requested_by: string;
  approved_by: string | null;
  provider_refund_ref: string | null;
  failure_reason: string | null;
  created_at: string;
  updated_at: string;
}

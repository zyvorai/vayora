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

export interface DeleteCashLagRulesByIdResponse {
  deleted: boolean;
}

export interface DeleteExportsAccountMappingsByIdResponse {
  deleted: boolean;
  ledger_account: string | null;
}

export interface DeleteFeeRulesByIdResponse {
  id: string;
  deleted: boolean;
}

export interface DeleteMailSuppressionsByEmailResponse {
  email: string;
  removed: boolean;
}

export interface DeleteOnboardingUploadsByIdResponse {
  id: string;
  deleted: boolean;
}

export interface DeletePlatformRotaDaysOffByIdResponse {
  id: string;
  deleted: boolean;
}

export interface DeleteSavedViewsByIdResponse {
  ok: boolean;
}

export interface DeleteSettingsLogoResponse {
  email_logo_url: unknown;
}

export interface DeleteTestClockResponse {
  date: unknown;
  frozen: boolean;
  real_date: string | null;
}

export interface GetAcquireKycResponse {
  organisation_id: string | null;
  acquire_kyc_status: string;
  acquire_kyc_approved_at: unknown;
}

export interface GetAnalyticsCostResponse {
  by_provider_method: unknown[];
  suggestions: unknown;
}

export interface GetAnalyticsRefundsResponse {
  from: unknown;
  to: unknown;
  bucket: unknown;
  currency: string;
  series: GetAnalyticsRefundsResponseSeriesItem[];
  totals: unknown;
  time_to_refund: unknown;
  sla: GetAnalyticsRefundsResponseSlaItem[];
  leakage: GetAnalyticsRefundsResponseLeakageItem[];
  note: string;
}

export interface GetAnalyticsRefundsResponseLeakageItem {
  provider: string;
  method: string;
  lines: number;
  settlements: number;
  amount_minor: number;
}

export interface GetAnalyticsRefundsResponseSeriesItem {
  starts: string;
  captured_count: unknown;
  captured_minor: number;
  refunds_requested_count: unknown;
  refunds_requested_minor: number;
  refunds_processed_count: unknown;
  refunds_processed_minor: number;
  refund_rate_bps: unknown;
}

export interface GetAnalyticsRefundsResponseSlaItem {
  status: string;
  count: number;
  amount_minor: number;
  oldest_hours: unknown;
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

export interface GetApiRequestsResponse {
  data: GetApiRequestsResponseDataItem[];
}

export interface GetApiRequestsResponseDataItem {
  id: number;
  actor: string;
  method: string;
  route: string;
  status: number;
  request_id: string;
  latency_ms: number;
  created_at: unknown;
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

export interface GetBankStatementsImportsResponse {
  data: unknown[];
}

export interface GetBankStatementsLayoutsResponse {
  name: string;
  about: unknown;
  mapping: unknown;
  date_formats: unknown;
}

export interface GetBankStatementsLinesResponse {
  data: unknown[];
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

export interface GetCardTokensResponse {
  tokens: unknown[];
}

export interface GetCashDashboardResponse {
  as_of: string | null;
  currency: string;
  matched_window_days: unknown;
  by_provider: unknown;
  totals: unknown;
  payouts_pending: GetCashDashboardResponsePayoutsPendingItem[];
  paid_payouts_without_bank_debit_match: unknown;
  note: string;
}

export interface GetCashDashboardResponsePayoutsPendingItem {
  status: string;
  count: number;
  amount_minor: number;
}

export interface GetCashForecastResponse {
  as_of: string | null;
  currency: string;
  horizon_days: unknown;
  days: GetCashForecastResponseDaysItem[];
  overdue: unknown;
  expected_after_horizon_minor: number;
  expected_without_range_minor: number;
  not_forecast: GetCashForecastResponseNotForecastItem[];
  basis: unknown[];
  note: string;
}

export interface GetCashForecastResponseDaysItem {
  date: unknown;
  expected_minor: number;
  cumulative_low_minor: number;
  cumulative_expected_minor: number;
  cumulative_high_minor: number;
}

export interface GetCashForecastResponseNotForecastItem {
  provider: string;
  method: unknown;
  payments: unknown;
  amount_minor: number;
  status: string;
}

export interface GetCashLagResponse {
  as_of: string | null;
  lookback_days: unknown;
  min_samples: unknown;
  data: unknown[];
  tenant_rules: GetCashLagResponseTenantRulesItem[];
  note: string;
}

export interface GetCashLagResponseTenantRulesItem {
  provider: string;
  method: unknown;
  expected_lag_days: unknown;
}

export interface GetCashLagRulesResponse {
  data: GetCashLagRulesResponseDataItem[];
}

export interface GetCashLagRulesResponseDataItem {
  id: string;
  provider: string;
  method: string;
  expected_lag_days: number;
  note: string;
  created_by: string;
}

export interface GetCashSettingsResponse {
  min_samples: unknown;
  alert_grace_days: unknown;
  refund_sla_hours: unknown;
}

export interface GetCheckoutByTenantIdByIntentIdEventsResponse {
  id: string;
  method: unknown;
  status: string;
  failure_reason: unknown;
  action: unknown;
}

export interface GetCheckoutByTenantIdByIntentIdResponse {
  id: string;
  method: unknown;
  status: string;
  failure_reason: unknown;
  action: unknown;
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
  open_amount: unknown;
  min_amount_minor: number;
  max_amount_minor: number;
  message: string | null;
  brand: unknown;
  allowed_methods: unknown;
  partial: unknown;
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
  intent_id: string;
  created_at: string;
  amount_minor: number;
  reason: string;
  status: string;
  respond_by: string | null;
  evidence_count: number;
}

export interface GetDonateByTenantIdByCampaignIdResponse {
  merchant: string | null;
  name: string;
  description: string | null;
  suggested_minor: number;
  min_minor: number;
  max_minor: number;
  currency: string;
  recurring_enabled: unknown;
}

export interface GetDonationsCampaignsByIdSummaryResponse {
  campaign_id: string;
  paid_count: number;
  paid_minor: number;
  refunded_minor: number;
  net_minor: number;
  pending_count: number;
  goal_minor: number | null;
  recurring_pledges: number;
}

export interface GetDonationsCampaignsResponse {
  data: unknown;
}

export interface GetDonationsResponse {
  data: GetDonationsResponseDataItem[];
}

export interface GetDonationsResponseDataItem {
  id: string;
  campaign_id: string | null;
  intent_id: string | null;
  amount_minor: number;
  status: string;
  donor: unknown;
  anonymous: unknown;
  receipt_no: unknown;
  created_at: string | null;
  paid_at: string | null;
  refunded_minor: number;
}

export interface GetDuesByKindGroupsResponse {
  data: GetDuesByKindGroupsResponseDataItem[];
}

export interface GetDuesByKindGroupsResponseDataItem {
  id: string;
  name: string;
  members: number;
  heads_total_minor: number;
}

export interface GetDuesByKindHeadsResponse {
  data: GetDuesByKindHeadsResponseDataItem[];
}

export interface GetDuesByKindHeadsResponseDataItem {
  id: string;
  group_id: string;
  name: string;
  amount_minor: number;
}

export interface GetDuesByKindMembersResponse {
  data: GetDuesByKindMembersResponseDataItem[];
}

export interface GetDuesByKindMembersResponseDataItem {
  id: string;
  group_id: string | null;
  group: unknown;
  member_ref: unknown;
  name: string;
  active: unknown;
  payer: GetDuesByKindMembersResponseDataItemPayerItem[];
}

export interface GetDuesByKindMembersResponseDataItemPayerItem {
  name: string;
  email: string;
  phone: string | null;
}

export interface GetDuesByKindReportCsvResponse {
  kind: string;
  as_of: string | null;
  totals: unknown;
  data: unknown;
}

export interface GetDuesByKindReportResponse {
  kind: string;
  as_of: string | null;
  totals: unknown;
  data: unknown;
}

export interface GetDuesByKindSettingsResponse {
  kind: string;
  receipt_note: string;
}

export interface GetEscrowProofResponse {
  escrow_available_net: number;
  open_payables_net: number;
  balanced: unknown;
}

export interface GetEventsByIdAttendeesResponse {
  data: unknown[];
  has_more: boolean;
}

export interface GetEventsByIdCheckinExportResponse {
  event: unknown;
  generated_at: string | null;
  count: number;
  tickets: unknown[];
}

export interface GetEventsByIdCheckinStatsResponse {
  checked_in: unknown;
  expected: unknown;
  refunded: unknown;
  void: unknown;
  recent_duplicates: GetEventsByIdCheckinStatsResponseRecentDuplicatesItem[];
}

export interface GetEventsByIdCheckinStatsResponseRecentDuplicatesItem {
  code: string;
  scanned_by: string;
  scanned_at: string;
}

export interface GetEventsByIdOrdersByOrderIdResponse {
  id: string;
  code: string;
  status: string;
  tier_id: string | null;
}

export interface GetEventsByIdOrdersResponse {
  data: unknown;
  has_more: boolean;
}

export interface GetEventsByIdRefundBatchesByBatchIdResponse {
  id: string;
  event_id: string | null;
  kind: string;
  reason: unknown;
  requested_by: unknown;
  created_at: string | null;
  orders: unknown;
  queued: unknown;
  requested: unknown;
  errors: unknown;
  refunds_by_status: unknown;
  amount_minor_by_status: unknown;
  finished: unknown;
  needs_attention: unknown;
}

export interface GetEventsByIdRefundReportResponse {
  event_id: string;
  currency: string;
  orders_to_refund: number;
  amount_minor: number;
  processed: unknown;
  in_progress: unknown;
  awaiting_approval: unknown;
  failed_or_rejected: unknown;
  not_yet_sent: unknown;
  errors: unknown;
  batches: unknown;
  items: GetEventsByIdRefundReportResponseItemsItem[];
}

export interface GetEventsByIdRefundReportResponseItemsItem {
  order_id: string;
  buyer_name: string;
  buyer_email: string;
  amount_minor: number;
  state: string;
  refund_id: string | null;
  refund_status: string | null;
  error: string | null;
  failure_reason: string | null;
}

export interface GetEventsByIdResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface GetEventsCatalogueResponse {
  data: GetEventsCatalogueResponseDataItem[];
}

export interface GetEventsCatalogueResponseDataItem {
  type: unknown;
  description: string | null;
}

export interface GetEventsResponse {
  data: unknown[];
  has_more: boolean;
}

export interface GetExportsAccountMappingsResponse {
  defaults: GetExportsAccountMappingsResponseDefaultsItem[];
  overrides: GetExportsAccountMappingsResponseOverridesItem[];
  effective: GetExportsAccountMappingsResponseEffectiveItem[];
  unmapped_code: unknown;
  note: string;
}

export interface GetExportsAccountMappingsResponseDefaultsItem {
  ledger_account: unknown;
  code: string;
  name: string;
}

export interface GetExportsAccountMappingsResponseEffectiveItem {
  ledger_account: unknown;
  code: string;
  name: string;
  mapped: unknown;
}

export interface GetExportsAccountMappingsResponseOverridesItem {
  id: number;
  ledger_account: string;
  code: string;
  name: string;
  updated_by: string;
}

export interface GetFeeChecksFindingsResponse {
  data: unknown[];
  note: string;
}

export interface GetFeeChecksSettingsResponse {
  tolerance_minor: number;
  owner: unknown;
}

export interface GetFeeChecksSummaryResponse {
  rules_configured: number;
  open_findings: unknown;
  open_cases: number;
  over_deducted_minor: number;
  under_deducted_minor: number;
  total_variance_minor: number;
  total_absolute_variance_minor: number;
  by_provider_method: GetFeeChecksSummaryResponseByProviderMethodItem[];
  lines_without_a_rule: number;
  message: string | null;
  note: string;
}

export interface GetFeeChecksSummaryResponseByProviderMethodItem {
  provider: string;
  method: string;
  open_findings: number;
  settlements: number;
  over_deducted_minor: number;
  under_deducted_minor: number;
  total_variance_minor: number;
}

export interface GetFeeRulesResponse {
  data: unknown;
  note: string;
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
  details: unknown;
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
  customer_id: string | null;
  customer_name: string | null;
  details: unknown;
}

export interface GetMailSuppressionsResponse {
  data: GetMailSuppressionsResponseDataItem[];
}

export interface GetMailSuppressionsResponseDataItem {
  email: string;
  source: string;
  created_at: unknown;
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

export interface GetMeResponse {
  tenant_id: string;
  env: string;
  key_id: string;
  actor: string;
  scopes: string[];
  branch_id: string | null;
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

export interface GetMoneyModeResponse {
  money_mode: unknown;
  acquire_kyc_status: string;
  settlement_delay_days: number;
}

export interface GetOnboardingResponse {
  automated_checks: unknown;
  review_reasons: string[];
  checks: GetOnboardingResponseChecksItem[];
  status: string;
  profile: GetOnboardingResponseProfileItem[];
  documents: GetOnboardingResponseDocumentsItem[];
  uploads: unknown;
  document_reader_available: unknown;
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

export interface GetOnboardingUploadsResponse {
  data: unknown;
  reader_available: unknown;
}

export interface GetOutboxByIdDeliveriesResponse {
  data: GetOutboxByIdDeliveriesResponseDataItem[];
}

export interface GetOutboxByIdDeliveriesResponseDataItem {
  id: number;
  attempt_no: number;
  ok: boolean;
  http_status: number | null;
  error: string | null;
  latency_ms: number | null;
  created_at: unknown;
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

export interface GetPaymentLinksAnalyticsResponse {
  from: unknown;
  to: unknown;
  totals: unknown;
  daily: unknown;
  funnel: unknown;
  notes: unknown;
}

export interface GetPaymentLinksByIdAnalyticsResponse {
  from: unknown;
  to: unknown;
  totals: unknown;
  daily: unknown;
  funnel: unknown;
  notes: unknown;
}

export interface GetPaymentLinksByIdShareResponse {
  id: string;
  url: unknown;
  text: unknown;
  sms_text: unknown;
  email_subject: unknown;
  qr_path: string;
  can_email: unknown;
  can_sms: unknown;
  sms_mode: unknown;
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
  trial_days: number;
  included_units: number;
  included_per_seat: boolean;
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

export interface GetPublicEventsByTenantIdByEventIdResponse {
  id: string;
  merchant: unknown;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  on_sale: unknown;
  hold_minutes: unknown;
  tiers: unknown[];
  form_fields: GetPublicEventsByTenantIdByEventIdResponseFormFieldsItem[];
}

export interface GetPublicEventsByTenantIdByEventIdResponseFormFieldsItem {
  key: string;
  label: unknown;
  kind: string;
  required: unknown;
  options: unknown;
  min_len: unknown;
  max_len: unknown;
}

export interface GetPublicEventsByTenantIdOrdersByOrderIdResponse {
  order_id: string | null;
  status: string;
  merchant: unknown;
  event: unknown;
  buyer: unknown;
  quantity: number;
  subtotal_minor: number;
  discount_minor: number;
  total_minor: number;
  currency: string;
  intent_id: string | null;
  hold_expires_at: string | null;
  paid_at: string | null;
  lines: GetPublicEventsByTenantIdOrdersByOrderIdResponseLinesItem[];
  tickets: GetPublicEventsByTenantIdOrdersByOrderIdResponseTicketsItem[];
}

export interface GetPublicEventsByTenantIdOrdersByOrderIdResponseLinesItem {
  tier: string;
  quantity: number;
  unit_price_minor: number;
}

export interface GetPublicEventsByTenantIdOrdersByOrderIdResponseTicketsItem {
  id: string;
  code: string;
  status: string;
  tier: string;
  checked_in_at: string | null;
  qr_payload: unknown;
  qr_path: string;
}

export interface GetReconAgeingResponse {
  bucket_days: unknown;
  open: unknown;
  unassigned: unknown;
  oldest_days: unknown;
  by_kind: unknown;
}

export interface GetReconCasesByIdSuggestionsResponse {
  case_id: string;
  kind: string;
  summary: string;
  ai_summary: string | null;
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

export interface GetRoutingAutomationResponse {
  settings: unknown;
  effective: unknown;
}

export interface GetRoutingHealthResponse {
  config: unknown;
  automation_paused: unknown;
  routing_health_rule: unknown;
  providers: unknown;
  recent_changes: unknown[];
}

export interface GetRoutingScorecardResponse {
  window_days: unknown;
  method: unknown;
  success_rate_rule: string;
  median_rule: string;
  scorecard: unknown;
}

export interface GetRoutingShadowReportResponse {
  window_days: unknown;
  method: unknown;
  shadow: unknown;
  decisions: number;
  agreed: number;
  differed: unknown;
  no_route_in_shadow: number;
  agreement_rate_bps: unknown;
  differences: GetRoutingShadowReportResponseDifferencesItem[];
  outcome_by_provider: GetRoutingShadowReportResponseOutcomeByProviderItem[];
  recent_differences: GetRoutingShadowReportResponseRecentDifferencesItem[];
  limits: string;
}

export interface GetRoutingShadowReportResponseDifferencesItem {
  method: string;
  applied_provider: string;
  shadow_provider: string | null;
  decisions: number;
  fee_comparable: number;
  estimated_fee_difference_minor: number;
}

export interface GetRoutingShadowReportResponseOutcomeByProviderItem {
  provider: string;
  method: string;
  payments_sent: number;
  succeeded: number;
  provider_failures: number;
  payer_declines: number;
  unresolved: number;
  of_which_shadow_would_have_chosen_another: number;
}

export interface GetRoutingShadowReportResponseRecentDifferencesItem {
  attempt_id: string;
  method: string;
  amount_minor: number;
  applied_provider: string;
  shadow_provider: string | null;
  applied_explanation: string | null;
  shadow_explanation: string | null;
  applied_fee_minor: number | null;
  shadow_fee_minor: number | null;
  attempt_status: string;
  at: string;
}

export interface GetSavedViewsResponse {
  data: unknown[];
}

export interface GetSettingsResponse {
  refund_approval_threshold_minor: number;
  routing: unknown;
  outbound_webhook_url: string | null;
  env: string;
  dunning: unknown;
  gst: unknown;
  payout_approval_threshold_minor: number;
  payment_receipts: boolean;
  reply_to_email: string | null;
  reminder_emails: boolean;
  invoice_emails: boolean;
  email_accent: string | null;
  email_logo_url: string | null;
  risk: unknown;
  webhook_events: unknown;
  return_url_origins: unknown;
}

export interface GetSettlementBatchesByIdResponse {
  id: string;
  currency: string;
  status: string;
  captures_minor: number;
  refunds_minor: number;
  fees_minor: number;
  holds_minor: number;
  net_minor: number;
  rail: unknown;
  rail_ref: unknown;
  created_at: string | null;
  released_at: string | null;
  items: GetSettlementBatchesByIdResponseItemsItem[];
}

export interface GetSettlementBatchesByIdResponseItemsItem {
  id: string;
  kind: string;
  reference: string;
  amount_minor: number;
  currency: string;
}

export interface GetSettlementBatchesResponse {
  data: unknown[];
}

export interface GetSettlementsByIdExplainResponse {
  settlement: unknown;
  payments: unknown[];
  refunds: unknown[];
  adjustments: unknown[];
  fees: unknown;
  fee_check: unknown;
  arithmetic: unknown;
  expected_minor: number;
  received_minor: number;
  difference_minor: number;
  difference_breakdown: unknown;
  bank_match: unknown;
  bank_difference: unknown;
  suggested_bank_matches: unknown;
  cases: unknown[];
  narrative: boolean;
}

export interface GetSettlementsByIdFeeCheckResponse {
  attempt_id: string | null;
  provider: string;
  method: unknown;
  status: string;
  payment_minor: number;
  refunded_minor: number;
  actual_minor: number;
  expected_minor: number;
  variance_minor: number;
  rule: unknown;
}

export interface GetSettlementsResponse {
  data: unknown[];
}

export interface GetStorefrontItemsResponse {
  data: unknown;
}

export interface GetStorefrontOrdersByIdResponse {
  item_id: string | null;
  name: string;
  unit_minor: number;
  quantity: number;
}

export interface GetStorefrontOrdersResponse {
  data: unknown;
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
  unbilled_usage: number;
  unit_amount_minor: number | null;
  trial_ends_on: string | null;
  billed: boolean;
}

export interface GetTestClockResponse {
  date: unknown;
  frozen: unknown;
  real_date: string | null;
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

export interface PatchDonationsCampaignsByIdResponse {
  id: string;
  name: string;
  description: string | null;
  suggested_minor: number;
  min_minor: number;
  max_minor: number;
  goal_minor: number;
  receipt_fields: unknown;
  recurring_enabled: unknown;
  status: string;
  created_at: string | null;
}

export interface PatchPlansByIdResponse {
  id: string;
  name: string;
  amount_minor: number;
  active: unknown;
  trial_days: unknown;
  included_units: unknown;
  included_per_seat: unknown;
  subscribers_kept_on_old_price: unknown;
}

export interface PatchStorefrontItemsByIdResponse {
  id: string;
  name: string;
  description: string | null;
  price_minor: number;
  image_id: string | null;
  stock: unknown;
  active: unknown;
  position: unknown;
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
  duplicates: unknown;
  rows: unknown;
  settlements_rechecked: number;
  matched: unknown;
  cases_opened: unknown;
}

export interface PostBankStatementsMatchesByIdConfirmResponse {
  id: string;
  status: string;
  bank_line_id: string | null;
  subject_type: unknown;
  subject_id: string | null;
  rule: unknown;
  decided_by: unknown;
}

export interface PostBankStatementsMatchesByIdRejectResponse {
  id: string;
  status: string;
}

export interface PostBankStatementsPreviewResponse {
  ok: boolean;
  layout: unknown;
  header_line: unknown;
  rows: number;
  new_rows: unknown;
  duplicate_rows: number;
  credits: unknown;
  debits: unknown;
  period_from: unknown;
  period_to: unknown;
  already_imported_as: number | null;
  errors: unknown;
  warnings: unknown;
  sample: unknown[];
}

export interface PostBankStatementsRematchResponse {
  settlements_rechecked: number;
  matched: unknown;
  cases_opened: unknown;
}

export interface PostBillingRunResponse {
  invoices_created: number;
}

export interface PostBranchesResponse {
  id: string;
  name: string;
}

export interface PostCardTokensByIdRevokeResponse {
  id: string;
  revoked: boolean;
}

export interface PostCardTokensResponse {
  id: string;
  last4: unknown;
  brand: unknown;
  exp_month: unknown;
  exp_year: unknown;
}

export interface PostCashAlertsScanResponse {
  pending_cases_opened: unknown;
  late_cases_opened: unknown;
  case_kinds: unknown;
}

export interface PostCashEntriesResponse {
  id: string;
}

export interface PostCashLagRulesResponse {
  id: string;
  provider: string;
  method: unknown;
  expected_lag_days: number;
  note: string | null;
}

export interface PostCashSettingsResponse {
  min_samples: unknown;
  alert_grace_days: unknown;
  refund_sla_hours: unknown;
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

export interface PostDonateByTenantIdByCampaignIdStartResponse {
  donation_id: string | null;
  intent_id: string | null;
  client_secret: string;
  amount_minor: number;
  currency: string;
  path: string;
}

export interface PostDonationsCampaignsByIdPledgesResponse {
  id: string;
  customer_id: string | null;
  mandate: unknown;
}

export interface PostDonationsCampaignsResponse {
  id: string;
  name: string;
  description: string | null;
  suggested_minor: number;
  min_minor: number;
  max_minor: number;
  goal_minor: number;
  receipt_fields: unknown;
  recurring_enabled: unknown;
  status: string;
  created_at: string | null;
}

export interface PostDuesByKindAssignResponse {
  created: unknown;
  already_assigned: unknown;
  skipped_no_payer: unknown;
  skipped_no_fee_heads: unknown;
}

export interface PostDuesByKindGroupsResponse {
  id: string;
  name: string;
  kind: string;
}

export interface PostDuesByKindHeadsResponse {
  id: string;
  group_id: string;
  name: string;
  amount_minor: number;
}

export interface PostDuesByKindMembersImportResponse {
  saved: boolean;
  rows: number;
  problems: unknown;
}

export interface PostDuesByKindMembersResponse {
  id: string;
  created: unknown;
}

export interface PostDuesByKindRemindersResponse {
  queued: unknown;
  skipped_no_email: unknown;
  skipped_already_sent_today: unknown;
}

export interface PostEventsByIdCancelResponse {
  event_id: string;
  status: string;
  already_cancelled: boolean;
  refund_batch: unknown;
}

export interface PostEventsByIdCheckinResponse {
  result: unknown;
  valid: boolean;
  code: string;
  manual_entry: unknown;
  ticket: unknown;
  checked_in_at: string | null;
  checked_in_by: unknown;
  message: string | null;
}

export interface PostEventsByIdCheckinSyncResponse {
  counts: unknown;
  results: unknown;
}

export interface PostEventsByIdOrdersByOrderIdCancelResponse {
  order: unknown;
  refund_batch: unknown;
}

export interface PostEventsByIdRefundBatchesByBatchIdProcessResponse {
  id: string;
  event_id: string | null;
  kind: string;
  reason: unknown;
  requested_by: unknown;
  created_at: string | null;
  orders: unknown;
  queued: unknown;
  requested: unknown;
  errors: unknown;
  refunds_by_status: unknown;
  amount_minor_by_status: unknown;
  finished: unknown;
  needs_attention: unknown;
}

export interface PostEventsByIdStatusResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PostEventsByIdTiersByTierIdArchiveResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PostEventsByIdTiersResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PostEventsResponse {
  vendor_id: string | null;
  share_bp: unknown;
}

export interface PostExportsAccountMappingsResponse {
  id: number;
  ledger_account: string;
  code: string;
  name: string;
}

export interface PostFeeChecksRunResponse {
  settlements_checked: number;
  flagged: unknown;
  ok: boolean;
  no_rule: unknown;
}

export interface PostFeeChecksSettingsResponse {
  tolerance_minor: number;
  owner: boolean;
}

export interface PostFeeRulesResponse {
  id: string;
  provider: string;
  method: unknown;
  bps: unknown;
  flat_minor: number;
  cap_minor: number;
  gst_bps: unknown;
  refund_returns_fee: unknown;
  effective_from: unknown;
  note: string | null;
  created_by: unknown;
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

export interface PostInvoicesByIdSendResponse {
  id: string;
  queued: boolean;
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

export interface PostOnboardingUploadsResponse {
  id: string;
  kind: string;
  filename: unknown;
  content_type: unknown;
  size_bytes: unknown;
  read_method: unknown;
  facts: unknown;
  checks: unknown;
  uploaded_by: unknown;
  created_at: string | null;
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

export interface PostPaymentLinksBulkResponse {
  import_id: string | null;
  repeat: boolean;
  dry_run: boolean;
  rows: number;
  created: unknown;
  existing: unknown;
  links: unknown;
  export_path: string;
}

export interface PostPaymentLinksByIdCancelResponse {
  id: string;
  status: string;
}

export interface PostPaymentLinksByIdSendResponse {
  id: string;
  queued: boolean;
}

export interface PostPaymentLinksByIdSendSmsResponse {
  id: string;
  queued: boolean;
  mode: unknown;
}

export interface PostPaymentLinksResponse {
  id: string;
  token: unknown;
  currency: string;
  path: string;
  max_uses: number | null;
  expires_at: string | null;
  existing: unknown;
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

export interface PostPayoutsByIdSendRailResponse {
  payout_id: string;
  status: string;
  bank_ref: unknown;
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

export interface PostPublicEventsByTenantIdOrdersByOrderIdCancelResponse {
  order_id: string | null;
  status: string;
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
  settlements_without_bank_credit: unknown;
  payouts_without_bank_debit: unknown;
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

export interface PostRoutingAutomationPauseResponse {
  settings: unknown;
  effective: unknown;
}

export interface PostRoutingAutomationResumeResponse {
  settings: unknown;
  effective: unknown;
}

export interface PostSavedViewsResponse {
  id: string;
  name: string;
  query: unknown;
  shared: unknown;
  mine: boolean;
  owner: unknown;
}

export interface PostSettingsLogoResponse {
  email_logo_url: string;
}

export interface PostSettingsResponse {
  ok: boolean;
}

export interface PostSettlementBatchesByIdItemsResponse {
  id: string;
  batch_id: string;
  kind: string;
  amount_minor: number;
}

export interface PostSettlementBatchesByIdReleaseResponse {
  batch_id: string | null;
  net_minor: number;
  rail_ref: unknown;
}

export interface PostSettlementBatchesResponse {
  id: string;
  status: string;
  currency: string;
}

export interface PostSettlementsByIdFeeCheckResponse {
  attempt_id: string | null;
  provider: string;
  method: unknown;
  status: string;
  payment_minor: number;
  refunded_minor: number;
  actual_minor: number;
  expected_minor: number;
  variance_minor: number;
  rule: unknown;
}

export interface PostSettlementsByIdSimulateBankResponse {
  added: number;
  duplicates: unknown;
  rows: unknown;
  settlements_rechecked: number;
  matched: unknown;
  cases_opened: unknown;
}

export interface PostSettlementsImportResponse {
  settlement: unknown;
  payments: unknown[];
  refunds: unknown[];
  adjustments: unknown[];
  fees: unknown;
  fee_check: unknown;
  arithmetic: unknown;
  expected_minor: number;
  received_minor: number;
  difference_minor: number;
  difference_breakdown: unknown;
  bank_match: unknown;
  bank_difference: unknown;
  suggested_bank_matches: unknown;
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

export interface PostSimFailNextRefundResponse {
  ok: boolean;
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

export interface PostStoreBySlugOrdersResponse {
  order_id: string | null;
  code: string;
  intent_id: string | null;
  client_secret: string;
  amount_minor: number;
  currency: string;
  tenant_id: string | null;
  path: string;
  hold_minutes: unknown;
}

export interface PostStorefrontImagesResponse {
  id: string;
  content_type: unknown;
  size_bytes: number;
}

export interface PostStorefrontItemsResponse {
  id: string;
  name: string;
  description: string | null;
  price_minor: number;
  image_id: string | null;
  stock: unknown;
  active: unknown;
  position: unknown;
}

export interface PostStorefrontOrdersByIdCancelResponse {
  item_id: string | null;
  name: string;
  unit_minor: number;
  quantity: number;
}

export interface PostStorefrontOrdersByIdFulfillResponse {
  item_id: string | null;
  name: string;
  unit_minor: number;
  quantity: number;
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

export interface PostSubscriptionsByIdTrialResponse {
  id: string;
  trial_ends_on: string;
  next_bill_date: string;
}

export interface PostSubscriptionsByIdUsageResponse {
  recorded: number;
  quantity: number;
}

export interface PostSubscriptionsResponse {
  id: string;
  status: string;
  next_bill_date: string | null;
  trial_ends_on: string | null;
}

export interface PostTestClockResponse {
  date: unknown;
  frozen: boolean;
  real_date: string | null;
}

export interface PostTestDataResetResponse {
  ok: boolean;
  removed: unknown;
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

export interface PutAcquireKycResponse {
  organisation_id: string | null;
  acquire_kyc_status: string;
}

export interface PutDuesByKindSettingsResponse {
  kind: string;
  receipt_note: unknown;
}

export interface PutEventsByIdFormFieldsResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PutEventsByIdResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PutEventsByIdTiersByTierIdResponse {
  id: string;
  name: string;
  description: string | null;
  mode: unknown;
  venue: unknown;
  online_url: unknown;
  starts_at: string | null;
  ends_at: string | null;
  status: string;
  currency: string;
  capacity: unknown;
  hold_minutes: unknown;
  seats_held: unknown;
  seats_sold: unknown;
  seats_available: unknown;
  splits: unknown;
  created_at: string | null;
  cancelled_at: string | null;
  tiers: unknown;
  form_fields: unknown;
  tickets: unknown;
  orders: unknown;
}

export interface PutMoneyModeResponse {
  money_mode: unknown;
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

export interface PutRoutingAutomationResponse {
  settings: unknown;
  effective: unknown;
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

// Code generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. DO NOT EDIT.

package zyvor

import "encoding/json"

// Decode turns the map a call returns into one of the response types:
//
//	plans, err := zyvor.Decode[zyvor.GetPlansResponse](c.ListPlans())
func Decode[T any](m map[string]any, err error) (T, error) {
	var out T
	if err != nil {
		return out, err
	}
	b, err := json.Marshal(m)
	if err != nil {
		return out, err
	}
	err = json.Unmarshal(b, &out)
	return out, err
}

type AttemptRow struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type DebitRow struct {
	ID            string  `json:"id"`
	MandateID     string  `json:"mandate_id"`
	AmountMinor   int64   `json:"amount_minor"`
	Currency      string  `json:"currency"`
	DueDate       string  `json:"due_date"`
	NotifiedAt    *string `json:"notified_at"`
	InvoiceID     *string `json:"invoice_id"`
	State         string  `json:"state"`
	AttemptID     *string `json:"attempt_id"`
	RetryNo       int64   `json:"retry_no"`
	FailureReason *string `json:"failure_reason"`
	CreatedAt     string  `json:"created_at"`
}

type DeletePlatformRotaDaysOffByIdResponse struct {
	ID      string `json:"id"`
	Deleted bool   `json:"deleted"`
}

type GetAnalyticsCostResponse struct {
	ByProviderMethod []any `json:"by_provider_method"`
	Suggestions      any   `json:"suggestions"`
}

type GetApiKeysResponse struct {
	Data []GetApiKeysResponseDataItem `json:"data"`
}

type GetApiKeysResponseDataItem struct {
	BranchID     *string  `json:"branch_id"`
	ID           string   `json:"id"`
	Name         string   `json:"name"`
	Prefix       string   `json:"prefix"`
	Scopes       []string `json:"scopes"`
	CreatedAt    any      `json:"created_at"`
	RevokedAt    any      `json:"revoked_at"`
	ExpiresAt    any      `json:"expires_at"`
	LastUsedAt   any      `json:"last_used_at"`
	AllowedCidrs []string `json:"allowed_cidrs"`
}

type GetAuditEventsResponse struct {
	Data []GetAuditEventsResponseDataItem `json:"data"`
}

type GetAuditEventsResponseDataItem struct {
	ID        int64   `json:"id"`
	Actor     string  `json:"actor"`
	Action    string  `json:"action"`
	Subject   *string `json:"subject"`
	Detail    any     `json:"detail"`
	CreatedAt any     `json:"created_at"`
}

type GetAuditEventsVerifyResponse struct {
	Ok           bool `json:"ok"`
	SealsChecked any  `json:"seals_checked"`
	BrokenSeal   any  `json:"broken_seal"`
	EventsRange  any  `json:"events_range"`
	Reason       any  `json:"reason"`
}

type GetBranchesByIdSummaryResponse struct {
	Branch any   `json:"branch"`
	From   any   `json:"from"`
	To     any   `json:"to"`
	Data   []any `json:"data"`
}

type GetBranchesResponse struct {
	Data []GetBranchesResponseDataItem `json:"data"`
}

type GetBranchesResponseDataItem struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type GetCheckoutByTenantIdByIntentIdResponse struct {
	Merchant    *string `json:"merchant"`
	AmountMinor int64   `json:"amount_minor"`
	Currency    string  `json:"currency"`
	Description *string `json:"description"`
	Status      string  `json:"status"`
	Phase       any     `json:"phase"`
	Methods     []any   `json:"methods"`
	UPIFlow     any     `json:"upi_flow"`
}

type GetCheckoutLinksByTenantIdByTokenResponse struct {
	Merchant       *string `json:"merchant"`
	Description    *string `json:"description"`
	AmountMinor    int64   `json:"amount_minor"`
	DiscountMinor  int64   `json:"discount_minor"`
	FeeMinor       int64   `json:"fee_minor"`
	TotalMinor     int64   `json:"total_minor"`
	CouponApplied  any     `json:"coupon_applied"`
	Currency       string  `json:"currency"`
	Status         string  `json:"status"`
	OpenAmount     bool    `json:"open_amount"`
	MinAmountMinor *int64  `json:"min_amount_minor"`
	MaxAmountMinor *int64  `json:"max_amount_minor"`
}

type GetClosingsResponse struct {
	Data []any `json:"data"`
}

type GetCollectionsAgeingResponse struct {
	AsOf                     *string `json:"as_of"`
	Data                     []any   `json:"data"`
	TotalOutstandingInrMinor *int64  `json:"total_outstanding_inr_minor"`
	UsdRateDate              *string `json:"usd_rate_date"`
}

type GetCollectionsDraftsResponse struct {
	Data []any `json:"data"`
}

type GetCollectionsPromisesResponse struct {
	Data []any `json:"data"`
}

type GetConsoleBranchesResponse struct {
	Data []GetConsoleBranchesResponseDataItem `json:"data"`
}

type GetConsoleBranchesResponseDataItem struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type GetConsoleInvitationsResponse struct {
	Data []GetConsoleInvitationsResponseDataItem `json:"data"`
}

type GetConsoleInvitationsResponseDataItem struct {
	ID        string  `json:"id"`
	Email     string  `json:"email"`
	Role      string  `json:"role"`
	ExpiresAt any     `json:"expires_at"`
	InvitedBy *string `json:"invited_by"`
}

type GetConsoleUsersResponse struct {
	Data []any `json:"data"`
}

type GetCountersResponse struct {
	Data []GetCountersResponseDataItem `json:"data"`
}

type GetCountersResponseDataItem struct {
	ID       string `json:"id"`
	BranchID string `json:"branch_id"`
	Name     string `json:"name"`
	Active   bool   `json:"active"`
}

type GetCouponsResponse struct {
	Data []any `json:"data"`
}

type GetCustomersByIdResponse struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Email       string  `json:"email"`
	Phone       *string `json:"phone"`
	ExternalRef *string `json:"external_ref"`
	GSTIN       *string `json:"gstin"`
	Address1    *string `json:"address1"`
	Location    *string `json:"location"`
	Pincode     *string `json:"pincode"`
}

type GetCustomersByIdStatementResponse struct {
	CustomerID   string  `json:"customer_id"`
	Name         *string `json:"name"`
	Lines        []any   `json:"lines"`
	BalanceMinor any     `json:"balance_minor"`
}

type GetCustomersResponse struct {
	Data []GetCustomersResponseDataItem `json:"data"`
}

type GetCustomersResponseDataItem struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Email       *string `json:"email"`
	Phone       *string `json:"phone"`
	ExternalRef *string `json:"external_ref"`
	GSTIN       *string `json:"gstin"`
	Address1    *string `json:"address1"`
	Location    *string `json:"location"`
	Pincode     *string `json:"pincode"`
}

type GetDisputesResponse struct {
	Data []GetDisputesResponseDataItem `json:"data"`
}

type GetDisputesResponseDataItem struct {
	ID            string  `json:"id"`
	AttemptID     string  `json:"attempt_id"`
	AmountMinor   int64   `json:"amount_minor"`
	Reason        string  `json:"reason"`
	Status        string  `json:"status"`
	RespondBy     *string `json:"respond_by"`
	EvidenceCount int64   `json:"evidence_count"`
}

type GetEventsCatalogueResponse struct {
	Data []GetEventsCatalogueResponseDataItem `json:"data"`
}

type GetEventsCatalogueResponseDataItem struct {
	Type        any     `json:"type"`
	Description *string `json:"description"`
}

type GetFxConvertResponse struct {
	AmountMinor int64  `json:"amount_minor"`
	Currency    string `json:"currency"`
	Rate        any    `json:"rate"`
}

type GetFxExposureResponse struct {
	AsOf                *string `json:"as_of"`
	RateDate            *string `json:"rate_date"`
	OutstandingUsdMinor int64   `json:"outstanding_usd_minor"`
	Complete            any     `json:"complete"`
	BookedInrMinor      int64   `json:"booked_inr_minor"`
	CurrentInrMinor     int64   `json:"current_inr_minor"`
	UnrealisedInrMinor  int64   `json:"unrealised_inr_minor"`
	Note                string  `json:"note"`
	Data                any     `json:"data"`
}

type GetFxRatesResponse struct {
	Data []any `json:"data"`
}

type GetGstGstr1Response struct {
	Note      string                       `json:"note"`
	Period    any                          `json:"period"`
	GSTIN     *string                      `json:"gstin"`
	LegalName *string                      `json:"legal_name"`
	B2b       []GetGstGstr1ResponseB2bItem `json:"b2b"`
	B2cl      any                          `json:"b2cl"`
	B2cs      any                          `json:"b2cs"`
	Cdnr      []any                        `json:"cdnr"`
	Cdnur     []any                        `json:"cdnur"`
	Hsn       any                          `json:"hsn"`
	Totals    any                          `json:"totals"`
}

type GetGstGstr1ResponseB2bItem struct {
	BuyerGSTIN int64 `json:"buyer_gstin"`
	Invoices   any   `json:"invoices"`
}

type GetInboxResponse struct {
	Data []GetInboxResponseDataItem `json:"data"`
}

type GetInboxResponseDataItem struct {
	ID         int64  `json:"id"`
	Provider   string `json:"provider"`
	EventID    string `json:"event_id"`
	Kind       string `json:"kind"`
	Outcome    string `json:"outcome"`
	ReceivedAt any    `json:"received_at"`
}

type GetInsightsCashflowResponse struct {
	AsOf                      *string                                `json:"as_of"`
	Currency                  string                                 `json:"currency"`
	Weeks                     []GetInsightsCashflowResponseWeeksItem `json:"weeks"`
	OverdueMinor              int64                                  `json:"overdue_minor"`
	DueLaterMinor             int64                                  `json:"due_later_minor"`
	NoDueDateMinor            int64                                  `json:"no_due_date_minor"`
	PromisedMinor             int64                                  `json:"promised_minor"`
	PayoutsWaitingMinor       int64                                  `json:"payouts_waiting_minor"`
	DailyReceiptsRunRateMinor int64                                  `json:"daily_receipts_run_rate_minor"`
	Note                      string                                 `json:"note"`
}

type GetInsightsCashflowResponseWeeksItem struct {
	Starts   any   `json:"starts"`
	DueMinor int64 `json:"due_minor"`
}

type GetInsightsRefundAnomaliesResponse struct {
	RefundsLast7Days    int64                                         `json:"refunds_last_7_days"`
	WeeklyAverageBefore any                                           `json:"weekly_average_before"`
	Flags               []GetInsightsRefundAnomaliesResponseFlagsItem `json:"flags"`
	Note                string                                        `json:"note"`
}

type GetInsightsRefundAnomaliesResponseFlagsItem struct {
	Code    string  `json:"code"`
	Message *string `json:"message"`
}

type GetInstallmentPlansByIdResponse struct {
	ID             string  `json:"id"`
	CustomerID     *string `json:"customer_id"`
	Name           string  `json:"name"`
	TotalMinor     int64   `json:"total_minor"`
	Currency       string  `json:"currency"`
	Instalments    any     `json:"instalments"`
	ExternalRef    *string `json:"external_ref"`
	SettledMinor   int64   `json:"settled_minor"`
	RemainingMinor int64   `json:"remaining_minor"`
	Invoices       []any   `json:"invoices"`
}

type GetInvoicesByIdEinvoiceResponse struct {
	Draft      bool   `json:"draft"`
	Note       string `json:"note"`
	Version    string `json:"Version"`
	TranDtls   any    `json:"TranDtls"`
	DocDtls    any    `json:"DocDtls"`
	SellerDtls any    `json:"SellerDtls"`
	BuyerDtls  any    `json:"BuyerDtls"`
	ItemList   []any  `json:"ItemList"`
	ValDtls    any    `json:"ValDtls"`
}

type GetInvoicesByIdLinesResponse struct {
	InvoiceID     string  `json:"invoice_id"`
	Number        string  `json:"number"`
	PlaceOfSupply *string `json:"place_of_supply"`
	BuyerGSTIN    *string `json:"buyer_gstin"`
	TotalMinor    int64   `json:"total_minor"`
	TaxMinor      int64   `json:"tax_minor"`
	Lines         []any   `json:"lines"`
}

type GetInvoicesByIdRemindersResponse struct {
	Data []any `json:"data"`
}

type GetInvoicesByIdResponse struct {
	ID               string                                   `json:"id"`
	Number           string                                   `json:"number"`
	Currency         string                                   `json:"currency"`
	AmountMinor      int64                                    `json:"amount_minor"`
	TaxMinor         int64                                    `json:"tax_minor"`
	ExternalRef      *string                                  `json:"external_ref"`
	Status           string                                   `json:"status"`
	DueDate          *string                                  `json:"due_date"`
	CustomerID       *string                                  `json:"customer_id"`
	PlaceOfSupply    *string                                  `json:"place_of_supply"`
	BuyerGSTIN       *string                                  `json:"buyer_gstin"`
	ReverseCharge    bool                                     `json:"reverse_charge"`
	IssuedOn         string                                   `json:"issued_on"`
	PaidMinor        int64                                    `json:"paid_minor"`
	CreditedMinor    int64                                    `json:"credited_minor"`
	OutstandingMinor int64                                    `json:"outstanding_minor"`
	Allocations      []GetInvoicesByIdResponseAllocationsItem `json:"allocations"`
}

type GetInvoicesByIdResponseAllocationsItem struct {
	IntentID    string `json:"intent_id"`
	AmountMinor int64  `json:"amount_minor"`
	Reference   string `json:"reference"`
}

type GetInvoicesResponse struct {
	Data []GetInvoicesResponseDataItem `json:"data"`
}

type GetInvoicesResponseDataItem struct {
	ID               string  `json:"id"`
	Number           string  `json:"number"`
	AmountMinor      int64   `json:"amount_minor"`
	PaidMinor        int64   `json:"paid_minor"`
	OutstandingMinor int64   `json:"outstanding_minor"`
	Status           string  `json:"status"`
	DueDate          *string `json:"due_date"`
}

type GetMandatesByIdDebitsResponse struct {
	Data []DebitRow `json:"data"`
}

type GetMandatesByIdResponse struct {
	Mandate any        `json:"mandate"`
	Debits  []DebitRow `json:"debits"`
}

type GetMandatesResponse struct {
	Data []MandateRow `json:"data"`
}

type GetMilestonesResponse struct {
	Data []GetMilestonesResponseDataItem `json:"data"`
}

type GetMilestonesResponseDataItem struct {
	ID          string  `json:"id"`
	CustomerID  string  `json:"customer_id"`
	Name        string  `json:"name"`
	AmountMinor int64   `json:"amount_minor"`
	Status      string  `json:"status"`
	InvoiceID   *string `json:"invoice_id"`
}

type GetOnboardingResponse struct {
	AutomatedChecks any                                     `json:"automated_checks"`
	ReviewReasons   []string                                `json:"review_reasons"`
	Checks          []GetOnboardingResponseChecksItem       `json:"checks"`
	Status          string                                  `json:"status"`
	Profile         []GetOnboardingResponseProfileItem      `json:"profile"`
	Documents       []GetOnboardingResponseDocumentsItem    `json:"documents"`
	BankAccounts    []GetOnboardingResponseBankAccountsItem `json:"bank_accounts"`
}

type GetOnboardingResponseBankAccountsItem struct {
	ID           string  `json:"id"`
	HolderName   any     `json:"holder_name"`
	Ifsc         any     `json:"ifsc"`
	AccountLast4 *string `json:"account_last4"`
	Verified     bool    `json:"verified"`
}

type GetOnboardingResponseChecksItem struct {
	Kind    string `json:"kind"`
	Subject any    `json:"subject"`
	Outcome string `json:"outcome"`
	Detail  any    `json:"detail"`
}

type GetOnboardingResponseDocumentsItem struct {
	ID        string `json:"id"`
	Kind      string `json:"kind"`
	Reference any    `json:"reference"`
	Status    string `json:"status"`
}

type GetOnboardingResponseProfileItem struct {
	LegalName    *string `json:"legal_name"`
	BusinessType any     `json:"business_type"`
	PAN          string  `json:"pan"`
	GSTIN        *string `json:"gstin"`
	Address      any     `json:"address"`
	SubmittedBy  any     `json:"submitted_by"`
	ReviewedBy   any     `json:"reviewed_by"`
	ReviewNote   any     `json:"review_note"`
}

type GetOutboxResponse struct {
	Data []GetOutboxResponseDataItem `json:"data"`
}

type GetOutboxResponseDataItem struct {
	ID        string  `json:"id"`
	Type      string  `json:"type"`
	Status    string  `json:"status"`
	Attempts  int64   `json:"attempts"`
	LastError *string `json:"last_error"`
	Payload   any     `json:"payload"`
	CreatedAt any     `json:"created_at"`
}

type GetPaymentAttemptsByIdResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type GetPaymentIntentsByIdResponse struct {
	Intent   IntentRow    `json:"intent"`
	Attempts []AttemptRow `json:"attempts"`
}

type GetPaymentIntentsByIdTimelineResponse struct {
	PaymentID string `json:"payment_id"`
	Status    string `json:"status"`
	Standing  any    `json:"standing"`
	Events    any    `json:"events"`
}

type GetPaymentIntentsResponse struct {
	Data    []IntentRow `json:"data"`
	HasMore bool        `json:"has_more"`
}

type GetPaymentLinksResponse struct {
	Data []any `json:"data"`
}

type GetPayoutsByIdResponse struct {
	ID          string  `json:"id"`
	VendorID    *string `json:"vendor_id"`
	AmountMinor int64   `json:"amount_minor"`
	Currency    string  `json:"currency"`
	Status      string  `json:"status"`
	Purpose     any     `json:"purpose"`
	ExternalRef *string `json:"external_ref"`
	RequestedBy any     `json:"requested_by"`
	ApprovedBy  any     `json:"approved_by"`
	PaidBy      any     `json:"paid_by"`
	BankRef     any     `json:"bank_ref"`
	CreatedAt   *string `json:"created_at"`
}

type GetPayoutsResponse struct {
	Data any `json:"data"`
}

type GetPlansResponse struct {
	Data []GetPlansResponseDataItem `json:"data"`
}

type GetPlansResponseDataItem struct {
	ID             string `json:"id"`
	Name           string `json:"name"`
	Currency       string `json:"currency"`
	AmountMinor    int64  `json:"amount_minor"`
	IntervalUnit   string `json:"interval_unit"`
	IntervalCount  int64  `json:"interval_count"`
	UsageUnitMinor int64  `json:"usage_unit_minor"`
	Active         bool   `json:"active"`
}

type GetPlatformChangesResponse struct {
	Data []GetPlatformChangesResponseDataItem `json:"data"`
}

type GetPlatformChangesResponseDataItem struct {
	ID                 string  `json:"id"`
	OrganisationID     *string `json:"organisation_id"`
	Organisation       any     `json:"organisation"`
	Kind               string  `json:"kind"`
	Summary            any     `json:"summary"`
	Status             string  `json:"status"`
	RequestedBy        any     `json:"requested_by"`
	RequestedAt        *string `json:"requested_at"`
	Note               *string `json:"note"`
	DueAt              *string `json:"due_at"`
	Overdue            any     `json:"overdue"`
	SlaHours           any     `json:"sla_hours"`
	Stage              string  `json:"stage"`
	FirstApprovedBy    any     `json:"first_approved_by"`
	FirstApprovedByMe  any     `json:"first_approved_by_me"`
	NeedsSecondBecause *string `json:"needs_second_because"`
	EscalationNote     any     `json:"escalation_note"`
	AssignedTo         any     `json:"assigned_to"`
	AssignedToName     any     `json:"assigned_to_name"`
	AssignedToMe       any     `json:"assigned_to_me"`
}

type GetPlatformMeResponse struct {
	ID    string  `json:"id"`
	Name  string  `json:"name"`
	Email *string `json:"email"`
}

type GetPlatformReviewsByOrganisationIdResponse struct {
	Kind    string `json:"kind"`
	Outcome string `json:"outcome"`
	Detail  any    `json:"detail"`
}

type GetPlatformReviewsResponse struct {
	Data any `json:"data"`
}

type GetPlatformRotaCoverageResponse struct {
	Configured bool `json:"configured"`
	Days       any  `json:"days"`
}

type GetPlatformRotaResponse struct {
	Configured       bool                                 `json:"configured"`
	IAmLead          any                                  `json:"i_am_lead"`
	UtcOffsetMinutes any                                  `json:"utc_offset_minutes"`
	Timezone         any                                  `json:"timezone"`
	LocalNow         string                               `json:"local_now"`
	LocalDate        string                               `json:"local_date"`
	OnDutyNow        any                                  `json:"on_duty_now"`
	HolidayToday     any                                  `json:"holiday_today"`
	Reviewers        []any                                `json:"reviewers"`
	DaysOff          []GetPlatformRotaResponseDaysOffItem `json:"days_off"`
}

type GetPlatformRotaResponseDaysOffItem struct {
	ID         string  `json:"id"`
	ReviewerID *string `json:"reviewer_id"`
	Reviewer   *string `json:"reviewer"`
	Desk       any     `json:"desk"`
	Mine       any     `json:"mine"`
	Removable  any     `json:"removable"`
	FirstDay   string  `json:"first_day"`
	LastDay    string  `json:"last_day"`
	Note       string  `json:"note"`
}

type GetProviderAccountsResponse struct {
	Data any `json:"data"`
}

type GetReconCasesByIdSuggestionsResponse struct {
	CaseID    string   `json:"case_id"`
	Kind      string   `json:"kind"`
	Summary   string   `json:"summary"`
	Evidence  []any    `json:"evidence"`
	NextSteps []string `json:"next_steps"`
	Note      string   `json:"note"`
}

type GetReconCasesResponse struct {
	Data []any `json:"data"`
}

type GetRefundsByIdResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type GetRefundsResponse struct {
	Data    []RefundRow `json:"data"`
	HasMore bool        `json:"has_more"`
}

type GetSettingsResponse struct {
	RefundApprovalThresholdMinor int64   `json:"refund_approval_threshold_minor"`
	Routing                      any     `json:"routing"`
	OutboundWebhookURL           *string `json:"outbound_webhook_url"`
	Env                          string  `json:"env"`
	Dunning                      any     `json:"dunning"`
	Gst                          any     `json:"gst"`
	PayoutApprovalThresholdMinor int64   `json:"payout_approval_threshold_minor"`
	Risk                         any     `json:"risk"`
	WebhookEvents                any     `json:"webhook_events"`
}

type GetSettlementsByIdExplainResponse struct {
	Settlement          any   `json:"settlement"`
	Payments            []any `json:"payments"`
	Refunds             []any `json:"refunds"`
	Adjustments         []any `json:"adjustments"`
	Fees                any   `json:"fees"`
	Arithmetic          any   `json:"arithmetic"`
	ExpectedMinor       int64 `json:"expected_minor"`
	ReceivedMinor       int64 `json:"received_minor"`
	DifferenceMinor     int64 `json:"difference_minor"`
	DifferenceBreakdown any   `json:"difference_breakdown"`
	BankMatch           any   `json:"bank_match"`
	Cases               []any `json:"cases"`
	Narrative           bool  `json:"narrative"`
}

type GetSettlementsResponse struct {
	Data []any `json:"data"`
}

type GetSubscriptionsResponse struct {
	Data []GetSubscriptionsResponseDataItem `json:"data"`
}

type GetSubscriptionsResponseDataItem struct {
	ID           string `json:"id"`
	CustomerID   string `json:"customer_id"`
	PlanID       string `json:"plan_id"`
	Quantity     int64  `json:"quantity"`
	Status       string `json:"status"`
	NextBillDate string `json:"next_bill_date"`
}

type GetVendorsByIdBalanceResponse struct {
	VendorID             string `json:"vendor_id"`
	Balances             any    `json:"balances"`
	AwaitingCaptureMinor int64  `json:"awaiting_capture_minor"`
}

type GetVendorsByIdDetailsResponse struct {
	ID      string `json:"id"`
	Details any    `json:"details"`
}

type GetVendorsResponse struct {
	Data []any `json:"data"`
}

type IntentRow struct {
	ID               string   `json:"id"`
	CustomerID       *string  `json:"customer_id"`
	InvoiceID        *string  `json:"invoice_id"`
	BranchID         *string  `json:"branch_id"`
	AmountMinor      int64    `json:"amount_minor"`
	Currency         string   `json:"currency"`
	Status           string   `json:"status"`
	CaptureMethod    string   `json:"capture_method"`
	AllowedProviders []string `json:"allowed_providers"`
	ExternalRef      *string  `json:"external_ref"`
	Description      *string  `json:"description"`
	Metadata         any      `json:"metadata"`
	CreatedAt        string   `json:"created_at"`
	UpdatedAt        string   `json:"updated_at"`
}

type MandateRow struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PatchCustomersByIdResponse struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Email       string  `json:"email"`
	Phone       *string `json:"phone"`
	ExternalRef *string `json:"external_ref"`
	GSTIN       *string `json:"gstin"`
	Address1    *string `json:"address1"`
	Location    *string `json:"location"`
	Pincode     *string `json:"pincode"`
}

type PostApiKeysByIdRevokeResponse struct {
	ID      string `json:"id"`
	Revoked bool   `json:"revoked"`
}

type PostApiKeysByIdRotateResponse struct {
	ID                   string   `json:"id"`
	Key                  string   `json:"key"`
	Prefix               string   `json:"prefix"`
	Scopes               []string `json:"scopes"`
	RotatedFrom          string   `json:"rotated_from"`
	OldKeyExpiresInHours int64    `json:"old_key_expires_in_hours"`
	Note                 string   `json:"note"`
}

type PostApiKeysResponse struct {
	ID           string   `json:"id"`
	Key          string   `json:"key"`
	Prefix       string   `json:"prefix"`
	Scopes       []string `json:"scopes"`
	ExpiresAt    any      `json:"expires_at"`
	AllowedCidrs []string `json:"allowed_cidrs"`
	Note         string   `json:"note"`
}

type PostAuditEventsSealResponse struct {
	SealedEvents any `json:"sealed_events"`
}

type PostBankStatementsImportResponse struct {
	Added                int64 `json:"added"`
	SettlementsRechecked int64 `json:"settlements_rechecked"`
}

type PostBillingRunResponse struct {
	InvoicesCreated int64 `json:"invoices_created"`
}

type PostBranchesResponse struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type PostCashEntriesResponse struct {
	ID string `json:"id"`
}

type PostCheckoutByTenantIdByIntentIdAttemptsResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostCheckoutLinksByTenantIdByTokenStartResponse struct {
	IntentID     string `json:"intent_id"`
	ClientSecret string `json:"client_secret"`
	AmountMinor  int64  `json:"amount_minor"`
	Currency     string `json:"currency"`
	Path         string `json:"path"`
}

type PostClosingsByIdSignoffResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostClosingsResponse struct {
	ID                string `json:"id"`
	ExpectedCashMinor int64  `json:"expected_cash_minor"`
	CountedCashMinor  int64  `json:"counted_cash_minor"`
	DifferenceMinor   int64  `json:"difference_minor"`
	Digital           []any  `json:"digital"`
	Status            string `json:"status"`
}

type PostCollectionsDraftsByIdApproveResponse struct {
	ID         string `json:"id"`
	Status     string `json:"status"`
	ReminderID string `json:"reminder_id"`
}

type PostCollectionsDraftsByIdRejectResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostCollectionsDunningRunResponse struct {
	Reminded      int64    `json:"reminded"`
	DraftsCreated int64    `json:"drafts_created"`
	DraftIds      []string `json:"draft_ids"`
}

type PostCollectionsPromisesByIdCancelResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostConsoleAuthenticateResponse struct {
	User        any `json:"user"`
	MfaEnrolled any `json:"mfa_enrolled"`
}

type PostConsoleInvitationsByIdRevokeResponse struct {
	ID      string `json:"id"`
	Revoked bool   `json:"revoked"`
}

type PostConsoleInvitationsResponse struct {
	ID        string `json:"id"`
	Email     string `json:"email"`
	Role      string `json:"role"`
	ExpiresAt any    `json:"expires_at"`
}

type PostConsoleLogoutResponse struct {
	Ok    bool `json:"ok"`
	Ended any  `json:"ended"`
}

type PostConsoleSwitchEnvResponse struct {
	User       any    `json:"user"`
	SessionKey any    `json:"session_key"`
	TenantID   string `json:"tenant_id"`
	Env        string `json:"env"`
}

type PostConsoleUsersByIdDisableResponse struct {
	ID string `json:"id"`
	Ok bool   `json:"ok"`
}

type PostConsoleUsersByIdEnableResponse struct {
	ID string `json:"id"`
	Ok bool   `json:"ok"`
}

type PostConsoleUsersByIdMfaBeginResponse struct {
	Secret     any    `json:"secret"`
	OtpauthUri string `json:"otpauth_uri"`
}

type PostConsoleUsersByIdMfaConfirmResponse struct {
	RecoveryCodes []string `json:"recovery_codes"`
	Note          string   `json:"note"`
}

type PostConsoleUsersByIdMfaResetResponse struct {
	ID string `json:"id"`
	Ok bool   `json:"ok"`
}

type PostConsoleUsersByIdPasswordResponse struct {
	ID string `json:"id"`
	Ok bool   `json:"ok"`
}

type PostConsoleUsersResponse struct {
	ID       string  `json:"id"`
	Username any     `json:"username"`
	Role     string  `json:"role"`
	BranchID *string `json:"branch_id"`
}

type PostCountersByIdDeactivateResponse struct {
	ID     string `json:"id"`
	Active bool   `json:"active"`
}

type PostCountersResponse struct {
	ID       string `json:"id"`
	BranchID string `json:"branch_id"`
	Name     string `json:"name"`
}

type PostCouponsByIdDeactivateResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostCouponsResponse struct {
	ID   string `json:"id"`
	Code string `json:"code"`
}

type PostCustomersResponse struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Email       *string `json:"email"`
	Phone       *string `json:"phone"`
	ExternalRef *string `json:"external_ref"`
	GSTIN       *string `json:"gstin"`
}

type PostDisputesByIdEvidenceResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostDisputesByIdResolveResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostDisputesResponse struct {
	ID          string `json:"id"`
	Status      string `json:"status"`
	AmountMinor int64  `json:"amount_minor"`
}

type PostFxRatesResponse struct {
	ID       string  `json:"id"`
	Currency string  `json:"currency"`
	Rate     any     `json:"rate"`
	AsOf     *string `json:"as_of"`
}

type PostInstallmentPlansResponse struct {
	ID             string  `json:"id"`
	CustomerID     *string `json:"customer_id"`
	Name           string  `json:"name"`
	TotalMinor     int64   `json:"total_minor"`
	Currency       string  `json:"currency"`
	Instalments    any     `json:"instalments"`
	ExternalRef    *string `json:"external_ref"`
	SettledMinor   int64   `json:"settled_minor"`
	RemainingMinor int64   `json:"remaining_minor"`
	Invoices       []any   `json:"invoices"`
}

type PostInvitationsAcceptResponse struct {
	Email  string `json:"email"`
	Joined bool   `json:"joined"`
}

type PostInvitationsLookupResponse struct {
	Email        string `json:"email"`
	Organisation any    `json:"organisation"`
	Role         string `json:"role"`
}

type PostInvoicesByIdCreditNotesResponse struct {
	ID          string `json:"id"`
	InvoiceID   string `json:"invoice_id"`
	AmountMinor int64  `json:"amount_minor"`
}

type PostInvoicesByIdPromisesResponse struct {
	ID            string `json:"id"`
	InvoiceID     string `json:"invoice_id"`
	PromisedMinor int64  `json:"promised_minor"`
	PromisedDate  string `json:"promised_date"`
	Status        string `json:"status"`
}

type PostInvoicesByIdRemindersDraftResponse struct {
	ID        string  `json:"id"`
	InvoiceID string  `json:"invoice_id"`
	Channel   string  `json:"channel"`
	Message   *string `json:"message"`
	Source    string  `json:"source"`
	Status    string  `json:"status"`
}

type PostInvoicesByIdRemindersResponse struct {
	ID        string `json:"id"`
	InvoiceID string `json:"invoice_id"`
	Channel   string `json:"channel"`
}

type PostInvoicesByIdVoidResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostInvoicesResponse struct {
	ID          string `json:"id"`
	Number      string `json:"number"`
	AmountMinor int64  `json:"amount_minor"`
	TaxMinor    int64  `json:"tax_minor"`
	Status      string `json:"status"`
}

type PostLiveAccessResponse struct {
	Status string `json:"status"`
}

type PostMandatesByIdDebitsResponse struct {
	ID            string  `json:"id"`
	MandateID     string  `json:"mandate_id"`
	AmountMinor   int64   `json:"amount_minor"`
	Currency      string  `json:"currency"`
	DueDate       string  `json:"due_date"`
	NotifiedAt    *string `json:"notified_at"`
	InvoiceID     *string `json:"invoice_id"`
	State         string  `json:"state"`
	AttemptID     *string `json:"attempt_id"`
	RetryNo       int64   `json:"retry_no"`
	FailureReason *string `json:"failure_reason"`
	CreatedAt     string  `json:"created_at"`
}

type PostMandatesByIdPauseResponse struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PostMandatesByIdRefreshResponse struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PostMandatesByIdResumeResponse struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PostMandatesByIdRevokeResponse struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PostMandatesResponse struct {
	ID                 string  `json:"id"`
	CustomerID         string  `json:"customer_id"`
	Currency           string  `json:"currency"`
	MaxAmountMinor     int64   `json:"max_amount_minor"`
	Frequency          string  `json:"frequency"`
	StartDate          string  `json:"start_date"`
	EndDate            string  `json:"end_date"`
	Status             string  `json:"status"`
	Provider           string  `json:"provider"`
	ProviderMandateRef *string `json:"provider_mandate_ref"`
	Authorization      any     `json:"authorization"`
	FailureReason      *string `json:"failure_reason"`
	RevokedBy          *string `json:"revoked_by"`
	ExternalRef        *string `json:"external_ref"`
	CreatedAt          string  `json:"created_at"`
	UpdatedAt          string  `json:"updated_at"`
}

type PostMilestonesByIdCancelResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostMilestonesByIdCompleteResponse struct {
	ID        string  `json:"id"`
	Status    string  `json:"status"`
	InvoiceID *string `json:"invoice_id"`
}

type PostMilestonesResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostOnboardingBankAccountsByIdVerifyResponse struct {
	ID       string `json:"id"`
	Verified bool   `json:"verified"`
}

type PostOnboardingBankAccountsResponse struct {
	ID           string                                        `json:"id"`
	AccountLast4 *string                                       `json:"account_last4"`
	Check        []PostOnboardingBankAccountsResponseCheckItem `json:"check"`
}

type PostOnboardingBankAccountsResponseCheckItem struct {
	Outcome string `json:"outcome"`
	Detail  any    `json:"detail"`
}

type PostOnboardingDocumentsResponse struct {
	ID string `json:"id"`
}

type PostOnboardingProfileChangeResponse struct {
	ID      string `json:"id"`
	Status  string `json:"status"`
	Summary string `json:"summary"`
}

type PostOnboardingReviewResponse struct {
	Status string `json:"status"`
}

type PostOnboardingSubmitResponse struct {
	Status    string `json:"status"`
	Automatic bool   `json:"automatic"`
	Reasons   any    `json:"reasons"`
}

type PostOutboxByIdReplayResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostOutboxTestResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostPasswordResetConfirmResponse struct {
	Ok bool `json:"ok"`
}

type PostPasswordResetRequestResponse struct {
	Ok bool `json:"ok"`
}

type PostPaymentAttemptsByIdCancelResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostPaymentAttemptsByIdCaptureResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostPaymentAttemptsByIdRefreshResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostPaymentAttemptsByIdSimulateEventResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostPaymentIntentsByIdAttemptsResponse struct {
	ID                  string  `json:"id"`
	IntentID            string  `json:"intent_id"`
	Provider            string  `json:"provider"`
	Method              string  `json:"method"`
	Status              string  `json:"status"`
	AmountMinor         int64   `json:"amount_minor"`
	Currency            string  `json:"currency"`
	ProviderOrderRef    *string `json:"provider_order_ref"`
	ProviderPaymentRef  *string `json:"provider_payment_ref"`
	FailureReason       *string `json:"failure_reason"`
	Route               any     `json:"route"`
	ClientAction        any     `json:"client_action"`
	RefundedMinor       int64   `json:"refunded_minor"`
	ReservedRefundMinor int64   `json:"reserved_refund_minor"`
	CreatedAt           string  `json:"created_at"`
	UpdatedAt           string  `json:"updated_at"`
}

type PostPaymentIntentsByIdRefundsResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type PostPaymentIntentsResponse struct {
	ID               string   `json:"id"`
	CustomerID       *string  `json:"customer_id"`
	InvoiceID        *string  `json:"invoice_id"`
	BranchID         *string  `json:"branch_id"`
	AmountMinor      int64    `json:"amount_minor"`
	Currency         string   `json:"currency"`
	Status           string   `json:"status"`
	CaptureMethod    string   `json:"capture_method"`
	AllowedProviders []string `json:"allowed_providers"`
	ExternalRef      *string  `json:"external_ref"`
	Description      *string  `json:"description"`
	Metadata         any      `json:"metadata"`
	CreatedAt        string   `json:"created_at"`
	UpdatedAt        string   `json:"updated_at"`
	ClientSecret     string   `json:"client_secret"`
}

type PostPaymentLinksByIdCancelResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostPaymentLinksResponse struct {
	ID        string  `json:"id"`
	Token     any     `json:"token"`
	Currency  string  `json:"currency"`
	Path      string  `json:"path"`
	MaxUses   *int64  `json:"max_uses"`
	ExpiresAt *string `json:"expires_at"`
}

type PostPayoutsBulkResponse struct {
	Count          int64 `json:"count"`
	AlreadyExisted any   `json:"already_existed"`
	NewTotalMinor  int64 `json:"new_total_minor"`
	Data           any   `json:"data"`
}

type PostPayoutsByIdApproveResponse struct {
	PayoutID    *string `json:"payout_id"`
	AmountMinor int64   `json:"amount_minor"`
}

type PostPayoutsByIdCancelResponse struct {
	ID          string  `json:"id"`
	VendorID    *string `json:"vendor_id"`
	AmountMinor int64   `json:"amount_minor"`
	Currency    string  `json:"currency"`
	Status      string  `json:"status"`
	Purpose     any     `json:"purpose"`
	ExternalRef *string `json:"external_ref"`
	RequestedBy any     `json:"requested_by"`
	ApprovedBy  any     `json:"approved_by"`
	PaidBy      any     `json:"paid_by"`
	BankRef     any     `json:"bank_ref"`
	CreatedAt   *string `json:"created_at"`
}

type PostPayoutsByIdMarkPaidResponse struct {
	ID          string  `json:"id"`
	VendorID    *string `json:"vendor_id"`
	AmountMinor int64   `json:"amount_minor"`
	Currency    string  `json:"currency"`
	Status      string  `json:"status"`
	Purpose     any     `json:"purpose"`
	ExternalRef *string `json:"external_ref"`
	RequestedBy any     `json:"requested_by"`
	ApprovedBy  any     `json:"approved_by"`
	PaidBy      any     `json:"paid_by"`
	BankRef     any     `json:"bank_ref"`
	CreatedAt   *string `json:"created_at"`
}

type PostPayoutsByIdRejectResponse struct {
	PayoutID    *string `json:"payout_id"`
	AmountMinor int64   `json:"amount_minor"`
}

type PostPayoutsResponse struct {
	PayoutID    string `json:"payout_id"`
	AmountMinor int64  `json:"amount_minor"`
	Currency    string `json:"currency"`
}

type PostPlansResponse struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	AmountMinor  int64  `json:"amount_minor"`
	IntervalUnit string `json:"interval_unit"`
}

type PostPlatformChangesByIdAssignResponse struct {
	ID             string  `json:"id"`
	AssignedTo     *string `json:"assigned_to"`
	AssignedToName *string `json:"assigned_to_name"`
}

type PostPlatformChangesByIdDecisionResponse struct {
	ID                 string  `json:"id"`
	Status             string  `json:"status"`
	Stage              string  `json:"stage"`
	NeedsSecondBecause *string `json:"needs_second_because"`
}

type PostPlatformChangesByIdEscalateResponse struct {
	ID        string `json:"id"`
	Status    string `json:"status"`
	Escalated bool   `json:"escalated"`
}

type PostPlatformReviewsByOrganisationIdAssignResponse struct {
	OrganisationID string  `json:"organisation_id"`
	AssignedTo     *string `json:"assigned_to"`
	AssignedToName *string `json:"assigned_to_name"`
}

type PostPlatformReviewsByOrganisationIdDecisionResponse struct {
	OrganisationID     string  `json:"organisation_id"`
	Status             string  `json:"status"`
	Stage              string  `json:"stage"`
	NeedsSecondBecause *string `json:"needs_second_because"`
}

type PostPlatformReviewsByOrganisationIdEscalateResponse struct {
	OrganisationID string `json:"organisation_id"`
	Status         string `json:"status"`
	Escalated      bool   `json:"escalated"`
}

type PostPlatformReviewsByOrganisationIdRestoreResponse struct {
	OrganisationID *string `json:"organisation_id"`
	Status         string  `json:"status"`
}

type PostPlatformReviewsByOrganisationIdRevokeResponse struct {
	OrganisationID *string `json:"organisation_id"`
	Status         string  `json:"status"`
}

type PostPlatformRotaDaysOffResponse struct {
	ID         string  `json:"id"`
	Desk       bool    `json:"desk"`
	ReviewerID *string `json:"reviewer_id"`
	FirstDay   string  `json:"first_day"`
	LastDay    string  `json:"last_day"`
	Note       string  `json:"note"`
}

type PostPlatformRotaRebalanceResponse struct {
	Moved  any `json:"moved"`
	Evened any `json:"evened"`
	Left   any `json:"left"`
}

type PostProviderAccountsByProviderCheckResponse struct {
	Provider string `json:"provider"`
	Ok       bool   `json:"ok"`
	Detail   any    `json:"detail"`
	Env      string `json:"env"`
}

type PostProviderAccountsByProviderEnabledResponse struct {
	Provider string `json:"provider"`
	Enabled  bool   `json:"enabled"`
}

type PostProviderAccountsResponse struct {
	ID         string `json:"id"`
	Provider   string `json:"provider"`
	Updated    any    `json:"updated"`
	WebhookURL string `json:"webhook_url"`
}

type PostReconCasesByIdAssignResponse struct {
	ID       string `json:"id"`
	Status   string `json:"status"`
	Assignee string `json:"assignee"`
}

type PostReconCasesByIdResolveResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostReconScanResponse struct {
	NewCases int64 `json:"new_cases"`
}

type PostRefundsByIdApproveResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type PostRefundsByIdRefreshResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type PostRefundsByIdRejectResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type PostRefundsByIdRetryResponse struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

type PostSettingsResponse struct {
	Ok bool `json:"ok"`
}

type PostSettlementsByIdSimulateBankResponse struct {
	Added                int64 `json:"added"`
	SettlementsRechecked int64 `json:"settlements_rechecked"`
}

type PostSettlementsImportResponse struct {
	Settlement          any   `json:"settlement"`
	Payments            []any `json:"payments"`
	Refunds             []any `json:"refunds"`
	Adjustments         []any `json:"adjustments"`
	Fees                any   `json:"fees"`
	Arithmetic          any   `json:"arithmetic"`
	ExpectedMinor       int64 `json:"expected_minor"`
	ReceivedMinor       int64 `json:"received_minor"`
	DifferenceMinor     int64 `json:"difference_minor"`
	DifferenceBreakdown any   `json:"difference_breakdown"`
	BankMatch           any   `json:"bank_match"`
	Cases               []any `json:"cases"`
	Narrative           bool  `json:"narrative"`
}

type PostSettlementsSyncResponse struct {
	Imported int64 `json:"imported"`
}

type PostSignupResendResponse struct {
	Ok bool `json:"ok"`
}

type PostSignupResponse struct {
	OrganisationID *string `json:"organisation_id"`
	TestTenant     any     `json:"test_tenant"`
	TestKey        any     `json:"test_key"`
	Email          string  `json:"email"`
	Verification   string  `json:"verification"`
}

type PostSignupVerifyResponse struct {
	OrganisationID *string `json:"organisation_id"`
	Verified       bool    `json:"verified"`
}

type PostSimCirclePayResponse struct {
	Status         string `json:"status"`
	RemainingMinor int64  `json:"remaining_minor"`
}

type PostSimCircleResponse struct {
	CircleID *string `json:"circle_id"`
}

type PostSimCircleRevokeResponse struct {
	Status string `json:"status"`
}

type PostSimMandateResponse struct {
	Status string `json:"status"`
}

type PostSimMandateRevokeResponse struct {
	Status string `json:"status"`
}

type PostSimPayResponse struct {
	Status string `json:"status"`
}

type PostSubscriptionsByIdCancelResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostSubscriptionsByIdChangePlanResponse struct {
	ID                 string  `json:"id"`
	PlanID             string  `json:"plan_id"`
	Quantity           int64   `json:"quantity"`
	ProrationMinor     bool    `json:"proration_minor"`
	ProrationInvoiceID *string `json:"proration_invoice_id"`
	RemainingDays      int64   `json:"remaining_days"`
	PeriodDays         int64   `json:"period_days"`
}

type PostSubscriptionsByIdMandateResponse struct {
	ID        string  `json:"id"`
	MandateID *string `json:"mandate_id"`
}

type PostSubscriptionsByIdPauseResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostSubscriptionsByIdResumeResponse struct {
	ID     string `json:"id"`
	Status string `json:"status"`
}

type PostSubscriptionsByIdUsageResponse struct {
	Recorded int64 `json:"recorded"`
	Quantity int64 `json:"quantity"`
}

type PostSubscriptionsResponse struct {
	ID           string  `json:"id"`
	Status       string  `json:"status"`
	NextBillDate *string `json:"next_bill_date"`
}

type PostUsersResponse struct {
	ID    string `json:"id"`
	Email string `json:"email"`
	Role  string `json:"role"`
}

type PostVendorsResponse struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type PostWebhooksByProviderByTenantIdResponse struct {
	Status string `json:"status"`
}

type PutOnboardingResponse struct {
	Status string `json:"status"`
}

type PutPlatformMeResponse struct {
	ID    string  `json:"id"`
	Name  string  `json:"name"`
	Email *string `json:"email"`
}

type PutPlatformRotaMeResponse struct {
	Shifts int64 `json:"shifts"`
	OnDuty any   `json:"on_duty"`
}

type PutPlatformRotaReviewersByIdResponse struct {
	Reviewer string `json:"reviewer"`
	Name     string `json:"name"`
	Shifts   int64  `json:"shifts"`
	OnDuty   any    `json:"on_duty"`
}

type PutProviderAccountsByProviderResponse struct {
	ID       string `json:"id"`
	Provider string `json:"provider"`
	Enabled  bool   `json:"enabled"`
	Replaced any    `json:"replaced"`
}

type RefundRow struct {
	ID                string  `json:"id"`
	AttemptID         string  `json:"attempt_id"`
	IntentID          string  `json:"intent_id"`
	AmountMinor       int64   `json:"amount_minor"`
	Currency          string  `json:"currency"`
	Status            string  `json:"status"`
	Reason            *string `json:"reason"`
	ExternalRef       *string `json:"external_ref"`
	RequestedBy       string  `json:"requested_by"`
	ApprovedBy        *string `json:"approved_by"`
	ProviderRefundRef *string `json:"provider_refund_ref"`
	FailureReason     *string `json:"failure_reason"`
	CreatedAt         string  `json:"created_at"`
	UpdatedAt         string  `json:"updated_at"`
}

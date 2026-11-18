# Cash visibility and refund analytics

Where the money is between "the customer paid" and "it is in your bank", when it should arrive, and whether refunds
are going wrong. Read-only except for the delay alerts, which open reconciliation cases and queue an event. Nothing
here moves money or posts to the ledger.

## Honest status

- **Simulators and hand-built synthetic histories only.** Built and tested against the local provider simulators and
  made-up settlement histories. It has not seen a real provider's settlements, a real bank statement or real traffic.
- **No provider SLA is built in.** A provider's expected settlement time is learned from *your own* past settlements
  or stated by *you*. With neither, the answer is "not enough data" and nothing is forecast or alerted.
- **INR only.** Other currencies are left out of every figure here.
- **Calendar days.** Lag is counted from the payment's IST date to the settlement's date. Weekends and bank holidays
  are not modelled, so a Friday payment looks slower than a Monday one.
- **The range is a description of the past, not a promise.** It says how this business's settlements behaved, not how
  the next one will.
- Settlements must be imported (provider sync or `POST /v1/settlements/import`) for lag to exist; bank credits must be
  imported (bank statements) for "matched" to mean anything.

## Settlement lag

`GET /v1/cash/lag` shows, per provider and payment method, the median and the 10th/90th percentile (nearest rank) of
the days between a payment and the settlement that carried it, over the last 180 days, with min and max. It is shown
only with at least `min_samples` settled payments (default 10, set 3-500 with `POST /v1/cash/settings`); below that
the entry says `not_enough_data` with the count. A settlement dated before its payment is a data error and is left out.

You can also state the lag you expect (`POST /v1/cash/lag_rules`: provider, method or `*`, days 0-60, from your own
contract). It is used in two places: for the forecast when there is too little history (with no range), and as the
alert threshold in place of the observed range.

## Forecast

`GET /v1/cash/forecast?days=7` (1-30) takes captured payments that are in no settlement yet (net of refunds already
made) and places each on a day:

- expected: `ceil(median)` days after the payment (a payment whose expected day has passed but is still inside its
  range is expected today);
- the range: it could arrive from the p10 day, and has nearly always arrived by the p90 day. Each day reports
  cumulative `low` (all payments at their p90), `expected` (at the median) and `high` (at their p10);
- a payment already past its provider's p90 is **overdue**: counted separately, not in the forecast;
- a provider and method with too little history and no stated lag is listed under `not_forecast` with its amount,
  never guessed; one with only a stated lag is forecast on its expected day alone and shown under
  `expected_without_range_minor` (it contributes to no low/high figure).

## Delay alerts

`POST /v1/cash/alerts/scan` (the worker runs it every cycle) opens a reconciliation case, and queues one
`settlement.delayed` event, when:

1. **pending**: payments of one provider, method and day are still unsettled after more days than the threshold
   (case kind `settlement_delayed`, subject `provider:method:date`); or
2. **arrived late**: a settlement in the last 14 days carried a payment that took longer than the threshold, judged
   against the other settlements' history, not itself (case kind `settlement_late`, subject the settlement).

The threshold is your stated lag if you set one (plus `alert_grace_days`), otherwise the provider's own observed p90
(plus grace), otherwise there is no alert. A subject that already has a case (resolved or not) never gets a second
case or event, so scanning repeatedly is safe. Event payloads carry provider, method, dates, counts, amount, threshold
and basis; no customer, payment or staff identifiers. Subscribe with `settlement.delayed`.

## Dashboard

`GET /v1/cash/dashboard` by provider (INR):

| Figure | Meaning |
|---|---|
| `in_transit` | captured, in no imported settlement yet (gross, refunded so far, net) |
| `settled_not_matched_to_bank` | in a settlement with no bank credit confirmed against it (a suggestion needing a person does not count), with the oldest settlement date |
| `matched` | settlements with a confirmed bank credit, in the last `matched_days` (default 30) |
| `refunds_pending` | waiting for approval, processing, unknown or failed, by status |
| `payouts_pending` | waiting for approval or approved; shown for the business, not by provider |

## Refund analytics

`GET /v1/analytics/refunds?from&to&bucket=day|week` (default last 30 days). Aggregates and ids only; no names or
contact details.

- **Refund rate over time**: per bucket, captured, refunds requested and refunds processed (counts and paise) and the
  processed-over-captured rate in basis points. Captured and refunded in one bucket are different payments.
- **Time to refund**: request to processed (includes any wait for approval; measured to the row's last update, so an
  upper estimate), median / p90 / max, overall and per provider, with the same minimum-sample rule.
- **SLA breaches**: refunds still `unknown`, `failed`, `processing` or `pending_approval` after `refund_sla_hours`
  (default 48; `POST /v1/cash/settings`), counts by status and a list of ids with age.
- **Leakage**: (a) refunded payments where your fee rule says the provider returns the fee and the settlement still
  deducted more than the rule expects beyond your tolerance (from the fee check lines; refunds with no fee rule are
  counted as unchecked, not as fine); (b) duplicate refund requests that were stopped: an idempotent replay, or a
  request for more than was left to refund. (b) is counted only from when this feature was installed.

## Scopes

`cash:read` (dashboard, lag, forecast, lag rules, settings, refund analytics), `cash:write` (settings, lag rules,
alert scan). The finance role has both, the read-only console role has `cash:read`, `admin` has everything.

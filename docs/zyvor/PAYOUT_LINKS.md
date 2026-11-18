# Payout links

Status: **implemented and tested against the simulator only.** "Verified" below means the simulator's answer; it verifies
nothing about a real account. Live mode needs a real verification provider (see [RISK_ENGINE.md](RISK_ENGINE.md)).

Code: `crates/zyvor-api/src/payout_links.rs`, using `verify.rs` and `payouts::create_one`. Tests: `tests/link_rules.rs`. Migration `0087`.

A payout link lets a recipient enter their own bank account or UPI id and be paid an amount **you** fixed.

1. **Create** (`POST /v1/payout_links`, scope `payouts:write`): `amount_minor`, `purpose`, optional `expected_name`,
   `external_ref` (repeat-safe; the token is shown once, so a repeat returns the link without it), `expires_in_hours` (default
   72, up to 720). The token is stored only as a hash. The recipient's pages take no amount, so it cannot be changed.
2. **Claim** (public: `GET /v1/checkout/payout_links/{tenant_id}/{token}` for the details, `POST .../claim` with a `name`
   when you gave no `expected_name`, and one of `bank_account { account_number, ifsc }` or `upi { vpa }`). The details are
   verified first through the penny-drop / UPI primitive. Only a valid account whose holder name **matches** the expected name
   (the conservative comparison) goes on; not found, someone else's, a partial match, all say the same thing to the recipient and
   count against the link (five lock it). A provider that does not answer is `try_later` and counts for nothing.
3. **Payout:** a vendor is created (details encrypted, masks in lists) and a payout is requested through the existing payout code
   with **you, the link's creator, as the requester**. So the normal rules hold: a second person approves (you cannot), unless it is
   under your approval threshold; then `mark_paid` or the rail, exactly as for any payout. The link is single use: once claimed it cannot be
   claimed again whatever happens to the payout. Its status follows the payout: `claimed`, `paid`, or `payout_ended` (rejected or
   cancelled: make a new link). Other states: `active`, `expired`, `cancelled`, `locked`.

What the recipient sees: your business name, the amount, the purpose, the expiry and the state of their payout. Never the name the bank
holds. A token from another business or a wrong one is the same `404`. Concurrent claims make one payout (the link row is locked and
the payout reference is `plink:<id>`). `GET /v1/payout_links`, `GET /v1/payout_links/{id}` and `POST /v1/payout_links/{id}/cancel`
(unclaimed links only) are for you.

Limits: the simulator is the only provider with a UPI id check, and this moves no money itself. The check shows that an account
exists in a name, not that the person at the keyboard is who you meant. Without an `expected_name` the name the recipient types is
used, so anyone holding the link can be paid to an account in their own name: give `expected_name` whenever you know it, and treat
the link like cash until it is claimed or expires.

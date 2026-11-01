# Vayora iPhone apps (test mode)

Two SwiftUI apps and a shared package, for testing Vayora Payments end to end on an iPhone. **Nothing here moves real money**, and neither app is a bank-linked UPI app (that needs NPCI TPAP approval and a sponsor bank).

| Target | What it is |
|---|---|
| `VayoraPay` | Payer: scan a QR or paste a `upi://pay` link, confirm with a local test PIN, see history and receipts. The wallet is a test balance kept on the phone |
| `VayoraMerchant` | Merchant: create a payment, show its QR, watch it complete, list payments, refund |
| `Packages/VayoraKit` | API client, `Money` (integer paise), `UPIPayload` parser for hostile QR input, QR rendering, Keychain, shared theme |

## Run it
```sh
cd apps/ios && xcodegen generate && open Vayora.xcodeproj
cd Packages/VayoraKit && swift test      # unit tests
```
Start the API with the simulator payer on: `ZYVOR_SIM_PAYER=1` (also set `ZYVOR_AUTO_MIGRATE=1` for a throwaway database). Register a `sim` provider account, create an API key, and put the server address and key in the Merchant app's Settings; put the server address in Pay's Settings. On a real iPhone use the Mac's LAN address or a public HTTPS test URL.

Flow: Merchant "Collect" → QR → Pay "Scan & pay" → PIN → the Merchant screen turns to "Received".

## Honest limits
- `POST /v1/sim/pay` exists only when `ZYVOR_SIM_PAYER=1` and only touches the in-process simulator; with a live provider it can pay nothing.
- With Cashfree sandbox the QR is Cashfree's simulator, which no scanner app pays; use its simulator page instead.
- Verified so far: `VayoraKit` unit tests and a build of both apps for the iOS Simulator SDK. Not yet run on a simulator or device.


## Previewing the screens without a simulator
`VayoraGallery` renders the screens (fixture data only, no network) to PNG on a Mac:

```sh
cd Packages/VayoraKit && swift run VayoraGallery ../../docs/gallery
```

The PNGs in `docs/gallery` are its output. The screens live in the `VayoraUI` target and both apps use them, so what the gallery shows is what the apps draw. It cannot show the camera, haptics or motion; check those on a phone. The Mac renderer also cannot draw scroll views, system progress spinners or default-style buttons, so the gallery uses flat variants of those.

The server address and API key are remembered in the Keychain, so deleting and reinstalling an app keeps them. The Merchant app's Settings and Pay's first screen can fill them in by scanning a setup QR (`vayora://setup?server=…&key=zk_test_…`); live keys are refused.

Text uses a Dynamic Type aware rounded font (`vfont`), capped at the second accessibility size at the app roots. macOS has no text-size setting, so the gallery cannot show large text, and it draws the fallback frosted card, not the iOS 26 Liquid Glass; check both on a phone.

## Person-to-person features (test mode)
Vayora's simulator only knows merchant payments, so these are kept on the phone and labelled as such:
- **Send** to a saved payee (a UPI ID, or a 10-digit mobile number kept as `<number>@zyvor-test`). The money moves between test wallets on the paying phone; Vayora's server and ledger are not involved.
- **Split a bill** shows a request QR (`vayora://request?pa=…&pn=…&am=…&tn=…`). Another phone's Pay scans it and pays; the amount is required and read with the same rules as a `upi://pay` link.
- **Small payments** (off by default, in Settings): up to ₹200 a payment and ₹1,000 a day need only the slide, not the PIN.
- **Bills** are monthly local notifications. They remind; they never pay.
- **Planned** (Bills, third tab): plan a payment for a date and get a notification; when it is due, "Pay now" opens the usual review, slide and PIN (it is a test transfer to a saved payee, and it is marked done only after a successful payment). "Repeating payments" lists payees you paid three times at an even weekly or monthly gap with similar amounts, found from your history, with a one-tap "Plan next". "Splits" keeps each bill you split and who has paid you back, ticked by hand because this phone is not told when another phone pays.
- **Safety checks** on the review screen: first time paying a UPI ID, a UPI ID one character away from one you saved, and an amount more than 5x the usual for that payee. A first payment of ₹5,000 or more to someone new waits 10 seconds. These warn; they never block, and the payer decides.
- **Pay again** on Home: up to three payees you pay most, weighted toward this weekday and hour; one tap opens the usual review, slide and PIN.
- **Ask your money** in History: type "food last week", "more than 500 this month", "failed payments" or a name. It is plain-word matching on the phone (no model, no network); it shows a total of paid payments above the list.
- **Budgets** (Settings): a monthly limit per category. The review screen warns near or over a limit; it never blocks a payment. Spend is the same paid history the month summary uses (a failed payment never counts).
- **History** has category and Failed chips, search, and **Export** as a CSV file (cells that start with `=`, `+`, `-` or `@` are quoted as text so a spreadsheet never runs them).
- **Receive** (Settings): a QR with your test UPI ID and no amount (`vayora://request?pa=…&pn=…&open=1`). Another Pay scans it and types the amount; a request without `open=1` and without an amount is still refused.
- **Learns categories**: change a payment's category on its receipt and every payment to that UPI ID, earlier and later, uses it (budgets and "Ask your money" follow). The word-guess stays the fallback.
- **Jars** (Settings): set test money aside for a goal with a target. Money moves between the wallet and a jar and is never created or lost; the wallet and a jar never go below zero; deleting a jar returns its money. Resetting the wallet clears the jars. No bank, no interest.
- **Receipts vault**: on any receipt, add a note, tags and a photo of the bill (kept on the phone, scaled down). Tags and notes are found by History search. "Month PDF" in History makes a one-page report of the month's paid payments to share.
- **Split by item** (Bills, Planned): name the people, add items, tick who shared each, and see each person's share. The shares always add up to exactly the total (left-over paise go to the first people). Each person gets their own request QR, and the split is saved to Splits.
- **Siri and Shortcuts** (Pay): "How much did I spend in Vayora Pay" and "My balance in Vayora Pay" answer from this phone's saved history (test money). They appear in the Shortcuts app after the first launch. No widget yet: a widget needs its own target and an App Group, which changes signing on the Mac mini, so it waits for a go-ahead.
- **Merchant cashier mode** (Settings): set an owner PIN and lock the phone for a cashier. It hides AutoPay, Settings and refunds until the PIN is entered from "Owner" on the Collect screen (five wrong tries lock it for 30 seconds). It is a convenience on a shared counter phone, not security: the API key on the phone still opens everything. If no PIN was ever set, unlocking never traps you.
- **Merchant closing reminder** (Settings): a daily notification at a time you choose that asks you to open the app and close the day. Its text is fixed when scheduled, so it cannot carry today's total.
- **Merchant regulars**: name a paid payment (Payment detail) and see a running total per name in Settings, Regulars. Records only: nothing is lent, charged or sent, and refunds are not subtracted.
- **Merchant share QR**: on the QR screen, "Share QR" sends the payment QR as an image to a customer who is not at the counter.
- **Merchant accountant export** (Settings): two CSV files, every payment and a total per day. Days are Indian days (IST); totals are paid payments net of refunds; cells that could run as a formula are quoted as text. Covers the most recent 300 payments.
- **Merchant staff** (Settings, Staff): name the people on the till and pick who is on from the Collect screen; each sale made on this phone is tagged with that name, and Staff shows today's takings by name. Local records, not accounts or logins; refunds are not subtracted.
- **Merchant counter speaker** (Settings, off by default): says "250 rupees received" in English or Hindi when a payment you collected is paid. It uses the phone's own voice and plays over the silent switch.
- **Merchant end of day** (Settings): record cash taken, handed back or put in the till (`POST /v1/cash_entries`), count the till and close the day (`POST /v1/closings`) against the expected cash and digital totals. A second person signs it off on the console. A tenant with no branch can name one from the app (needs the admin scope).
- **Merchant**: a tip (5, 10, 15%) on the keypad that goes into the QR total, "Repeat last amount", takings by the hour on Payments, and a shareable receipt.
- **AutoPay** (test mode, inside the Bills tab). Scan a merchant's AutoPay QR (`upi://mandate?…`), read "up to ₹X, how often, until when", slide and enter the PIN; the simulator plays your bank and turns it on. Pay shows the debits the bank **took** (polled from `GET /v1/sim/mandate/{id}`), takes each from the test wallet exactly once and lists it in History. It cannot show upcoming debits: Zyvor tells the merchant before a debit and does not deliver that notice to the payer. You can stop an AutoPay from Pay; a merchant revoke shows up on the next refresh.
- **Merchant AutoPay tab**: a new AutoPay (customer name, how often, end date, most per debit) shows its QR and waits for approval; the detail screen schedules debits, pauses, resumes, revokes, and has a sandbox-only **Run now** because the in-process simulator only moves when the API runs the pass. The server needs `ZYVOR_SIM_PAYER=1`, and `ZYVOR_MANDATE_NOTICE_HOURS=0` so a debit scheduled for today can go out. Not checked against NPCI rules; real AutoPay needs a bank.

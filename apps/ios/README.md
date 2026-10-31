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

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
- UPI AutoPay mandates need server work and are not built (see `docs/zyvor/IMPLEMENTATION_STATUS.md`).

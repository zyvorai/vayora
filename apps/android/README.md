# Vayora Android apps (test mode)

Kotlin + Jetpack Compose counterparts to the [iPhone apps](../ios/README.md). **Nothing here moves real money**, and neither app is a bank-linked UPI app (that needs NPCI TPAP approval and a sponsor bank).

| Module | What it is |
|---|---|
| `pay` | **Vayora Pay** — scan/paste pay or mandate/circle QR, voice pay, jars, budgets, Circle, biometric lock |
| `merchant` | **Vayora Merchant** — Collect → QR, Payments, Owed, Cashflow/GST, AutoPay, end of day, setup-QR |
| `vayora-kit` | Money/UPI/mandate/circle/voice/budgets/jars + ops client |
| `vayora-ui` | Dark-first tokens and shared Compose screens |

Visual language matches iOS `VayoraUI` (near-black canvas, indigo pay / emerald merchant, Test mode chip).

## Run it

Needs Android Studio Ladybug+ or JDK 17 and the Android SDK.

```sh
cd apps/android
./gradlew :pay:assembleDebug :merchant:assembleDebug :vayora-kit:test
```

Install on an emulator or device:

```sh
./gradlew :pay:installDebug :merchant:installDebug
```

Emulator → host machine API: `http://10.0.2.2:8080` (default in Settings). Physical phone: your Mac’s LAN address.

Start the API with the simulator payer for real QR approval: `ZYVOR_SIM_PAYER=1`. Put a `zk_test_…` key in Merchant Settings and turn **Demo mode** off.

Flow: Merchant Collect → Show QR → Pay Scan (or Paste link) → PIN → Merchant shows Received.

## Honest limits

- Camera QR scan and voice recognition are wired; not exercised on a physical device in CI.
- Demo mode invents a QR and marks it paid after ~3.5s with no server.
- Not checked: TalkBack, large text, haptics end-to-end on device.
- Real UPI Circle rules, NPCI TPAP, sponsor LOI, RBI PA, live sandboxes and PCI vault remain external.

## Design parity

| iOS | Android |
|---|---|
| `VUI` / `Brand` | `Vui` / `Brand` |
| `PayHomeScreen` | `PayHomeScreen` |
| `CollectKeypadScreen` / `QRHeroScreen` | same names |
| `Money` / `UPIPayload` | same rules and Indian grouping |
| Test mode flask chip | `TestChip` |

Extend by adding screens under `vayora-ui` (shared) then wiring navigation in each app, same pattern as iOS `VayoraUI` + app targets.

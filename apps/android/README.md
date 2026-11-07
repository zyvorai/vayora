# Vayora Android apps (test mode)

Kotlin + Jetpack Compose counterparts to the [iPhone apps](../ios/README.md). **Nothing here moves real money**, and neither app is a bank-linked UPI app (that needs NPCI TPAP approval and a sponsor bank).

| Module | What it is |
|---|---|
| `pay` | **Vayora Pay** — scan/paste `upi://pay` or AutoPay mandate QR, test PIN, jars, biometric lock, scam hints |
| `merchant` | **Vayora Merchant** — Collect → QR, Payments, Owed, Cashflow/GST, AutoPay, setup-QR + connection check |
| `vayora-kit` | `Money`, `UPIPayload`, `MandatePayload`, `JarBook`, `LockPolicy`, `VayoraClient`, ops models |
| `vayora-ui` | Dark-first tokens and shared Compose screens (Collect, Pay, Owed, Cashflow, AutoPay, Jars) |

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

- Camera QR scan is wired (CameraX + ZXing); not yet exercised on a physical device in CI.
- Circle, voice notes, budgets and end-of-day are not ported.
- Demo mode invents a QR and marks it paid after ~3.5s with no server.
- Not checked: TalkBack, large text, haptics end-to-end on device.

## Design parity

| iOS | Android |
|---|---|
| `VUI` / `Brand` | `Vui` / `Brand` |
| `PayHomeScreen` | `PayHomeScreen` |
| `CollectKeypadScreen` / `QRHeroScreen` | same names |
| `Money` / `UPIPayload` | same rules and Indian grouping |
| Test mode flask chip | `TestChip` |

Extend by adding screens under `vayora-ui` (shared) then wiring navigation in each app, same pattern as iOS `VayoraUI` + app targets.

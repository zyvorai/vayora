# Vayora Android apps (test mode)

Kotlin + Jetpack Compose counterparts to the [iPhone apps](../ios/README.md). **Nothing here moves real money**, and neither app is a bank-linked UPI app (that needs NPCI TPAP approval and a sponsor bank).

| Module | What it is |
|---|---|
| `pay` | **Vayora Pay** — paste a `upi://pay` link (camera scan next), confirm with a local test PIN, test wallet on the phone |
| `merchant` | **Vayora Merchant** — Collect keypad → QR → watch it complete (demo auto-pays; sandbox polls the intent), Payments list |
| `vayora-kit` | `Money` (integer paise), hostile-safe `UPIPayload`, `VayoraClient`, `TestWallet` |
| `vayora-ui` | Dark-first tokens (`Vui` / `Brand.Pay` indigo, `Brand.Merchant` emerald) and shared Compose screens |

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

Flow: Merchant Collect → Show QR → Pay Paste link (or later Scan) → PIN → Merchant shows Received.

## Honest limits (v0.2 scaffold)

- Camera QR scan is not wired yet (paste `upi://pay` instead); iOS has a full scanner.
- Pay person-to-person extras (jars, Circle, voice, budgets, Face lock) are not ported.
- Merchant Owed / Cashflow / AutoPay / end-of-day are not ported.
- Demo mode invents a QR and marks it paid after ~3.5s with no server.
- Not checked: TalkBack, large text, haptics, a real camera on device.

## Design parity

| iOS | Android |
|---|---|
| `VUI` / `Brand` | `Vui` / `Brand` |
| `PayHomeScreen` | `PayHomeScreen` |
| `CollectKeypadScreen` / `QRHeroScreen` | same names |
| `Money` / `UPIPayload` | same rules and Indian grouping |
| Test mode flask chip | `TestChip` |

Extend by adding screens under `vayora-ui` (shared) then wiring navigation in each app, same pattern as iOS `VayoraUI` + app targets.

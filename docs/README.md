# Documentation

Start with the [README](../README.md) for what Zyvor Payments is, then pick what you need.

## Understand it
| Doc | What it covers |
|---|---|
| [How it works](zyvor/HOW_IT_WORKS.md) | The customer's journey, the server's rules, the worker loop and reconciliation, each with a diagram |
| [Architecture](zyvor/ARCHITECTURE.md) | Components and the invariants the code keeps |
| [Threat model](zyvor/THREAT_MODEL.md) | What the code defends against today, and what it does not |
| [Regulatory position](zyvor/REGULATORY.md) | Why Zyvor does not hold money, and what running it in production needs (not legal advice) |
| [Compliance map](compliance/README.md) | Requirements by area and where each lands in the code |
| [Where Zyvor stands](zyvor/MARKET.md) | A comparison with Indian payment gateways, with its source and caveats |

## What is true today
| Doc | What it covers |
|---|---|
| [Status](zyvor/STATUS.md) | Every area: built and tested, simulator-only, partial or not built |
| [Implementation status](zyvor/IMPLEMENTATION_STATUS.md) | The four completion levels, which capability is at which level, and the gaps |
| [Changelog](../CHANGELOG.md) | What shipped, newest first |

## Connect a provider
| Doc | What it covers |
|---|---|
| [Bring your own provider](zyvor/BRING_YOUR_OWN_PROVIDER.md) | Why a provider is needed and how to swap in your own gateway |
| [Provider HTTP contract](zyvor/PROVIDER_CONTRACT.md) | The JSON API your gateway implements for the `custom` adapter |
| [Cashfree partner setup](zyvor/PARTNER_SETUP.md) | What Cashfree's docs say, what is unconfirmed, and the end-to-end test plan |
| [Real provider sandboxes](zyvor/REAL_SANDBOX.md) | Taking a payment through Razorpay test mode and Cashfree's sandbox |

## Run and try it
| Doc | What it covers |
|---|---|
| [Sandbox, deploy and SDK quick start](../README.md#try-the-sandbox-simulated-providers-no-real-money) | Local sandbox, tests and a first API call |
| [k3s sandbox](../deploy/zyvor/k3s/README.md) | A single-node cluster with HTTPS in front |
| [FluxVM lab](zyvor/FLUXVM_LAB.md) | A second, independent place to run the stack on an Apple silicon Mac |
| [Examples](../examples/README.md) | Runnable scripts: school fees, clinic billing, a shop counter, marketplace splits |
| [iOS test apps](../apps/ios/README.md) | Vayora Pay and Vayora Merchant, and how to preview the screens |
| [OpenAPI](../openapi/zyvor.yaml) | The API spec; a test fails if a route is undocumented |

## Decisions
[ADR 0001: monorepo](adr/0001-monorepo.md) · [ADR 0002: vertical slice (superseded)](adr/0002-vertical-slice.md) · [ADR 0003: extend Rust; UPI QR and intent as the default flow](adr/0003-extend-rust-and-upi-qr-intent.md)

Also: [Security policy](../SECURITY.md) · [Contributing](../CONTRIBUTING.md) · [Code of conduct](../CODE_OF_CONDUCT.md)

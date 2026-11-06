# PCI scope for Zyvor cards

Zyvor **never** stores PAN or CVV. Card acceptance uses network tokens via `zyvor-cards` and/or sponsor hosted fields.

## In scope for Zyvor (SAQ-oriented)

- Storage of `network_token_ref`, last4, brand, expiry
- TLS to clients and to the vault / acquirer
- Access control and logging around `card_tokens`

## Out of scope (must stay outside Zyvor hosts)

- Raw PAN/CVV entry (browser iframe or redirect to PCI vault / sponsor)
- Card data encryption keys for PAN
- Magnetic stripe / chip data

## Controls

- `zyvor_cards::reject_raw_card_fields` refuses API payloads that include `pan` / `cvv` / aliases
- Separate network segment for any future hosted-fields proxy (document in deploy)
- Quarterly review that no logs contain full PAN

See crate `zyvor-cards` and `POST /v1/card_tokens`.

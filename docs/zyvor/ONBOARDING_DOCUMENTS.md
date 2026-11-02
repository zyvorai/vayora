# Onboarding documents: upload, reading and comparison

A business can upload the documents Zyvor's review needs instead of only linking to them. Each file is stored
encrypted, read for the facts on it, and compared with what the business entered. This page says what that does, what
it does not, and what a deployment needs.

## What happens to an upload

1. `POST /v1/onboarding/uploads` (console: Get set up, then Documents). A PDF, PNG or JPEG up to 5 MB, found from its
   first bytes. The business profile must exist first.
2. An Aadhaar card or letter is **refused** and nothing is stored (the word Aadhaar or the authority's name together with
   a 12-digit number in groups of four). Zyvor has no use for the number and may not keep it.
3. The file is read: the PDF's own text layer with `pdftotext` when it has one (certificates from the GST portal do), otherwise
   the first pages are rendered with `pdftoppm` and read with `tesseract` (the Registrar's certificate of incorporation is a
   scan). A JPEG or PNG goes straight to `tesseract`.
4. Facts are found by their fixed shapes: GSTIN (check character validated), PAN, CIN, IFSC, and the legal name from the
   labelled line ("Legal Name", "certify that ... is incorporated", "Name"). Only these are kept: GSTIN, CIN, IFSC, the last four
   characters of the PAN and the legal name.
5. The facts are compared with the profile, each time they are shown (so the result follows a change of profile):

| Document | Compared |
|---|---|
| GST certificate | a valid GSTIN is present; it is the GSTIN entered; its PAN ends as the PAN entered; legal name |
| Certificate of incorporation | a CIN is present; PAN ending; legal name |
| PAN card | PAN ending; name |
| Cancelled cheque | IFSC is that of an account entered |
| Address proof, other | kept for the reviewer, nothing compared |

   Each comparison is `match`, `mismatch`, `not_found` (the document did not show it) or `not_checked` (nothing to compare it with).
   Names are compared ignoring case, punctuation and the usual abbreviations (Pvt Ltd = Private Limited, & = and); a name that is
   close but not the same counts as a mismatch.
6. On submit, any `mismatch` or `not_found`, and any document that could not be read, becomes a **reason for a person to
   look**. Nothing read from a document ever approves a business; an auto-approval by the provider checks still goes to a person if a
   document disagrees.

## Checked on real documents

The company's own GST registration certificate (a text PDF) and certificate of incorporation (a 300 dpi scan with no text
layer) were uploaded through the console. Both were read, and all comparisons matched, including the name entered as "Pvt. Ltd."
against "PRIVATE LIMITED" on the page. With the profile changed to another business, the same documents showed mismatches on PAN and name.

## What it does not do

- It does not decide a document is genuine. A forged certificate with the right numbers reads the same. The GSTIN/PAN/bank provider
  checks (`KYC_CASHFREE.md`, not yet run against Cashfree) and a person are what test the facts against a register.
- OCR of a photo (a cheque, a PAN card on a table) is only as good as the photo. An unreadable document goes to a person.
- A Zyvor reviewer can open each uploaded file (`GET /v1/platform/reviews/{organisation_id}/uploads/{id}/file`, and an Open file button on the review page), but only for a business that has asked for review, and every opening is written to that business's own audit trail as `reviewer:<name>` (`onboarding.upload_viewed_by_reviewer`), so the business can see who looked. Only the reviewer who holds the case can open its files (anyone else gets 403 until they take it), so a reviewer cannot browse the documents of every pending business.
- It reads the first three pages (two for scans), English text.

## What a deployment needs

- `poppler-utils` and `tesseract-ocr` in the API image (the supplied Dockerfile installs them). Without them, documents are
  stored but not read, and every one goes to a person; the API logs a warning at start and the console says so.
- A writable `/tmp` (the supplied manifests mount a 256 Mi `emptyDir`, because the root filesystem is read-only) and about
  768 Mi of memory for the API container.
- `ZYVOR_DOC_READER=off` turns reading off (everything goes to a person).
- Files are encrypted with `ZYVOR_MASTER_KEY`; keep the key rotation notes in `THREAT_MODEL.md` in mind: a stored file is decrypted with
  any key the master set still holds.

## API

`POST /v1/onboarding/uploads` (`{kind, filename, data_base64}`), `GET /v1/onboarding/uploads`,
`GET /v1/onboarding/uploads/{id}/file`, `DELETE /v1/onboarding/uploads/{id}` (only while the profile is a draft or sent back).
Node `onboarding.uploads.*`, Python `*_onboarding_upload(s)` / `upload_onboarding_document`, Go `*OnboardingUpload(s)` /
`UploadOnboardingDocument`. Every upload, download and deletion is audited.

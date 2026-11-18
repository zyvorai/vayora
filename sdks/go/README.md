# Zyvor Go SDK

Amounts are whole paise. Methods return `map[string]any`; `zyvor.Decode[T]` turns a result into a generated type.
Exercised against simulators only.

## Fee checks, bank statements and routing health

```go
c := zyvor.New("zk_test_demo", "http://localhost:8080")

_, err := c.CreateFeeRule("razorpay", "upi", 200, "2026-04-01", map[string]any{"gst_bps": 1800})
_, err = c.UpdateFeeCheckSettings(map[string]any{"tolerance_minor": 100})
_, err = c.RunFeeCheck(map[string]any{"since": "2026-04-01"})
findings, err := zyvor.Decode[zyvor.GetFeeChecksFindingsResponse](c.ListFeeFindings(map[string]any{"status": "open", "limit": 20}))

csv := "Date,Narration,Debit,Credit\n01/04/2026,DEMO CREDIT,,1000.00\n"
_, err = c.PreviewBankStatement(csv, map[string]any{"layout": "generic"})
_, err = c.ImportBankCsv(csv, map[string]any{"layout": "generic"})
_, err = c.ListBankLines(map[string]any{"status": "suggested", "from": "2026-04-01"})
_, err = c.ConfirmBankMatch(9, "checked against the settlement report")
_, err = c.ReconAgeing()

health, err := c.RoutingHealth("upi")
_, err = c.UpdateRoutingHealthSettings(map[string]any{"min_samples": 20})
if e, ok := err.(*zyvor.Error); ok { fmt.Println(e.Status, e.Code) }
```

Lists take `limit`; none of these endpoints take an idempotency key. Tests: `cd sdks/go && go vet ./... && go test ./...`.

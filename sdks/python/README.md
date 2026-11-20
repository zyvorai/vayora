# Zyvor Python SDK

Standard library only. Amounts are whole paise. Exercised against simulators only.

## Fee checks, bank statements and routing health

```python
from zyvor import Zyvor, ZyvorError

z = Zyvor("zk_test_demo", base_url="http://localhost:8080")

z.create_fee_rule("razorpay", "upi", 200, "2026-04-01", gst_bps=1800)  # your contract's rate
z.update_fee_check_settings(tolerance_minor=100)
z.run_fee_check(since="2026-04-01")
findings = z.list_fee_findings(status="open", limit=20)
one = z.settlement_fee_check("set_demo_1")  # read only; recheck_settlement_fee() records it

csv = "Date,Narration,Debit,Credit\n01/04/2026,DEMO CREDIT,,1000.00\n"
if z.preview_bank_statement(csv, layout="generic")["ok"]:
    z.import_bank_csv(csv, layout="generic")
lines = z.list_bank_lines(status="suggested", from_date="2026-04-01", limit=50)
z.confirm_bank_match(9, note="checked against the settlement report")
ageing = z.recon_ageing()

health = z.routing_health(method="upi")
try:
    z.update_routing_health_settings(min_samples=20)  # admin scope
except ZyvorError as e:
    print(e.status, e.code)
```

Lists take `limit`; none of these endpoints take an idempotency key. Tests: `cd sdks/python && python3 -m unittest test_zyvor`.

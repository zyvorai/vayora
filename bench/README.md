# bench/: reproducible benchmark harness (simulator only)

```
bench/run.sh                 # everything: writes bench/results/run-<time>/results.json and results.md
bench/run.sh --quick         # smaller sample sizes (about 3 minutes with the cold build)
bench/run.sh --out DIR --only s1_outage,s3_recon
bench/ci.sh                  # CI entry: two full runs, reproducibility check, hard gates, markdown summary
node bench/check-repro.mjs run1/results.json run2/results.json
```

Method, results, tolerances and limits: [docs/zyvor/BENCHMARKS.md](../docs/zyvor/BENCHMARKS.md).

Layout: `run.sh` builds the API and the simulated provider (`crates/zyvor-provider-mock`), starts them on private ports
with a private Postgres database and role (`lib/stack.sh`, no `pkill`: only the processes it started are stopped and
the database is dropped), runs the scenarios in `scenarios/` (Node, no dependencies), then `devtime.sh` (clean checkout
to first payment, checkout page weight), then `report.mjs`.

Everything is a local simulation: no real provider, no real money, no network beyond localhost.

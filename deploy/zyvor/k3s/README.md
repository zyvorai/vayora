# Zyvor sandbox on k3s

`./deploy.sh user@host` builds the images on the node, imports them into k3s and applies `zyvor.yaml`
(Postgres 17, migrate Job, API, worker, simulated Razorpay/Cashfree, console, checkout) into a `zyvor` namespace.
It needs `CONSOLE_PASSWORD` in your environment; every other secret is generated on the node. This is a
**sandbox**: the providers are simulators.

## HTTPS

The console and checkout Services are cluster-internal (ClusterIP), so nothing speaks plain HTTP to the outside.
Terminate TLS in front of them. On a node that already runs Caddy, add one site per service and reload:

```
console.<name> { reverse_proxy <console ClusterIP>:3003 }
pay.<name>     { reverse_proxy <checkout ClusterIP>:3000 }
```

(`kubectl -n zyvor get svc` shows the ClusterIPs. A `<ip-with-dashes>.sslip.io` name needs no DNS and Caddy
obtains the certificate itself.) Keep the HSTS header: the console's session cookie is `Secure` and is dropped over plain HTTP.

### Letting a real provider call back

A provider (Cashfree, Razorpay) must reach the API to tell it a payment finished, so the webhook path needs a
public address. Expose **only** `/v1/webhooks/*` (each call is checked against the provider's HMAC signature);
never the rest of the API:

```
api.<name> {
	@hooks path /v1/webhooks/*
	reverse_proxy @hooks <zyvor-api ClusterIP>:8080
	respond 404
}
```

The console's Providers page shows each provider's exact webhook address (it uses `API_PUBLIC_URL`, set in
`zyvor.yaml`; change it if your hostname differs). Paste that address and your webhook secret into the provider's dashboard.

Remove everything with `kubectl delete namespace zyvor` (this also deletes the Postgres data).

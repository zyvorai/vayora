#!/usr/bin/env bash
# Development-only self-signed certificate. Use a real CA in production.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p certs
openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost" -keyout certs/tls.key -out certs/tls.crt
echo "wrote certs/tls.crt and certs/tls.key (self-signed, 30 days)"

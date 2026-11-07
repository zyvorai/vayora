#!/usr/bin/env bash
# Build a versioned zip of the Shopify scaffold for manual deploy.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
ver="$(node -p "require('$root/plugins/shopify/package.json').version" 2>/dev/null || echo "0.1.0")"
out="$root/dist/zyvor-shopify-${ver}.zip"
mkdir -p "$root/dist"
rm -f "$out"
(
  cd "$root/plugins"
  zip -r "$out" shopify \
    -x 'shopify/.DS_Store' \
    -x 'shopify/**/.DS_Store' \
    -x 'shopify/node_modules/*' \
    -x 'shopify/node_modules/**'
)
echo "wrote $out"

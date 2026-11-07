#!/usr/bin/env bash
# Build a versioned zip of the WooCommerce plugin for manual install.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
ver="$(grep -E '^ \* Version:' "$root/plugins/woocommerce/zyvor-gateway.php" | awk '{print $3}')"
out="$root/dist/zyvor-woocommerce-${ver}.zip"
mkdir -p "$root/dist"
rm -f "$out"
(
  cd "$root/plugins"
  zip -r "$out" woocommerce \
    -x 'woocommerce/.DS_Store' \
    -x 'woocommerce/**/.DS_Store'
)
echo "wrote $out"

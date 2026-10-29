#!/usr/bin/env bash
# Regenerates social cards and the touch icon from scripts/social/*.html using headless Chrome.
set -euo pipefail
cd "$(dirname "$0")/.."
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
shot() { # html out width height
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size="$3,$4" --screenshot="$PWD/$2" "file://$PWD/$1" >/dev/null 2>&1
}
shot scripts/social/card.html   site/assets/og.png        1200 630
shot scripts/social/square.html site/assets/og-square.png 1200 1200
rsvg-convert -w 180 -h 180 site/assets/favicon.svg -o site/assets/apple-touch-icon.png
echo "wrote site/assets/{og,og-square,apple-touch-icon}.png"

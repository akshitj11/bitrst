#!/usr/bin/env bash
# Render a small terminal demo GIF from deterministic `bitrst mine` output.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BITRST="${BITRST_BIN:-$ROOT/target/debug/bitrst}"
OUT="${1:-$ROOT/docs/assets/mine-demo.gif}"
FRAMES_DIR="${TMPDIR:-/tmp}/bitrst-demo-frames-$$"
FONT="${BITRST_DEMO_FONT:-Adwaita-Mono}"
NETWORK_TIME="${BITRST_DEMO_NETWORK_TIME:-1231007105}"
COUNT="${BITRST_DEMO_COUNT:-2}"

cleanup() {
  rm -rf "$FRAMES_DIR"
}
trap cleanup EXIT

if ! command -v magick >/dev/null 2>&1; then
  echo "error: ImageMagick (magick) is required" >&2
  exit 1
fi

if [[ ! -x "$BITRST" ]]; then
  echo "building bitrst debug binary..." >&2
  cargo build -p bitrst --bin bitrst >&2
fi

mkdir -p "$(dirname "$OUT")"
rm -rf "$FRAMES_DIR"
mkdir -p "$FRAMES_DIR"

mapfile -t lines < <(
  "$BITRST" mine --count "$COUNT" --network-time "$NETWORK_TIME"
)

if [[ ${#lines[@]} -eq 0 ]]; then
  echo "error: mine produced no output" >&2
  exit 1
fi

frame=0
accum=""
for line in "${lines[@]}"; do
  if [[ -z "$accum" ]]; then
    accum="$line"
  else
    accum="$accum
$line"
  fi
  out_png="$FRAMES_DIR/frame-$(printf '%02d' "$frame").png"
  magick -background '#1e1e1e' -fill '#d4d4d4' -font "$FONT" -pointsize 16 \
    label:"$accum" -bordercolor '#1e1e1e' -border 20 "$out_png"
  frame=$((frame + 1))
done

last="$FRAMES_DIR/frame-$(printf '%02d' $((frame - 1))).png"
for hold in 1 2 3; do
  cp "$last" "$FRAMES_DIR/hold-$hold.png"
done

magick -delay 80 -loop 0 "$FRAMES_DIR"/*.png "$OUT"
echo "wrote $OUT"

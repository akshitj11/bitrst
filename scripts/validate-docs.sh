#!/usr/bin/env bash
# Validate documentation assets and machine-readable progress metadata.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() {
  echo "validate-docs: $*" >&2
  exit 1
}

require_file() {
  local path="$1"
  [[ -f "$path" ]] || fail "missing required file: $path"
}

require_file "ARCHITECTURE.md"
require_file "README.md"
require_file "docs/architecture-diagram.mmd"
require_file "docs/progress.json"
require_file "docs/SUMMARY.md"
require_file "book.toml"
require_file "devlog/2026-W34.md"
require_file "docs/assets/mine-demo.gif"

python3 - <<'PY' || fail "docs/progress.json is not valid JSON"
import json
from pathlib import Path

data = json.loads(Path("docs/progress.json").read_text())
required = ("schema_version", "last_updated", "test_command", "milestones", "crates", "diagram")
for key in required:
    if key not in data:
        raise SystemExit(f"missing key: {key}")
if not isinstance(data["schema_version"], int):
    raise SystemExit("schema_version must be an integer")
if not isinstance(data["milestones"], list) or not data["milestones"]:
    raise SystemExit("milestones must be a non-empty list")
if not isinstance(data["crates"], list) or not data["crates"]:
    raise SystemExit("crates must be a non-empty list")
diagram = data["diagram"]
for key in ("source", "sha256"):
    if key not in diagram:
        raise SystemExit(f"diagram missing key: {key}")
PY

expected_sha="$(python3 -c "import json; print(json.load(open('docs/progress.json'))['diagram']['sha256'])")"
actual_sha="$(sha256sum docs/architecture-diagram.mmd | awk '{print $1}')"
[[ "$expected_sha" == "$actual_sha" ]] || fail "architecture-diagram.mmd sha256 mismatch (update docs/progress.json)"

gif_sig="$(head -c 6 docs/assets/mine-demo.gif | tr -d '\0')"
[[ "$gif_sig" == "GIF89a" || "$gif_sig" == "GIF87a" ]] || fail "docs/assets/mine-demo.gif is not a GIF"

if command -v magick >/dev/null 2>&1; then
  dims="$(magick identify -format '%w x %h' docs/assets/mine-demo.gif | head -1)"
  [[ -n "$dims" ]] || fail "could not read GIF dimensions"
fi

echo "validate-docs: ok"

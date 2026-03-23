#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"
TEMPLATE_PATH="${TEMPLATE_PATH:-$ROOT_DIR/packaging/homebrew/catpane.template.rb}"
OUTPUT_PATH="${OUTPUT_PATH:-$DIST_DIR/homebrew/Casks/catpane.rb}"
REPOSITORY="${REPOSITORY:-${GITHUB_REPOSITORY:-}}"
PYTHON_BIN="${PYTHON_BIN:-python3}"

DEFAULT_VERSION="$(
  "$PYTHON_BIN" - "$ROOT_DIR/Cargo.toml" <<'PY'
import pathlib
import sys
import tomllib

data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
package = data["package"]
print(package["version"])
PY
)"

VERSION="${RELEASE_VERSION:-$DEFAULT_VERSION}"
VERSION="${VERSION#v}"

if [[ -z "$REPOSITORY" ]]; then
  echo "REPOSITORY or GITHUB_REPOSITORY must be set (for example owner/catpane)." >&2
  exit 1
fi

find_sha_file() {
  local arch="$1"
  local explicit_path="$2"

  if [[ -n "$explicit_path" ]]; then
    if [[ ! -f "$explicit_path" ]]; then
      echo "Missing checksum file: $explicit_path" >&2
      return 1
    fi
    printf '%s\n' "$explicit_path"
    return 0
  fi

  local pattern="CatPane-v${VERSION}-macos-${arch}.sha256"
  local match
  match=$(find "$DIST_DIR" -type f -name "$pattern" | head -n 1 || true)
  if [[ -z "$match" ]]; then
    echo "Could not find checksum file matching $pattern under $DIST_DIR" >&2
    return 1
  fi

  printf '%s\n' "$match"
}

ARM64_SHA_FILE="$(find_sha_file "arm64" "${ARM64_SHA_FILE:-}")"
INTEL_SHA_FILE="$(find_sha_file "x86_64" "${INTEL_SHA_FILE:-}")"
ARM64_SHA="$(tr -d '[:space:]' < "$ARM64_SHA_FILE")"
INTEL_SHA="$(tr -d '[:space:]' < "$INTEL_SHA_FILE")"

mkdir -p "$(dirname "$OUTPUT_PATH")"

"$PYTHON_BIN" - "$TEMPLATE_PATH" "$OUTPUT_PATH" "$VERSION" "$REPOSITORY" "$ARM64_SHA" "$INTEL_SHA" <<'PY'
import pathlib
import sys

template_path = pathlib.Path(sys.argv[1])
output_path = pathlib.Path(sys.argv[2])
version = sys.argv[3]
repository = sys.argv[4]
arm64_sha = sys.argv[5]
intel_sha = sys.argv[6]

rendered = template_path.read_text()
for placeholder, value in {
    "__VERSION__": version,
    "__REPOSITORY__": repository,
    "__SHA_ARM64__": arm64_sha,
    "__SHA_X86_64__": intel_sha,
}.items():
    rendered = rendered.replace(placeholder, value)

output_path.write_text(rendered)
PY

echo "Created:"
echo "  $OUTPUT_PATH"

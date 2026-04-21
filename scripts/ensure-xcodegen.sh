#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
XCODEGEN_VERSION="${XCODEGEN_VERSION:-2.45.4}"
XCODEGEN_SHA256="${XCODEGEN_SHA256:-090ec29491aad50aec10631bf6e62253fed733c50f3aab0f5ffc86bc170bdbef}"
TOOLS_DIR="${TOOLS_DIR:-$ROOT_DIR/.build-tools/xcodegen/$XCODEGEN_VERSION}"
INSTALL_ROOT="$TOOLS_DIR/xcodegen"
BINARY_PATH="$INSTALL_ROOT/bin/xcodegen"
ARCHIVE_PATH="$TOOLS_DIR/xcodegen.zip"

if command -v xcodegen >/dev/null 2>&1; then
  command -v xcodegen
  exit 0
fi

if [[ ! -x "$BINARY_PATH" ]]; then
  rm -rf "$TOOLS_DIR"
  mkdir -p "$TOOLS_DIR"

  curl \
    --fail \
    --location \
    --silent \
    --show-error \
    "https://github.com/yonaskolb/XcodeGen/releases/download/$XCODEGEN_VERSION/xcodegen.zip" \
    --output "$ARCHIVE_PATH"

  ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
  if [[ "$ACTUAL_SHA256" != "$XCODEGEN_SHA256" ]]; then
    echo "Downloaded xcodegen checksum mismatch: expected $XCODEGEN_SHA256, got $ACTUAL_SHA256" >&2
    exit 1
  fi

  unzip -q "$ARCHIVE_PATH" -d "$TOOLS_DIR"
  rm -f "$ARCHIVE_PATH"
fi

echo "$BINARY_PATH"

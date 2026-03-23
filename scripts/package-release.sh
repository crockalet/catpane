#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PLATFORM="${1:-${RELEASE_PLATFORM:-macos}}"

case "$PLATFORM" in
  macos)
    exec "$ROOT_DIR/scripts/package-macos-app.sh"
    ;;
  *)
    echo "Unsupported release platform: $PLATFORM" >&2
    echo "Supported platforms: macos" >&2
    exit 1
    ;;
esac

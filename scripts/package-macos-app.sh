#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PROFILE="${PROFILE:-release}"
TARGET_TRIPLE="${TARGET_TRIPLE:-}"
APP_NAME="${APP_NAME:-CatPane}"
ARCHIVE_ROOT="${ARCHIVE_ROOT:-$ROOT_DIR/dist}"
ICON_FILE="${ICON_FILE:-$ROOT_DIR/assets/CatPane.icns}"
PYTHON_BIN="${PYTHON_BIN:-python3}"

IFS=$'\t' read -r DEFAULT_BINARY_NAME DEFAULT_VERSION <<EOF
$("$PYTHON_BIN" - "$ROOT_DIR/Cargo.toml" <<'PY'
import pathlib
import sys
import tomllib

data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
package = data["package"]
print(f'{package["name"]}\t{package["version"]}')
PY
)
EOF

BINARY_NAME="${BINARY_NAME:-$DEFAULT_BINARY_NAME}"
VERSION="${RELEASE_VERSION:-$DEFAULT_VERSION}"
VERSION="${VERSION#v}"

if [[ -n "$TARGET_TRIPLE" && "$TARGET_TRIPLE" != *apple-darwin ]]; then
  echo "TARGET_TRIPLE must be a macOS target, got: $TARGET_TRIPLE" >&2
  exit 1
fi

arch_slug() {
  case "$1" in
    "" )
      case "$(uname -m)" in
        arm64|aarch64) echo "arm64" ;;
        x86_64) echo "x86_64" ;;
        *)
          echo "Unsupported host architecture: $(uname -m)" >&2
          return 1
          ;;
      esac
      ;;
    aarch64-apple-darwin|arm64-apple-darwin) echo "arm64" ;;
    x86_64-apple-darwin) echo "x86_64" ;;
    *)
      echo "Unsupported macOS target triple: $1" >&2
      return 1
      ;;
  esac
}

ARCH="$(arch_slug "$TARGET_TRIPLE")"
ARCHIVE_NAME="${APP_NAME}-v${VERSION}-macos-${ARCH}"
OUTPUT_DIR="$ARCHIVE_ROOT/macos/$ARCH"
APP_DIR="$OUTPUT_DIR/${APP_NAME}.app"

if [[ -n "$TARGET_TRIPLE" ]]; then
  cargo build --profile "$PROFILE" --target "$TARGET_TRIPLE"
  BINARY_PATH="$ROOT_DIR/target/$TARGET_TRIPLE/$PROFILE/$BINARY_NAME"
else
  cargo build --profile "$PROFILE"
  BINARY_PATH="$ROOT_DIR/target/$PROFILE/$BINARY_NAME"
fi

if [[ ! -f "$BINARY_PATH" ]]; then
  echo "Built binary not found at $BINARY_PATH" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
rm -rf "$APP_DIR"

CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLIST_PATH="$CONTENTS_DIR/Info.plist"
ZIP_PATH="$OUTPUT_DIR/${ARCHIVE_NAME}.zip"
SHA_PATH="$OUTPUT_DIR/${ARCHIVE_NAME}.sha256"

mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$BINARY_PATH" "$MACOS_DIR/$BINARY_NAME"
chmod +x "$MACOS_DIR/$BINARY_NAME"

ICON_NAME=""
if [[ -f "$ICON_FILE" ]]; then
  ICON_NAME="$(basename "$ICON_FILE")"
  cp "$ICON_FILE" "$RESOURCES_DIR/$ICON_NAME"
fi

BUNDLE_ID="${BUNDLE_ID:-}"
if [[ -z "$BUNDLE_ID" ]]; then
  BUNDLE_ID="$(
    "$PYTHON_BIN" - <<'PY'
import os
import re

repo = os.environ.get("GITHUB_REPOSITORY", "")
parts = [part for part in repo.split("/") if part]
if len(parts) == 2:
    owner, name = parts
    clean = lambda value: re.sub(r"[^A-Za-z0-9.-]", "-", value.lower()).strip(".-") or "catpane"
    print(f"io.github.{clean(owner)}.{clean(name)}")
else:
    print("io.github.catpane")
PY
  )"
fi

"$PYTHON_BIN" - "$PLIST_PATH" "$BUNDLE_ID" "$BINARY_NAME" "$APP_NAME" "$VERSION" "$ICON_NAME" <<'PY'
import pathlib
import plistlib
import sys

plist_path = pathlib.Path(sys.argv[1])
bundle_id = sys.argv[2]
binary_name = sys.argv[3]
app_name = sys.argv[4]
version = sys.argv[5]
icon_name = sys.argv[6]

plist = {
    "CFBundleDevelopmentRegion": "en",
    "CFBundleDisplayName": app_name,
    "CFBundleExecutable": binary_name,
    "CFBundleIdentifier": bundle_id,
    "CFBundleInfoDictionaryVersion": "6.0",
    "CFBundleName": app_name,
    "CFBundlePackageType": "APPL",
    "CFBundleShortVersionString": version,
    "CFBundleVersion": version,
    "LSApplicationCategoryType": "public.app-category.developer-tools",
    "NSHighResolutionCapable": True,
    "NSPrincipalClass": "NSApplication",
}
if icon_name:
    plist["CFBundleIconFile"] = icon_name

plist_path.write_bytes(plistlib.dumps(plist, sort_keys=False))
PY

rm -f "$ZIP_PATH" "$SHA_PATH"
(
  cd "$OUTPUT_DIR"
  ditto -c -k --sequesterRsrc --keepParent "${APP_NAME}.app" "${ARCHIVE_NAME}.zip"
)
shasum -a 256 "$ZIP_PATH" | awk '{print $1}' > "$SHA_PATH"

echo "Created:"
echo "  $ZIP_PATH"
echo "  $SHA_PATH"

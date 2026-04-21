#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CLI_MANIFEST="${CLI_MANIFEST:-$ROOT_DIR/catpane-cli/Cargo.toml}"
PROFILE="${PROFILE:-release}"
TARGET_TRIPLE="${TARGET_TRIPLE:-}"
APP_NAME="${APP_NAME:-CatPane}"
ARCHIVE_ROOT="${ARCHIVE_ROOT:-$ROOT_DIR/dist}"
ICON_FILE="${ICON_FILE:-$ROOT_DIR/assets/CatPane.icns}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:-${APPLE_CODESIGN_IDENTITY:-}}"
CODESIGN_ENTITLEMENTS="${CODESIGN_ENTITLEMENTS:-}"
APPLE_NOTARY_KEY_ID="${APPLE_NOTARY_KEY_ID:-}"
APPLE_NOTARY_ISSUER="${APPLE_NOTARY_ISSUER:-}"
APPLE_NOTARY_KEY_PATH="${APPLE_NOTARY_KEY_PATH:-}"
NATIVE_MACOS_DIR="${NATIVE_MACOS_DIR:-$ROOT_DIR/native/macos}"
NATIVE_PROJECT_SPEC="${NATIVE_PROJECT_SPEC:-$NATIVE_MACOS_DIR/project.yml}"
NATIVE_PROJECT_DIR="${NATIVE_PROJECT_DIR:-$NATIVE_MACOS_DIR}"
NATIVE_PROJECT_PATH="${NATIVE_PROJECT_PATH:-$NATIVE_PROJECT_DIR/CatPaneNative.xcodeproj}"
NATIVE_SCHEME="${NATIVE_SCHEME:-CatPaneNative}"
NATIVE_DERIVED_DATA="${NATIVE_DERIVED_DATA:-$ROOT_DIR/.build-tools/native-derived-data}"
NATIVE_HELPER_NAME="${NATIVE_HELPER_NAME:-CatPaneThrottlingController}"
NATIVE_EXTENSION_NAME="${NATIVE_EXTENSION_NAME:-CatPaneThrottlingExtension.appex}"
NATIVE_HOST_ENTITLEMENTS="${NATIVE_HOST_ENTITLEMENTS:-$NATIVE_MACOS_DIR/Support/CatPaneHostApp.entitlements}"
NATIVE_EXTENSION_ENTITLEMENTS="${NATIVE_EXTENSION_ENTITLEMENTS:-$NATIVE_MACOS_DIR/Support/CatPaneThrottlingExtension.entitlements}"
HOST_PROVISIONING_PROFILE_PATH="${HOST_PROVISIONING_PROFILE_PATH:-${APPLE_HOST_PROVISIONING_PROFILE_PATH:-}}"
EXTENSION_PROVISIONING_PROFILE_PATH="${EXTENSION_PROVISIONING_PROFILE_PATH:-${APPLE_EXTENSION_PROVISIONING_PROFILE_PATH:-}}"

IFS=$'\t' read -r DEFAULT_BINARY_NAME DEFAULT_VERSION <<EOF
$("$PYTHON_BIN" - "$CLI_MANIFEST" <<'PY'
import pathlib
import sys
import tomllib

data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
package = data["package"]
binary_name = package.get("default-run")
if not binary_name:
    bins = data.get("bin", [])
    if bins:
        binary_name = bins[0]["name"]
    else:
        binary_name = package["name"]
print(f'{binary_name}\t{package["version"]}')
PY
)
EOF

BINARY_NAME="${BINARY_NAME:-$DEFAULT_BINARY_NAME}"
RUST_BUNDLE_BINARY_NAME="${RUST_BUNDLE_BINARY_NAME:-${BINARY_NAME}-rust}"
VERSION="${RELEASE_VERSION:-$DEFAULT_VERSION}"
VERSION="${VERSION#v}"
CURRENT_PROJECT_VERSION="${CURRENT_PROJECT_VERSION:-$("$PYTHON_BIN" - "$VERSION" <<'PY'
import re
import sys

parts = [int(part) for part in re.findall(r"\d+", sys.argv[1])[:3]]
while len(parts) < 3:
    parts.append(0)
print(parts[0] * 10000 + parts[1] * 100 + parts[2])
PY
)}"

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
NATIVE_CONFIGURATION="${NATIVE_CONFIGURATION:-$([[ "$PROFILE" == "release" ]] && echo "Release" || echo "Debug")}"

if [[ -n "$TARGET_TRIPLE" ]]; then
  cargo build --profile "$PROFILE" --target "$TARGET_TRIPLE" -p catpane-cli
  BINARY_PATH="$ROOT_DIR/target/$TARGET_TRIPLE/$PROFILE/$BINARY_NAME"
else
  cargo build --profile "$PROFILE" -p catpane-cli
  BINARY_PATH="$ROOT_DIR/target/$PROFILE/$BINARY_NAME"
fi

if [[ ! -f "$BINARY_PATH" ]]; then
  echo "Built binary not found at $BINARY_PATH" >&2
  exit 1
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

mkdir -p "$OUTPUT_DIR"
rm -rf "$APP_DIR"

CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
PLUGINS_DIR="$CONTENTS_DIR/PlugIns"
PLIST_PATH="$CONTENTS_DIR/Info.plist"
ZIP_PATH="$OUTPUT_DIR/${ARCHIVE_NAME}.zip"
SHA_PATH="$OUTPUT_DIR/${ARCHIVE_NAME}.sha256"
NATIVE_CONTROLLER_SOURCE=""
NATIVE_EXTENSION_SOURCE=""
NATIVE_CONTROLLER_DEST=""
NATIVE_EXTENSION_DEST=""
RUST_BUNDLE_DEST=""
USE_NETWORK_LAUNCHER=false

create_release_zip() {
  rm -f "$ZIP_PATH"
  (
    cd "$OUTPUT_DIR"
    ditto -c -k --sequesterRsrc --keepParent "${APP_NAME}.app" "${ARCHIVE_NAME}.zip"
  )
}

sign_path() {
  local path="$1"
  local entitlements="${2:-}"
  local args=()

  if [[ -n "$CODESIGN_IDENTITY" ]]; then
    args=(
      --force
      --options runtime
      --sign "$CODESIGN_IDENTITY"
      --timestamp
    )
  else
    args=(
      --force
      --sign -
    )
  fi

  if [[ -n "$entitlements" ]]; then
    args+=(--entitlements "$entitlements")
  fi

  /usr/bin/codesign "${args[@]}" "$path"
}

build_native_scaffold() {
  if [[ ! -f "$NATIVE_PROJECT_SPEC" ]]; then
    return 0
  fi

  local xcodegen_bin
  xcodegen_bin="$("$ROOT_DIR/scripts/ensure-xcodegen.sh")"

  CATPANE_BUNDLE_ID_BASE="$BUNDLE_ID" \
    "$xcodegen_bin" generate --spec "$NATIVE_PROJECT_SPEC" --project "$NATIVE_PROJECT_DIR" --quiet

  rm -rf "$NATIVE_DERIVED_DATA"
  xcodebuild \
    -project "$NATIVE_PROJECT_PATH" \
    -scheme "$NATIVE_SCHEME" \
    -configuration "$NATIVE_CONFIGURATION" \
    -destination "generic/platform=macOS" \
    -derivedDataPath "$NATIVE_DERIVED_DATA" \
    ARCHS="$ARCH" \
    ONLY_ACTIVE_ARCH=NO \
    MARKETING_VERSION="$VERSION" \
    CURRENT_PROJECT_VERSION="$CURRENT_PROJECT_VERSION" \
    CODE_SIGNING_ALLOWED=NO \
    CODE_SIGNING_REQUIRED=NO \
    -quiet \
    build

  local products_dir="$NATIVE_DERIVED_DATA/Build/Products/$NATIVE_CONFIGURATION"
  NATIVE_CONTROLLER_SOURCE="$products_dir/$NATIVE_HELPER_NAME"
  NATIVE_EXTENSION_SOURCE="$products_dir/$NATIVE_EXTENSION_NAME"

  if [[ ! -x "$NATIVE_CONTROLLER_SOURCE" ]]; then
    echo "Native controller not found at $NATIVE_CONTROLLER_SOURCE" >&2
    exit 1
  fi
  if [[ ! -d "$NATIVE_EXTENSION_SOURCE" ]]; then
    echo "Native extension bundle not found at $NATIVE_EXTENSION_SOURCE" >&2
    exit 1
  fi
}

NOTARIZE_ENABLED=false
if [[ -n "$APPLE_NOTARY_KEY_ID" || -n "$APPLE_NOTARY_ISSUER" || -n "$APPLE_NOTARY_KEY_PATH" ]]; then
  if [[ -z "$APPLE_NOTARY_KEY_ID" || -z "$APPLE_NOTARY_ISSUER" || -z "$APPLE_NOTARY_KEY_PATH" ]]; then
    echo "APPLE_NOTARY_KEY_ID, APPLE_NOTARY_ISSUER, and APPLE_NOTARY_KEY_PATH must all be set together." >&2
    exit 1
  fi
  if [[ -z "$CODESIGN_IDENTITY" ]]; then
    echo "Notarization requires CODESIGN_IDENTITY to be set." >&2
    exit 1
  fi
  if [[ ! -f "$APPLE_NOTARY_KEY_PATH" ]]; then
    echo "Notary API key file not found at $APPLE_NOTARY_KEY_PATH" >&2
    exit 1
  fi
  NOTARIZE_ENABLED=true
fi

if [[ -n "$CODESIGN_ENTITLEMENTS" && ! -f "$CODESIGN_ENTITLEMENTS" ]]; then
  echo "Codesign entitlements file not found at $CODESIGN_ENTITLEMENTS" >&2
  exit 1
fi

if [[ -n "$HOST_PROVISIONING_PROFILE_PATH" && ! -f "$HOST_PROVISIONING_PROFILE_PATH" ]]; then
  echo "Host provisioning profile not found at $HOST_PROVISIONING_PROFILE_PATH" >&2
  exit 1
fi
if [[ -n "$EXTENSION_PROVISIONING_PROFILE_PATH" && ! -f "$EXTENSION_PROVISIONING_PROFILE_PATH" ]]; then
  echo "Extension provisioning profile not found at $EXTENSION_PROVISIONING_PROFILE_PATH" >&2
  exit 1
fi

if [[ -n "$CODESIGN_IDENTITY" && -n "$HOST_PROVISIONING_PROFILE_PATH" && -n "$EXTENSION_PROVISIONING_PROFILE_PATH" ]]; then
  USE_NETWORK_LAUNCHER=true
fi

if [[ "$USE_NETWORK_LAUNCHER" == "true" && -z "$CODESIGN_ENTITLEMENTS" && -f "$NATIVE_HOST_ENTITLEMENTS" ]]; then
  CODESIGN_ENTITLEMENTS="$NATIVE_HOST_ENTITLEMENTS"
fi

build_native_scaffold

mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$PLUGINS_DIR"

if [[ "$USE_NETWORK_LAUNCHER" == "true" && -n "$NATIVE_CONTROLLER_SOURCE" ]]; then
  RUST_BUNDLE_DEST="$MACOS_DIR/$RUST_BUNDLE_BINARY_NAME"
  cp "$BINARY_PATH" "$RUST_BUNDLE_DEST"
  chmod +x "$RUST_BUNDLE_DEST"

  NATIVE_CONTROLLER_DEST="$MACOS_DIR/$BINARY_NAME"
  cp "$NATIVE_CONTROLLER_SOURCE" "$NATIVE_CONTROLLER_DEST"
  chmod +x "$NATIVE_CONTROLLER_DEST"
else
  RUST_BUNDLE_DEST="$MACOS_DIR/$BINARY_NAME"
  cp "$BINARY_PATH" "$RUST_BUNDLE_DEST"
  chmod +x "$RUST_BUNDLE_DEST"

  if [[ -n "$NATIVE_CONTROLLER_SOURCE" ]]; then
    NATIVE_CONTROLLER_DEST="$MACOS_DIR/$NATIVE_HELPER_NAME"
    cp "$NATIVE_CONTROLLER_SOURCE" "$NATIVE_CONTROLLER_DEST"
    chmod +x "$NATIVE_CONTROLLER_DEST"
  fi
fi

if [[ -n "$NATIVE_EXTENSION_SOURCE" ]]; then
  if [[ ! -f "$NATIVE_EXTENSION_ENTITLEMENTS" ]]; then
    echo "Native extension entitlements file not found at $NATIVE_EXTENSION_ENTITLEMENTS" >&2
    exit 1
  fi
  NATIVE_EXTENSION_DEST="$PLUGINS_DIR/$NATIVE_EXTENSION_NAME"
  ditto "$NATIVE_EXTENSION_SOURCE" "$NATIVE_EXTENSION_DEST"
fi

if [[ "$USE_NETWORK_LAUNCHER" == "true" ]]; then
  cp "$HOST_PROVISIONING_PROFILE_PATH" "$CONTENTS_DIR/embedded.provisionprofile"
  if [[ -n "$NATIVE_EXTENSION_DEST" ]]; then
    cp "$EXTENSION_PROVISIONING_PROFILE_PATH" "$NATIVE_EXTENSION_DEST/Contents/embedded.provisionprofile"
  fi
fi

ICON_NAME=""
if [[ -f "$ICON_FILE" ]]; then
  ICON_NAME="$(basename "$ICON_FILE")"
  cp "$ICON_FILE" "$RESOURCES_DIR/$ICON_NAME"
fi

cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
	<key>CFBundleExecutable</key>
	<string>$BINARY_NAME</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>CatPaneRustExecutable</key>
	<string>$(printf '%s' "${RUST_BUNDLE_DEST##*/}")</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.developer-tools</string>
EOF

if [[ -n "$ICON_NAME" ]]; then
  cat >> "$PLIST_PATH" <<EOF
	<key>CFBundleIconFile</key>
	<string>$ICON_NAME</string>
EOF
fi

cat >> "$PLIST_PATH" <<'EOF'
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
</dict>
</plist>
EOF

if [[ -n "$NATIVE_CONTROLLER_DEST" ]]; then
  if [[ "$USE_NETWORK_LAUNCHER" == "true" ]]; then
    sign_path "$NATIVE_CONTROLLER_DEST" "${CODESIGN_ENTITLEMENTS:-}"
  else
    sign_path "$NATIVE_CONTROLLER_DEST"
  fi
fi
if [[ -n "$NATIVE_EXTENSION_DEST" ]]; then
  sign_path "$NATIVE_EXTENSION_DEST" "$NATIVE_EXTENSION_ENTITLEMENTS"
fi
if [[ -n "$RUST_BUNDLE_DEST" ]]; then
  sign_path "$RUST_BUNDLE_DEST"
fi
sign_path "$APP_DIR" "${CODESIGN_ENTITLEMENTS:-}"

/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_DIR"

rm -f "$SHA_PATH"
create_release_zip

if [[ "$NOTARIZE_ENABLED" == "true" ]]; then
  /usr/bin/xcrun notarytool submit \
    "$ZIP_PATH" \
    --key "$APPLE_NOTARY_KEY_PATH" \
    --key-id "$APPLE_NOTARY_KEY_ID" \
    --issuer "$APPLE_NOTARY_ISSUER" \
    --wait
  /usr/bin/xcrun stapler staple "$APP_DIR"
  /usr/bin/xcrun stapler validate "$APP_DIR"
  create_release_zip
fi

shasum -a 256 "$ZIP_PATH" | awk '{print $1}' > "$SHA_PATH"

echo "Created:"
echo "  $ZIP_PATH"
echo "  $SHA_PATH"

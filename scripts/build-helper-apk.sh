#!/usr/bin/env bash
# Builds the CatPane Android helper APK and copies it to a deterministic
# location (`target/helper-apk/catpane-helper.apk`) for downstream packaging
# scripts (macOS bundle, GitHub release uploads).
#
# Requires:
#   * JDK 17+ on PATH or via JAVA_HOME
#   * Android SDK with build-tools 34 installed
#     (set ANDROID_HOME / ANDROID_SDK_ROOT, or ~/Library/Android/Sdk)
#
# CI: invoked from .github/workflows/release.yml before the macOS packaging
# step so the bundled APK is available to embed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="${REPO_ROOT}/android/catpane-helper-app"
OUT_DIR="${REPO_ROOT}/target/helper-apk"
OUT_APK="${OUT_DIR}/catpane-helper.apk"

if [[ ! -d "${APP_DIR}" ]]; then
  echo "error: helper app sources missing at ${APP_DIR}" >&2
  exit 1
fi

# Locate Android SDK if not already exported.
if [[ -z "${ANDROID_HOME:-}" && -z "${ANDROID_SDK_ROOT:-}" ]]; then
  for guess in "${HOME}/Library/Android/Sdk" "${HOME}/Android/Sdk" "/opt/android-sdk"; do
    if [[ -d "${guess}" ]]; then
      export ANDROID_HOME="${guess}"
      break
    fi
  done
fi
if [[ -z "${ANDROID_HOME:-}" ]]; then
  echo "error: ANDROID_HOME not set and no SDK found" >&2
  exit 1
fi

# local.properties is gitignored; regenerate so AGP can find the SDK.
echo "sdk.dir=${ANDROID_HOME}" > "${APP_DIR}/local.properties"

mkdir -p "${OUT_DIR}"

cd "${APP_DIR}"
./gradlew --no-daemon :app:assembleDebug

SRC_APK="${APP_DIR}/app/build/outputs/apk/debug/app-debug.apk"
if [[ ! -f "${SRC_APK}" ]]; then
  echo "error: APK not produced at ${SRC_APK}" >&2
  exit 1
fi

cp -f "${SRC_APK}" "${OUT_APK}"
echo "wrote ${OUT_APK}"

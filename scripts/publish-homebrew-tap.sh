#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CASK_NAME="${CASK_NAME:-catpane}"
CASK_SOURCE="${1:-${CASK_SOURCE:-$ROOT_DIR/dist/homebrew/Casks/${CASK_NAME}.rb}}"
TAP_REPOSITORY="${HOMEBREW_TAP_REPOSITORY:-}"
TAP_TOKEN="${HOMEBREW_TAP_TOKEN:-}"
TAP_BRANCH="${HOMEBREW_TAP_BRANCH:-}"
VERSION="${RELEASE_VERSION:-}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
CASK_DESTINATION="Casks/${CASK_NAME}.rb"
BETA_CASK_DESTINATION="Casks/catpane@beta.rb"

if [[ -z "$TAP_REPOSITORY" || -z "$TAP_TOKEN" ]]; then
  echo "HOMEBREW_TAP_REPOSITORY and HOMEBREW_TAP_TOKEN must be set." >&2
  exit 1
fi

if [[ ! -f "$CASK_SOURCE" ]]; then
  echo "Cask file not found at $CASK_SOURCE" >&2
  exit 1
fi

extract_cask_version() {
  local cask_path="$1"
  "$PYTHON_BIN" - "$cask_path" <<'PY'
import pathlib
import re
import sys

content = pathlib.Path(sys.argv[1]).read_text()
match = re.search(r'^\s*version\s+"([^"]+)"', content, re.MULTILINE)
if not match:
    raise SystemExit(f"Could not find version in {sys.argv[1]}")
print(match.group(1))
PY
}

version_gt() {
  "$PYTHON_BIN" - "$1" "$2" <<'PY'
import re
import sys

SEMVER_RE = re.compile(r'^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$')

def parse(version: str):
    match = SEMVER_RE.fullmatch(version)
    if not match:
        raise SystemExit(f"Unsupported semantic version: {version}")
    core = tuple(int(match.group(i)) for i in range(1, 4))
    prerelease = match.group(4)
    if prerelease is None:
        return core, None
    parts = []
    for part in prerelease.split('.'):
        if part.isdigit():
            parts.append((0, int(part)))
        else:
            parts.append((1, part))
    return core, tuple(parts)

def compare(left: str, right: str) -> int:
    left_core, left_pre = parse(left)
    right_core, right_pre = parse(right)
    if left_core != right_core:
        return 1 if left_core > right_core else -1
    if left_pre is None and right_pre is None:
        return 0
    if left_pre is None:
        return 1
    if right_pre is None:
        return -1
    for left_part, right_part in zip(left_pre, right_pre):
        if left_part == right_part:
            continue
        left_kind, left_value = left_part
        right_kind, right_value = right_part
        if left_kind != right_kind:
            return -1 if left_kind == 0 else 1
        return 1 if left_value > right_value else -1
    if len(left_pre) == len(right_pre):
        return 0
    return 1 if len(left_pre) > len(right_pre) else -1

sys.exit(0 if compare(sys.argv[1], sys.argv[2]) > 0 else 1)
PY
}

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

CLONE_URL="https://x-access-token:${TAP_TOKEN}@github.com/${TAP_REPOSITORY}.git"
CLONE_ARGS=()
if [[ -n "$TAP_BRANCH" ]]; then
  CLONE_ARGS+=(--branch "$TAP_BRANCH")
fi

git clone --quiet "${CLONE_ARGS[@]}" "$CLONE_URL" "$TMP_DIR"
mkdir -p "$TMP_DIR/Casks"
cp "$CASK_SOURCE" "$TMP_DIR/$CASK_DESTINATION"

pushd "$TMP_DIR" >/dev/null

if [[ -z "$TAP_BRANCH" ]]; then
  TAP_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || echo main)"
fi

git checkout -B "$TAP_BRANCH" >/dev/null

removed_beta_cask=false
if [[ "$CASK_NAME" == "catpane" && -n "$VERSION" && -f "$BETA_CASK_DESTINATION" ]]; then
  beta_version="$(extract_cask_version "$BETA_CASK_DESTINATION")"
  if version_gt "$VERSION" "$beta_version"; then
    rm -f "$BETA_CASK_DESTINATION"
    removed_beta_cask=true
  fi
fi

if [[ -z "$(git status --porcelain -- "$CASK_DESTINATION" "$BETA_CASK_DESTINATION")" ]]; then
  echo "Homebrew tap already up to date."
  exit 0
fi

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add "$CASK_DESTINATION"
if [[ "$removed_beta_cask" == "true" ]]; then
  git add -u "$BETA_CASK_DESTINATION"
fi

commit_message="Update ${CASK_NAME} cask${VERSION:+ to v$VERSION}"
if [[ "$removed_beta_cask" == "true" ]]; then
  commit_message="${commit_message} and remove stale catpane@beta cask"
fi
git commit -m "$commit_message"
git push --quiet origin "HEAD:refs/heads/$TAP_BRANCH"

popd >/dev/null

echo "Published ${CASK_NAME} cask to $TAP_REPOSITORY"

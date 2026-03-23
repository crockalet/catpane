#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
CASK_SOURCE="${1:-${CASK_SOURCE:-$ROOT_DIR/dist/homebrew/Casks/catpane.rb}}"
TAP_REPOSITORY="${HOMEBREW_TAP_REPOSITORY:-}"
TAP_TOKEN="${HOMEBREW_TAP_TOKEN:-}"
TAP_BRANCH="${HOMEBREW_TAP_BRANCH:-}"
VERSION="${RELEASE_VERSION:-}"

if [[ -z "$TAP_REPOSITORY" || -z "$TAP_TOKEN" ]]; then
  echo "HOMEBREW_TAP_REPOSITORY and HOMEBREW_TAP_TOKEN must be set." >&2
  exit 1
fi

if [[ ! -f "$CASK_SOURCE" ]]; then
  echo "Cask file not found at $CASK_SOURCE" >&2
  exit 1
fi

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
cp "$CASK_SOURCE" "$TMP_DIR/Casks/catpane.rb"

pushd "$TMP_DIR" >/dev/null

if git diff --quiet -- Casks/catpane.rb; then
  echo "Homebrew tap already up to date."
  exit 0
fi

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add Casks/catpane.rb
git commit -m "Update catpane cask${VERSION:+ to v$VERSION}"
git push --quiet origin HEAD

popd >/dev/null

echo "Published catpane cask to $TAP_REPOSITORY"

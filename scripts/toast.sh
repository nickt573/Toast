#!/bin/bash
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

if [ "${1:-}" = "version" ]; then
  grep -o '"version": "[0-9.]*"' "$REPO/src-tauri/tauri.conf.json" | grep -o '[0-9.]*'
  exit 0
fi

if [ "${1:-}" != "update" ] || [ -z "${2:-}" ]; then
  echo "Usage: toast update vX.X.X | toast version"
  exit 1
fi

TAG="$2"
if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Version must look like v1.2.3, got: $TAG"
  exit 1
fi
VERSION="${TAG#v}"

cd "$REPO"

sed -i '' "s/\"version\": \"[0-9.]*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json
sed -i '' "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" src-tauri/Cargo.toml
sed -i '' "/^name = \"toast\"\$/{n;s/^version = \".*\"/version = \"$VERSION\"/;}" src-tauri/Cargo.lock

git add src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "release $TAG"
git push --force origin main
git tag "$TAG"
git push origin "$TAG"

echo "Successfully updated Toast to $TAG"

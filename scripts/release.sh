#!/bin/bash
# Releases the version in Cargo.toml from a Mac. This computer checks everything, builds and
# tests macOS (with the disk image) and, when Docker is running (Colima), both Linux
# architectures, then uploads them to a draft release. GitHub Actions builds only what is left
# (Windows), signs every download and publishes. On Windows, scripts/release.ps1 does the same
# with Windows built locally.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target=aarch64-apple-darwin
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo "Run this on an Apple Silicon Mac." >&2; exit 1; }
for tool in gh cargo python3 hdiutil; do command -v "$tool" >/dev/null || { echo "Install $tool first." >&2; exit 1; }; done
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
tag="v$version"
[[ "$(git branch --show-current)" == main ]] || { echo "Release from main." >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo "Commit or stash local changes first." >&2; exit 1; }
git fetch -q origin main --tags
if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
  echo "$tag already exists. Raise the version in Cargo.toml and desktop-ui/Cargo.toml first." >&2; exit 1
fi

echo "Checking $tag on this Mac…"
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --target "$target"
(cd desktop-ui && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked)
(cd hosted && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked)
cargo build --locked --release --target "$target"
cargo build --locked --release --manifest-path desktop-ui/Cargo.toml --target-dir target --target "$target"
bin="target/$target/release"
python3 tests/smoke.py "$bin/psms"
python3 tests/mcp.py "$bin/psms"
packaging/macos/build-dmg.sh "$bin" "$version" >/dev/null

# The release job unpacks each bundle into binaries/papamo-sms-<target>, like a CI artifact.
bundles="target/release-bundles"
rm -rf "$bundles" && mkdir -p "$bundles/papamo-sms-$target"
cp "$bin/psms" "$bin/psms-desktop" "$bin/psms-macos-arm64.dmg" "$bundles/papamo-sms-$target/"
built="$target"
if docker info >/dev/null 2>&1; then
  for arch in arm64 x64; do
    scripts/build-linux.sh "$arch"
  done
  built="$built,aarch64-unknown-linux-gnu,x86_64-unknown-linux-gnu"
else
  echo "Docker is not running, so Actions builds Linux too (start it with: colima start)."
fi
uploads=()
for dir in "$bundles"/papamo-sms-*; do
  name="$(basename "$dir")"
  tar -czf "$bundles/local-$name.tar.gz" -C "$bundles" "$name"
  uploads+=("$bundles/local-$name.tar.gz")
done

echo "Publishing $tag: built here: $built; the rest on Actions…"
git push -q origin main
git tag "$tag"
git push -q origin "$tag"
gh release create "$tag" --draft --verify-tag --generate-notes --title "pSMS $tag"
gh release upload "$tag" "${uploads[@]}"
gh workflow run build.yml --ref main -f tag="$tag" -f skip="$built"
sleep 5
gh run list --workflow build.yml --limit 1
echo "Follow it with: gh run watch \$(gh run list --workflow build.yml --limit 1 --json databaseId --jq '.[0].databaseId')"

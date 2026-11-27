#!/bin/bash
# Build the x86_64 Linux server binaries on this machine and roll them out over SSH,
# without GitHub Actions. Needs the na/sg/ws/sa SSH aliases, zig and cargo-zigbuild.
set -euo pipefail
usage() { echo "Usage: $0 [--regions na,sg,ws,sa] [--skip-checks] [--publish]"; }
root="$(cd "$(dirname "$0")/.." && pwd)"
target=x86_64-unknown-linux-gnu
checks=1
rollout=()
while (($#)); do
  case "$1" in
    --skip-checks) checks=0 ;;
    --publish) rollout+=(--publish) ;;
    --regions) rollout+=(--regions "${2:?}"); shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done
missing=()
command -v zig >/dev/null || missing+=("zig (https://ziglang.org/download/)")
cargo zigbuild --help >/dev/null 2>&1 || missing+=("cargo-zigbuild (cargo install --locked cargo-zigbuild)")
rustup target list --installed | grep -qx "$target" || missing+=("Rust target (rustup target add $target)")
if ((${#missing[@]})); then
  printf 'Install first:\n'; printf '  %s\n' "${missing[@]}"; exit 1
fi
cd "$root"
if [[ -n "$(git status --porcelain)" ]]; then
  echo "Commit or stash local changes before deploying." >&2; exit 1
fi
if ((checks)); then
  cargo fmt --check
  cargo clippy --locked --all-targets -- -D warnings
  cargo test --locked
fi
cargo zigbuild --locked --release --target "$target" --bin psms
cargo zigbuild --locked --release --target "$target" --manifest-path hosted/Cargo.toml --target-dir target
out="target/$target/release"
echo "Deploying $(git rev-parse --short HEAD) to the servers."
python3 scripts/update-hosted.py "$out/psms-hosted" --psms "$out/psms" "${rollout[@]+"${rollout[@]}"}"

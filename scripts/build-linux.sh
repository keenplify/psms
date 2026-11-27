#!/bin/bash
# Builds and tests the Linux release for one architecture in a container, as the CI Linux jobs
# do, and stages it as target/release-bundles/papamo-sms-<target>/. Needs Docker (on a Mac,
# Colima with Rosetta: colima start --vm-type vz --vz-rosetta, and brew install docker-buildx).
# Usage: scripts/build-linux.sh arm64|x64
set -euo pipefail
arch="${1:?arm64 or x64}"
case "$arch" in
  arm64) platform=linux/arm64 target=aarch64-unknown-linux-gnu label=linux-arm64 ;;
  x64) platform=linux/amd64 target=x86_64-unknown-linux-gnu label=linux-x64 ;;
  *) echo "Use arm64 or x64." >&2; exit 2 ;;
esac
root="$(cd "$(dirname "$0")/.." && pwd)"
image="psms-linux-build:$arch"
docker buildx build --load -q --platform "$platform" -t "$image" -f "$root/packaging/linux/build.Dockerfile" "$root/packaging/linux" >/dev/null
docker run --rm --platform "$platform" \
  -v "$root:/work" -w /work \
  -v "psms-cargo-registry-$arch:/usr/local/cargo/registry" \
  -e CARGO_TARGET_DIR="/work/target/linux-$arch" \
  -e TARGET="$target" -e LABEL="$label" \
  "$image" bash -euo pipefail -c '
    bin="$CARGO_TARGET_DIR/$TARGET/release"
    cargo test --locked --target "$TARGET"
    cargo build --locked --release --target "$TARGET"
    cargo build --locked --release --manifest-path desktop-ui/Cargo.toml --target "$TARGET"
    python3 tests/smoke.py "$bin/psms"
    python3 tests/mcp.py "$bin/psms"
    desktop-file-validate packaging/linux/dev.papamo.psms.desktop packaging/linux/autostart.desktop
    pkg="$CARGO_TARGET_DIR/package"
    cmake -S packaging/linux -B "$pkg" -DCMAKE_INSTALL_PREFIX=/usr -DBIN_DIR="$bin" -DLABEL="$LABEL" >/dev/null
    cpack --config "$pkg/CPackConfig.cmake" -G DEB -B "$bin" >/dev/null
    cpack --config "$pkg/CPackConfig.cmake" -G RPM -B "$bin" >/dev/null
    dpkg-deb -c "$bin/psms-$LABEL.deb" | grep -F usr/share/applications/dev.papamo.psms.desktop >/dev/null
    rpm -qpl "$bin/psms-$LABEL.rpm" | grep -Fx /usr/share/applications/dev.papamo.psms.desktop >/dev/null
    out=/work/target/release-bundles/papamo-sms-$TARGET
    rm -rf "$out" && mkdir -p "$out"
    cp "$bin/psms" "$bin/psms-desktop" "$bin/psms-$LABEL.deb" "$bin/psms-$LABEL.rpm" "$out/"
  '
echo "Staged target/release-bundles/papamo-sms-$target"

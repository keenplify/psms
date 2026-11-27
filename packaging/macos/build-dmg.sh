#!/bin/bash
# Builds psms-macos-arm64.dmg: pSMS.app (the desktop app with the psms CLI inside) and a link
# to Applications. Usage: packaging/macos/build-dmg.sh <release bin dir> <version>
set -euo pipefail
bin="$(cd "${1:?release bin dir}" && pwd)"
version="${2:?version}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/stage/pSMS.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/psms" "$bin/psms-desktop" "$app/Contents/MacOS/"
cat > "$app/Contents/MacOS/launch" <<'EOF'
#!/bin/sh
# Opens the pSMS window, starting the archive service in the background when needed. The
# window then runs in this same process, so the Dock shows one pSMS. Opened at login, pSMS
# installs a waiting update straight away.
if [ "$1" = "--at-login" ]; then export PSMS_AT_LOGIN=1; fi
exec "$(dirname "$0")/psms" desktop
EOF
chmod 755 "$app/Contents/MacOS/"*
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>pSMS</string>
<key>CFBundleDisplayName</key><string>pSMS</string>
<key>CFBundleIdentifier</key><string>dev.papamo.psms</string>
<key>CFBundleExecutable</key><string>launch</string>
<key>CFBundleIconFile</key><string>pSMS</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSAppleEventsUsageDescription</key><string>pSMS sends the replies you write in your inbox through the Messages app.</string>
</dict></plist>
EOF
icons="$work/pSMS.iconset"
mkdir "$icons"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$root/assets/psms-mark.png" --out "$icons/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$root/assets/psms-mark.png" --out "$icons/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$icons" -o "$app/Contents/Resources/pSMS.icns"
codesign --force --deep --sign - "$app"
ln -s /Applications "$work/stage/Applications"
rm -f "$bin/psms-macos-arm64.dmg"
hdiutil create -quiet -volname "pSMS" -srcfolder "$work/stage" -format UDZO "$bin/psms-macos-arm64.dmg"
hdiutil verify -quiet "$bin/psms-macos-arm64.dmg"
echo "$bin/psms-macos-arm64.dmg"

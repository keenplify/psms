#!/bin/bash
set -euo pipefail
usage() { echo "Usage: $0 [mac | iphone | ipad | sign-only] [simulator-UDID]"; }
mode=${1:-mac}
case "$mode" in mac|iphone|ipad|sign-only) ;; --help|-h) usage; exit 0 ;; *) usage; exit 2 ;; esac
[[ "$(uname -s)" == Darwin ]] || { echo "Run this launcher on macOS." >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
out="${PSMS_CAPTURE_DIR:-$HOME/Downloads/pSMS-capture}"
mkdir -p "$out"
stage="$(mktemp -d)"
trap 'rm -f "$stage/pSMS-Capture.shortcut"; rmdir "$stage"' EXIT
/usr/bin/shortcuts sign --mode anyone --input "$root/examples/pSMS-Capture.shortcut" --output "$stage/pSMS-Capture.shortcut"
cp "$stage/pSMS-Capture.shortcut" "$out/pSMS-Capture.shortcut"
echo "Signed public template: $out/pSMS-Capture.shortcut"
[[ "$mode" != sign-only ]] || exit 0
/usr/bin/open -a Shortcuts "$out/pSMS-Capture.shortcut"
[[ "$mode" != mac ]] || exit 0
device="${2:-}"
if [[ -z "$device" ]]; then
  device="$(xcrun simctl list devices available -j | /usr/bin/python3 -c 'import json,sys
kind=sys.argv[1].lower()
groups=json.load(sys.stdin)["devices"]
devices=[d for k,ds in groups.items() if ".iOS-" in k for d in ds if d["isAvailable"] and kind in d["name"].lower()]
devices.sort(key=lambda d:d["state"] != "Booted")
print(devices[0]["udid"] if devices else "")' "$mode")"
fi
[[ -n "$device" ]] || { echo "Install an iOS runtime and create an $mode Simulator in Xcode." >&2; exit 1; }
if ! xcrun simctl list devices booted | grep -Fq "$device"; then xcrun simctl boot "$device"; fi
xcrun simctl bootstatus "$device" -b
open -a Simulator
xcrun simctl openurl "$device" 'https://www.papamo.dev/sms/inbox'
mkdir -p "$out/$mode-$device"
echo "Capture each screen with:"
printf 'xcrun simctl io %q screenshot %q\n' "$device" "$out/$mode-$device/01-sign-in.png"

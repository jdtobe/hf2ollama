#!/usr/bin/env bash
# Verify a packaged hf2ollama macOS disk image.
#
#   packaging/macos/verify.sh <path/to/hf2ollama-<version>-macos-universal.dmg> <version>
#
# Checks the DMG, the .app layout, Info.plist, the universal binary, the icon,
# the ad-hoc signature and the Applications link. Exits non-zero on the first
# failure.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <dmg> <version>" >&2
  exit 2
fi

dmg="$1"
version="$2"
bundle_id="io.github.jdtobe.hf2ollama"

[ -f "$dmg" ] || { echo "FAIL: no DMG at $dmg" >&2; exit 1; }

tmp="$(mktemp -d "${TMPDIR:-/tmp}/hf2ollama-verify.XXXXXX")"
mnt="$tmp/mnt"
mkdir -p "$mnt"
attached=0
cleanup() {
  if [ "$attached" -eq 1 ]; then
    hdiutil detach "$mnt" -quiet || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

echo "==> verifying DMG checksum: $dmg"
hdiutil verify "$dmg" >/dev/null

echo "==> mounting"
hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null
attached=1

app="$mnt/hf2ollama.app"
[ -d "$app" ] || { echo "FAIL: hf2ollama.app is missing from the DMG" >&2; exit 1; }
[ -L "$mnt/Applications" ] || { echo "FAIL: the Applications link is missing" >&2; exit 1; }
[ "$(readlink "$mnt/Applications")" = "/Applications" ] || {
  echo "FAIL: the Applications link does not point at /Applications" >&2
  exit 1
}

plist="$app/Contents/Info.plist"
exe="$app/Contents/MacOS/hf2ollama"
icon="$app/Contents/Resources/icon.icns"

for f in "$plist" "$exe" "$icon" \
  "$app/Contents/Resources/LICENSE-MIT" \
  "$app/Contents/Resources/LICENSE-APACHE" \
  "$app/Contents/Resources/THIRD_PARTY_LICENSES.md" \
  "$app/Contents/Resources/README.md"; do
  [ -s "$f" ] || { echo "FAIL: missing or empty $f" >&2; exit 1; }
done
[ -x "$exe" ] || { echo "FAIL: $exe is not executable" >&2; exit 1; }

echo "==> checking Info.plist"
plutil -lint "$plist" >/dev/null
[ "$(plutil -extract CFBundleIdentifier raw -o - "$plist")" = "$bundle_id" ] || {
  echo "FAIL: wrong CFBundleIdentifier" >&2
  exit 1
}
[ "$(plutil -extract CFBundleShortVersionString raw -o - "$plist")" = "$version" ] || {
  echo "FAIL: CFBundleShortVersionString is not $version" >&2
  exit 1
}
[ "$(plutil -extract CFBundleVersion raw -o - "$plist")" = "$version" ] || {
  echo "FAIL: CFBundleVersion is not $version" >&2
  exit 1
}
[ "$(plutil -extract CFBundleExecutable raw -o - "$plist")" = "hf2ollama" ] || {
  echo "FAIL: CFBundleExecutable is not hf2ollama" >&2
  exit 1
}
[ "$(plutil -extract CFBundleIconFile raw -o - "$plist")" = "icon.icns" ] || {
  echo "FAIL: CFBundleIconFile is not icon.icns" >&2
  exit 1
}

echo "==> checking the universal binary"
archs="$(lipo -archs "$exe")"
case " $archs " in
  *" arm64 "*) ;;
  *) echo "FAIL: arm64 slice missing (got: $archs)" >&2; exit 1 ;;
esac
case " $archs " in
  *" x86_64 "*) ;;
  *) echo "FAIL: x86_64 slice missing (got: $archs)" >&2; exit 1 ;;
esac

echo "==> checking the icon"
iconutil -c iconset -o "$tmp/icon.iconset" "$icon"

echo "==> checking the signature"
codesign --verify --deep --strict "$app"

echo "OK: $dmg ($version, $archs, signature valid)"

#!/bin/zsh
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
package_dir="$project_root/macos/VoiceInputApp"
output_dir="$project_root/dist"
final_app="$output_dir/Voice Input.app"
staging_root="$(mktemp -d "$project_root/.voice-input-app.XXXXXX")"
staging_app="$staging_root/Voice Input.app"
signing_identity="${VOICE_INPUT_CODESIGN_IDENTITY:--}"

cleanup() {
  /bin/rm -rf "$staging_root"
}
trap cleanup EXIT

cd "$project_root"
cargo build --release

developer_dir="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
DEVELOPER_DIR="$developer_dir" swift build \
  --package-path "$package_dir" \
  --configuration release
swift_bin_dir="$(DEVELOPER_DIR="$developer_dir" swift build \
  --package-path "$package_dir" \
  --configuration release \
  --show-bin-path)"

mkdir -p "$staging_app/Contents/MacOS"
mkdir -p "$staging_app/Contents/Helpers"
mkdir -p "$staging_app/Contents/Resources"
/usr/bin/ditto "$swift_bin_dir/VoiceInputApp" "$staging_app/Contents/MacOS/Voice Input"
/usr/bin/ditto "$project_root/target/release/voice-input" "$staging_app/Contents/Helpers/voice-input"
/usr/bin/ditto "$package_dir/Resources/Info.plist" "$staging_app/Contents/Info.plist"
chmod 755 "$staging_app/Contents/MacOS/Voice Input" "$staging_app/Contents/Helpers/voice-input"

/usr/bin/codesign --force --sign "$signing_identity" --options runtime --timestamp=none \
  "$staging_app/Contents/Helpers/voice-input"
/usr/bin/codesign --force --sign "$signing_identity" --options runtime --timestamp=none \
  "$staging_app"
/usr/bin/codesign --verify --deep --strict "$staging_app"
/usr/bin/plutil -lint "$staging_app/Contents/Info.plist"

mkdir -p "$output_dir"
if [[ "$final_app" != "$project_root/dist/Voice Input.app" ]]; then
  echo "refusing unexpected app destination: $final_app" >&2
  exit 1
fi
/bin/rm -rf "$final_app"
/bin/mv "$staging_app" "$final_app"

echo "$final_app"

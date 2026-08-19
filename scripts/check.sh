#!/bin/zsh
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
developer_dir="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

cd "$project_root"
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
DEVELOPER_DIR="$developer_dir" swift test --package-path "$project_root/macos/VoiceInputApp"

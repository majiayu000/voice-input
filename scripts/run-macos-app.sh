#!/bin/zsh
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
app_path="$project_root/dist/Voice Input.app"
"$script_dir/build-macos-app.sh"
/usr/bin/open "$app_path"
echo "Voice Input 已启动；请查看 macOS 菜单栏中的“听”。"

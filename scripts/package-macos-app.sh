#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

cargo build --release -p sideromelane-app

app_name="Sideromelane"
binary_name="sideromelane"
package_root="${repo_root}/target/package"
app_dir="${package_root}/${app_name}.app"
contents_dir="${app_dir}/Contents"
macos_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"

rm -rf "${app_dir}"
mkdir -p "${macos_dir}" "${resources_dir}"

cp "${repo_root}/target/release/${binary_name}" "${macos_dir}/${binary_name}"
chmod +x "${macos_dir}/${binary_name}"

cat > "${contents_dir}/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>sideromelane</string>
  <key>CFBundleIdentifier</key>
  <string>dev.nigelupstart.sideromelane</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Sideromelane</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

printf '%s\n' "${app_dir}"

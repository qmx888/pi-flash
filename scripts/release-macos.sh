#!/usr/bin/env bash
# pi-flash macOS 打包（.app + DMG）。
#
# 用法：
#   scripts/release-macos.sh <版本号> [说明文件.md]
#   例：scripts/release-macos.sh 0.2.0 tmp/release-notes-0.2.0.md
#
# 产出：
#   dist/pi-flash.app                                  未压缩的应用包
#   dist/pi-flash-<版本>-darwin-arm64.dmg              拖拽安装镜像
#   dist/pi-flash-<版本>-darwin-arm64.zip              应用内更新用的载荷（npm 薄壳同款布局）
#   dist/pi-flash-<版本>-darwin-arm64.dmg.sha256       校验和
#   dist/SHA256SUMS
#
# 与 Windows 的 scripts/release.sh 的差异：
#   - zip 根就是 `pi-flash.app`（不套 `pi-flash/` 包装目录；npm 薄壳与
#     应用内更新都按 `<root>/pi-flash.app/Contents/MacOS/pi-flash` 找载荷）
#   - 不碰 CHANGELOG（发版说明仍由 Windows 那条链路收编；这里只打包）
#   - 默认 ad-hoc 签名（`--sign -`）；设了 PF_MAC_IDENTITY 则用真证书签名
#   - 设了 PF_MAC_NOTARY_PROFILE 则额外提交公证 + staple
#
# ⛔ 本脚本绝不跑格式化：不在这里插任何 `cargo fmt` / `rustfmt` 步骤。

set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

VERSION=${1:-}
NOTES_FILE=${2:-}
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "用法: scripts/release-macos.sh <x.y.z> [说明文件.md]"; exit 1; }
[[ -z "$NOTES_FILE" || -f "$NOTES_FILE" ]] || { echo "说明文件不存在: $NOTES_FILE"; exit 1; }

# 宿主必须是 Apple silicon（本脚本只打 arm64；Intel 目标请自行加 --target 分支）
[[ "$(uname -s)" == "Darwin" ]] || { echo "✗ 本脚本只能在 macOS 上跑"; exit 1; }
[[ "$(uname -m)" == "arm64" ]] || echo "⚠ 宿主不是 arm64，产物按本机架构构建"

TAG="darwin-$( [[ "$(uname -m)" == "arm64" ]] && echo arm64 || echo x64 )"
APP_NAME="pi-flash.app"
APP="dist/$APP_NAME"
DMG="dist/pi-flash-${VERSION}-${TAG}.dmg"
ZIP="dist/pi-flash-${VERSION}-${TAG}.zip"

# --- 0. 前置校验 -----------------------------------------------------------
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  echo "✗ 工作区有未提交改动，先提交再打包："
  git status --short
  exit 1
fi
[[ -f vendor/pi/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js ]] || {
  echo "✗ vendor/pi 未安装（cd vendor/pi && npm ci）"; exit 1;
}

echo "==> 1/7 release 构建（$TAG）"
# macOS 不用 Metal shader 编译链（CommandLineTools 没有 `metal` 工具，gpui 的
# build.rs 会 xcrun 失败）。gpui 的 macos-blade 特性改走 WGSL/blade 渲染后端，
# 全程不需要 Xcode——这是本机（只装 CommandLineTools）能出包的唯一路径。
CARGO_FEATURES=(--features gpui/macos-blade)
cargo build --release -p app "${CARGO_FEATURES[@]}"

echo "==> 2/7 组装 $APP_NAME"
rm -rf "$APP" "$DMG" "$ZIP" "$DMG.sha256" dist/SHA256SUMS
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# 主可执行文件只能有这一个：Contents/MacOS 里多放东西会让外层 codesign 报
# "code object is not signed at all"（同 CI）。node / vendor / 图标全进
# Contents/Resources，pi_link::vendor 的搜索路径已含它。
cp target/release/pi-flash "$APP/Contents/MacOS/pi-flash"

NODE_BIN=${PI_FLASH_NODE:-$(command -v node || true)}
[[ -n "$NODE_BIN" ]] || { echo "✗ 找不到 node（PATH 或 PI_FLASH_NODE）"; exit 1; }
cp "$NODE_BIN" "$APP/Contents/Resources/node"
chmod +x "$APP/Contents/Resources/node"

cp -R vendor "$APP/Contents/Resources/vendor"
[[ -f crates/app/assets/macos/pi-flash.icns ]] && \
  cp crates/app/assets/macos/pi-flash.icns "$APP/Contents/Resources/pi-flash.icns"

# subagent 扩展（045）：运行时按 `-e <path>` 挂载，放在 Resources 下与 Windows 同构
if [[ -d extensions/pi-subagents ]]; then
  mkdir -p "$APP/Contents/Resources/extensions"
  cp -R extensions/pi-subagents "$APP/Contents/Resources/extensions/"
fi

# 裁非宿主 esbuild 二进制（与 release.sh 同规则；pi 1.2.0+ 已无 esbuild 则跳过）
for ESD in \
  "$APP/Contents/Resources/vendor/pi/node_modules/@earendil-works/pi-coding-agent/node_modules/@esbuild" \
  "$APP/Contents/Resources/vendor/pi/node_modules/@esbuild"; do
  [[ -d "$ESD/$TAG" ]] || continue
  for d in "$ESD"/*; do
    [[ "$(basename "$d")" == "$TAG" ]] || rm -rf "$d"
  done
  echo "    esbuild 裁剪完成（保留 $TAG）"
done

# 版本号同时写进 Info.plist 与 vendor 旁的戳，供应用内更新对账
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>pi-flash</string>
  <key>CFBundleDisplayName</key><string>pi-flash</string>
  <key>CFBundleIdentifier</key><string>dev.pi-flash.app</string>
  <key>CFBundleExecutable</key><string>pi-flash</string>
  <key>CFBundleIconFile</key><string>pi-flash</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
printf '%s\n' "$VERSION" > "$APP/Contents/Resources/.payload-version"

echo "==> 3/7 签名"
# 由内而外：先签 bundle 内所有 Mach-O，再签最外层。
# 禁用 codesign --deep：它会把 node_modules 里带点号的目录（.bin、ts3.4…）
# 误判成 bundle（Apple TN2206 也不推荐 --deep）。
IDENTITY=${PF_MAC_IDENTITY:--}
SIGN_FLAGS=(--force --sign "$IDENTITY")
[[ "$IDENTITY" != "-" ]] && SIGN_FLAGS+=(--options runtime --timestamp)
while IFS= read -r -d '' f; do
  case "$(file -b "$f")" in
    Mach-O*) codesign "${SIGN_FLAGS[@]}" "$f" || exit 1 ;;
  esac
done < <(find "$APP" -type f \( -perm -u+x -o -name '*.node' -o -name '*.dylib' \) -print0)
codesign "${SIGN_FLAGS[@]}" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
echo "    签名身份：$IDENTITY"

if [[ -n "${PF_MAC_NOTARY_PROFILE:-}" ]]; then
  echo "==> 3b 公证（notarytool profile: $PF_MAC_NOTARY_PROFILE）"
  ditto -c -k --keepParent "$APP" "dist/notarize.zip"
  xcrun notarytool submit "dist/notarize.zip" --keychain-profile "$PF_MAC_NOTARY_PROFILE" --wait
  rm -f "dist/notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
fi

echo "==> 4/7 包内烟测（独立目录启动）"
SMOKE_LOG="$ROOT/dist/smoke.log"
# 烟测只验「启动无输出」：release 构建默认跑更新检查，GitHub Release 还没
# 创建时 /releases/latest 必 404（首发鸡生蛋），这里关掉。
PI_FLASH_UPDATE_CHECK=0
export PI_FLASH_UPDATE_CHECK
(cd / && "$ROOT/$APP/Contents/MacOS/pi-flash" > "$SMOKE_LOG" 2>&1 &)
sleep 6
if ! pgrep -f "$ROOT/$APP/Contents/MacOS/pi-flash" > /dev/null; then
  echo "✗ 打包后启动失败："; cat "$SMOKE_LOG"; exit 1
fi
pkill -f "$ROOT/$APP/Contents/MacOS/pi-flash" || true
sleep 1
# 启动期会往 stderr 打一行 `[startup] dev-flags: …` 的常规状态（不是错误，
# 开发/正式构建都打）。烟测只拦**非该行**的输出。
SMOKE_BAD=$(grep -v '^\[startup\] ' "$SMOKE_LOG" || true)
if [[ -n "$SMOKE_BAD" ]]; then
  echo "✗ 启动有异常输出："; printf '%s\n' "$SMOKE_BAD"; exit 1
fi
rm -f "$SMOKE_LOG"
echo "    启动正常"

echo "==> 5/7 生成 $DMG（拖拽安装布局）"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
# UDZO = zlib 压缩，兼容 macOS 13+；HFS+ 卷名不带版本，Finder 里显示为 pi-flash
hdiutil create -volname "pi-flash" -srcfolder "$STAGE" -ov -format UDZO "$DMG" > /dev/null
hdiutil verify "$DMG" > /dev/null

echo "==> 6/7 生成 $ZIP（应用内更新载荷）"
# 应用内更新 / npm 薄壳解压后按 `<root>/pi-flash.app` 找 exe（lib.js exeRelPath），
# 所以 zip 根必须是 .app 本身，不能套包装目录。
(cd dist && zip -qry "$(basename "$ZIP")" "$APP_NAME")

echo "==> 7/7 校验和"
(cd dist && shasum -a 256 "$(basename "$DMG")" "$(basename "$ZIP")" | tee SHA256SUMS)
shasum -a 256 "$DMG" | awk '{print $1"  '"$(basename "$DMG")"'"}' > "$DMG.sha256"

echo
echo "完成："
ls -lh "$DMG" "$ZIP" "$DMG.sha256" dist/SHA256SUMS
cat <<EOF

挂资产（tag 已存在时）：
  gh release upload v${VERSION} "$DMG" "$DMG.sha256" "$ZIP" --clobber

说明：DMG 给用户手动安装（拖进 Applications）；ZIP 给应用内更新与 npm 薄壳
（两者都按 <root>/pi-flash.app/Contents/MacOS/pi-flash 找载荷）。
EOF

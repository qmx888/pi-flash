# pi-flash 开发/打包任务入口。
#
# 用法：just            （列出全部任务）
#      just <task>     （执行）
#
# 平台：macOS 用 `dmg` / `publish`；Windows 用 `zip`。
# 不跨平台的 task 会在开头自检并给出提示，不会跑一半才失败。

set shell := ["bash", "-euo", "pipefail", "-c"]

# 版本号（唯一事实源：crates/app/Cargo.toml）
version := `sed -n 's/^version = "\(.*\)"$/\1/p' crates/app/Cargo.toml | head -1`
# 宿主平台
is_mac := `[ "$(uname -s)" = "Darwin" ] && echo 1 || echo 0`
arch := `uname -m`

# ---------------------------------------------------------------- 默认

# 列出所有任务
default:
    @just --list

# ---------------------------------------------------------------- 开发

# debug 构建并启动（不打包）
dev:
    cargo run -p app --features gpui/macos-blade

# 只编译检查 app（快速）
check:
    cargo check -p app --features gpui/macos-blade

# 只编译检查 pi-link（协议层，无平台依赖）
check-link:
    cargo check -p pi-link

# ---------------------------------------------------------------- 测试

# 全部测试（pi-link + app）
test:
    RUST_MIN_STACK=33554432 cargo test -p pi-link
    RUST_MIN_STACK=33554432 cargo test -p app --features gpui/macos-blade

# 只跑协议/路径测试
test-link:
    RUST_MIN_STACK=33554432 cargo test -p pi-link

# 只跑界面测试
test-app:
    RUST_MIN_STACK=33554432 cargo test -p app --features gpui/macos-blade

# 架构守护规则（文件/函数行数红线）
check-arch:
    bash scripts/check_arch.sh

# ---------------------------------------------------------------- 打包

# release 构建（只出二进制，不组装 .app）
build:
    cargo build --release -p app --features gpui/macos-blade

# 打包为 DMG + ZIP（macOS）
dmg:
    @echo "一条龙：构建 → 组装 .app → 签名 → 冒烟 → DMG → ZIP → 校验和"
    @echo "产物：dist/pi-flash-{{version}}-darwin-{{arch}}.{dmg,zip}"
    @echo "真证书：PF_MAC_IDENTITY=\"Developer ID Application: ...\" just dmg"
    @echo "公证：  PF_MAC_NOTARY_PROFILE=<keychain-profile> just dmg"
    bash scripts/release-macos.sh {{version}}

# bundle 是 dmg 的别名（习惯叫法）
alias b := dmg

# Windows 绿色包 zip（需发版说明文件）
zip ver notes:
    bash scripts/release.sh {{ver}} {{notes}}

# 打包并挂到已存在的 GitHub release
publish ver:
    bash scripts/release-macos.sh {{ver}}
    gh release upload "v{{ver}}" \
        "dist/pi-flash-{{ver}}-darwin-{{arch}}.dmg" \
        "dist/pi-flash-{{ver}}-darwin-{{arch}}.dmg.sha256" \
        "dist/pi-flash-{{ver}}-darwin-{{arch}}.zip" --clobber

# ---------------------------------------------------------------- 清理

# 清理 target/ 与 dist/（保留 vendor 的 node_modules）
clean:
    cargo clean
    rm -rf dist
    rm -rf pi-flash-dev/logs
    @echo "已清理：target/ dist/ pi-flash-dev/logs/"

# 深度清理（含 dev 数据目录与 tmp/）
clean-all: clean
    @echo "会删掉本地启动状态（last-open / 窗口位置 / 自动化实例登记）"
    @echo "pi 自己的数据（~/.pi/agent）不受影响"
    rm -rf pi-flash-dev
    rm -rf tmp

# 只清打包产物 dist/（保留 target/，免得下次全量重编译）
clean-dist:
    rm -rf dist

# 只清 dev 数据目录（下次启动自动重建；debug 构建的 home 是 ./pi-flash-dev）
clean-dev:
    rm -rf pi-flash-dev

# 清 vendor/pi 的 node_modules（释放 ~148MB；恢复需 npm ci，几分钟）
clean-vendor:
    rm -rf vendor/pi/node_modules
    @echo "恢复：cd vendor/pi && npm ci"

# ---------------------------------------------------------------- 杂项

# 格式化（仓库 pre-commit 口径）
fmt:
    bash scripts/fmt.sh

# 查看构建体积构成（需先 just dmg）
size:
    @[ -d dist/pi-flash.app ] || { echo "dist/pi-flash.app 不存在，先跑 just dmg"; exit 1; }
    du -sh dist/pi-flash.app
    du -sh dist/pi-flash.app/Contents/MacOS dist/pi-flash.app/Contents/Resources/* | sort -rh
    @ls -lh dist/*.dmg dist/*.zip 2>/dev/null || true

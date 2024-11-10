#!/usr/bin/env sh

set -ex

package=leaf-ffi
name=leaf
lib=lib$name.a

# The script is assumed to run in the root of the workspace
base=$(dirname "$0")

export IPHONEOS_DEPLOYMENT_TARGET=10.0

# Strict workspace root check
if [ ! -f "Cargo.toml" ] || [ ! -d "leaf" ] || [ ! -d "leaf-cli" ] || [ ! -d "leaf-ffi" ]; then
    echo "错误: 此脚本必须在工作空间根目录下运行！"
    echo "请确保当前目录包含以下文件和目录："
    echo "  - Cargo.toml"
    echo "  - leaf/"
    echo "  - leaf-cli/"
    echo "  - leaf-ffi/"
    exit 1
fi

# 添加目标架构
rustup target add aarch64-apple-ios-sim    # 用于 "My Mac (Designed for iPad)" 调试
rustup target add aarch64-apple-ios        # 用于实机运行

# 为调试版本设置更完整的编译标记
export RUSTFLAGS="-C debuginfo=2 -C symbol-mangling-version=v0"
export CARGO_PROFILE_DEBUG_DEBUG=true
export CARGO_PROFILE_DEBUG_SPLIT_DEBUGINFO="packed"  # 或者 "unpacked"

# iOS 模拟器 debug 版本
cargo build -p $package \
    --no-default-features \
    --features "trojan-only" \
    --target aarch64-apple-ios-sim

# iOS 实机 release 版本
unset RUSTFLAGS
unset CARGO_PROFILE_DEBUG_DEBUG
unset CARGO_PROFILE_DEBUG_SPLIT_DEBUGINFO
cargo build -p $package \
    --release \
    --no-default-features \
    --features "trojan-only" \
    --target aarch64-apple-ios

cargo install --force cbindgen

# 准备目录
rm -rf target/apple/universal
mkdir -p target/apple/universal/include
mkdir -p target/apple/universal/ios-sim-debug    # iOS 模拟器调试版
mkdir -p target/apple/universal/ios-release      # iOS 实机发布版

# 复制库文件（移除 dsymutil 步骤）
cp target/aarch64-apple-ios-sim/debug/$lib target/apple/universal/ios-sim-debug/
cp target/aarch64-apple-ios/release/$lib target/apple/universal/ios-release/

# 生成头文件
cbindgen \
    --config $package/cbindgen.toml \
    $package/src/lib.rs > target/apple/universal/include/$name.h

# 创建 modulemap
cat << EOF > "target/apple/universal/include/module.modulemap"
module $name {
    header "$name.h"
    export *
}
EOF

# 创建 XCFramework（移除 debug-symbols 参数）
xcodebuild -create-xcframework \
    -library target/apple/universal/ios-sim-debug/$lib \
    -headers target/apple/universal/include \
    -library target/apple/universal/ios-release/$lib \
    -headers target/apple/universal/include \
    -output target/apple/universal/$name.xcframework

ls target/apple/universal/$name.xcframework
open target/apple/universal
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
rustup target add aarch64-apple-ios-sim    # 用于 "My Mac (Designed for iPad)"
rustup target add aarch64-apple-ios        # 用于实机运行

# 设置最高级别优化
export RUSTFLAGS="-C opt-level=3 -C target-cpu=native -C codegen-units=1"

# iOS 模拟器 release 版本
cargo build -p $package \
    --release \
    --no-default-features \
    --features "trojan-only" \
    --target aarch64-apple-ios-sim

# iOS 实机 release 版本
cargo build -p $package \
    --release \
    --no-default-features \
    --features "trojan-only" \
    --target aarch64-apple-ios

cargo install --force cbindgen

# 准备目录
rm -rf target/apple/universal
mkdir -p target/apple/universal/include
mkdir -p target/apple/universal/ios-sim-release    # iOS 模拟器发布版
mkdir -p target/apple/universal/ios-release        # iOS 实机发布版

# 复制库文件
cp target/aarch64-apple-ios-sim/release/$lib target/apple/universal/ios-sim-release/
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

# 创建 XCFramework
xcodebuild -create-xcframework \
    -library target/apple/universal/ios-sim-release/$lib \
    -headers target/apple/universal/include \
    -library target/apple/universal/ios-release/$lib \
    -headers target/apple/universal/include \
    -output target/apple/universal/$name.xcframework

ls target/apple/universal/$name.xcframework
open target/apple/universal
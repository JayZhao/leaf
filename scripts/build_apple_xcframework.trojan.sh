#!/usr/bin/env sh

# 设置脚本编码为 UTF-8
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8
export LANGUAGE=en_US.UTF-8

# 默认构建模式为 release
BUILD_MODE="release"

# 解析命令行参数
while [ "$#" -gt 0 ]; do
    case "$1" in
        --debug)
            BUILD_MODE="debug"
            ;;
        --release)
            BUILD_MODE="release"
            ;;
        *)
            echo "未知参数: $1"
            echo "用法: $0 [--debug|--release]"
            exit 1
            ;;
    esac
    shift
done

clear

printf "开始构建 XCFramework... (模式: %s)\n" "$BUILD_MODE"

# 启用错误检查和命令回显
set -ex

# 设置基础变量
package=leaf-ffi
name=leaf
lib=lib$name.a

# 假定脚本在工作空间根目录运行
base=$(dirname "$0")

# 设置 iOS 最低部署版本
export IPHONEOS_DEPLOYMENT_TARGET=10.0

# 严格的工作空间根目录检查
if [ ! -f "Cargo.toml" ] || [ ! -d "leaf" ] || [ ! -d "leaf-cli" ] || [ ! -d "leaf-ffi" ]; then
    echo "错误: 此脚本必须在工作空间根目录下运行！"
    echo "请确保当前目录包含以下文件和目录："
    echo "  - Cargo.toml"
    echo "  - leaf/"
    echo "  - leaf-cli/"
    echo "  - leaf-ffi/"
    exit 1
fi

printf "正在添加目标架构...\n"
rustup target add aarch64-apple-ios-sim    # 用于 "My Mac (Designed for iPad)"
rustup target add aarch64-apple-ios        # 用于实机运行

printf "设置编译标志...\n"
if [ "$BUILD_MODE" = "release" ]; then
    # Release 模式：最高优化，无调试信息
    export RUSTFLAGS="-C opt-level=3 -C target-cpu=native -C codegen-units=1"
else
    # Debug 模式：无优化，包含调试信息
    export RUSTFLAGS="-C opt-level=0 -g"
fi

# 根据构建模式调整路径
BUILD_DIR=$([ "$BUILD_MODE" = "release" ] && echo "release" || echo "debug")
mkdir -p target/apple/universal/ios-sim-$BUILD_MODE     # iOS 模拟器版本
mkdir -p target/apple/universal/ios-$BUILD_MODE         # iOS 实机版本

# 设置 Cargo 输出样式
export CARGO_TERM_PROGRESS_WHEN=never

printf "正在编译 iOS 模拟器版本...\n"
cargo build -p $package \
    --no-default-features \
    --features "trojan-only" \
    $([ "$BUILD_MODE" = "release" ] && echo "--release") \
    --target aarch64-apple-ios-sim \
    --message-format=short

printf "正在编译 iOS 实机版本...\n"
cargo build -p $package \
    --no-default-features \
    --features "trojan-only" \
    $([ "$BUILD_MODE" = "release" ] && echo "--release") \
    --target aarch64-apple-ios \
    --message-format=short

# 安装 cbindgen 工具
printf "正在安装 cbindgen...\n"
cargo install --force cbindgen --message-format=short

# 准备输出目录
printf "准备输出目录...\n"
rm -rf target/apple/universal
mkdir -p target/apple/universal/include
mkdir -p target/apple/universal/ios-sim-$BUILD_MODE     # iOS 模拟器版本
mkdir -p target/apple/universal/ios-$BUILD_MODE         # iOS 实机版本

# 修改复制路径
printf "复制编译产物...\n"
cp target/aarch64-apple-ios-sim/$BUILD_DIR/$lib target/apple/universal/ios-sim-$BUILD_MODE/
cp target/aarch64-apple-ios/$BUILD_DIR/$lib target/apple/universal/ios-$BUILD_MODE/

# 生成 C 语言头文件
printf "生成头文件...\n"
cbindgen \
    --config $package/cbindgen.toml \
    $package/src/lib.rs > target/apple/universal/include/$name.h

# 创建模块映射文件
printf "创建 modulemap...\n"
cat << EOF > "target/apple/universal/include/module.modulemap"
module $name {
    header "$name.h"
    export *
}
EOF

# 修改 XCFramework 创建命令中的路径
printf "正在创建 XCFramework...\n"
xcodebuild -create-xcframework \
    -library target/apple/universal/ios-sim-$BUILD_MODE/$lib \
    -headers target/apple/universal/include \
    -library target/apple/universal/ios-$BUILD_MODE/$lib \
    -headers target/apple/universal/include \
    -output target/apple/universal/$name.xcframework

# 验证输出
printf "验证 XCFramework 创建结果...\n"
ls target/apple/universal/$name.xcframework

printf "打开输出目录...\n"
open target/apple/universal

printf "XCFramework 构建完成！✨\n"
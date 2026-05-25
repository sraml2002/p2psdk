#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SDK_DIR="$SCRIPT_DIR/sdk"
DEMO_DIR="$SCRIPT_DIR/app-test-rust"
DIST_DIR="$DEMO_DIR/dist"

OHOS_TARGET="aarch64-unknown-linux-ohos"

# 解析参数
TARGET="$OHOS_TARGET"
while [[ $# -gt 0 ]]; do
  case $1 in
    --target) TARGET="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; echo "Usage: $0 [--target <triple>]"; exit 1 ;;
  esac
done

if [[ "$TARGET" == "$OHOS_TARGET" ]]; then
  TARGET_LABEL="OHOS ($OHOS_TARGET)"
  CARGO_TARGET_FLAG="--target $OHOS_TARGET"
  # 交叉编译需要 OHOS NDK 链接器配置
  mkdir -p "$DEMO_DIR/.cargo"
  cp "$SDK_DIR/.cargo/config.toml" "$DEMO_DIR/.cargo/config.toml"
else
  TARGET_LABEL="macOS (native)"
  CARGO_TARGET_FLAG=""
fi

echo "=========================================="
echo "Building app-test-rust ($TARGET_LABEL)..."
echo "=========================================="

cd "$DEMO_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release $CARGO_TARGET_FLAG 2>&1

# 清理并创建输出目录
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# 复制可执行文件
if [[ "$TARGET" == "$OHOS_TARGET" ]]; then
  BIN_PATH="$DEMO_DIR/target/$OHOS_TARGET/release/app-test-rust"
else
  BIN_PATH="$DEMO_DIR/target/release/app-test-rust"
fi

if [[ -f "$BIN_PATH" ]]; then
  cp "$BIN_PATH" "$DIST_DIR/"
elif [[ -f "${BIN_PATH}.exe" ]]; then
  cp "${BIN_PATH}.exe" "$DIST_DIR/"
fi

# 复制配置样例
cp "$DEMO_DIR/config.example.json" "$DIST_DIR/"

echo ""
echo "=========================================="
echo "Build completed! ($TARGET_LABEL)"
echo "=========================================="
echo "Output: $DIST_DIR/"
ls -lh "$DIST_DIR/"

if [[ "$TARGET" == "$OHOS_TARGET" ]]; then
  echo ""
  echo "Deploy to device:"
  echo "  hdc file send $DIST_DIR/app-test-rust /data/local/tmp/"
  echo "  hdc file send $DIST_DIR/config.example.json /data/local/tmp/"
  echo "  hdc shell chmod +x /data/local/tmp/app-test-rust"
  echo ""
  echo "Run on device:"
  echo "  1. hdc shell"
  echo "  2. cd /data/local/tmp && cp config.example.json config.json && vi config.json"
  echo "  3. ./app-test-rust config.json"
else
  echo ""
  echo "Usage:"
  echo "  cd $DIST_DIR && cp config.example.json config.json && edit config.json"
  echo "  ./app-test-rust config.json"
fi

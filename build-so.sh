#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SDK_DIR="$SCRIPT_DIR/sdk"
TARGET_DIR="$SCRIPT_DIR/app-test-hmos/entry/libs/arm64-v8a"
SO_NAME="libppsdk.so"
TARGET="aarch64-unknown-linux-ohos"
TOKEN_FILE="$SDK_DIR/crates/p2p-napi/build.jwt.nogit"

# 解析 --token 参数
TOKEN=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --token) TOKEN="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# 如果传了 --token，写入 token 文件
if [[ -n "$TOKEN" ]]; then
  echo "$TOKEN" > "$TOKEN_FILE"
  echo "Token written to build.jwt.nogit"
fi

# 检查 token 文件
if [[ ! -f "$TOKEN_FILE" ]]; then
  echo "ERROR: $TOKEN_FILE not found."
  echo "Usage: $0 --token <JWT_TOKEN>"
  echo "  Or manually create $TOKEN_FILE with the JWT token."
  exit 1
fi

TOKEN_CONTENT="$(cat "$TOKEN_FILE" | tr -d '[:space:]')"
if [[ -z "$TOKEN_CONTENT" || "$TOKEN_CONTENT" == "change-me-to-valid-jwt-token" ]]; then
  echo "ERROR: $TOKEN_FILE contains placeholder or is empty."
  echo "Provide a valid JWT token: $0 --token <JWT_TOKEN>"
  exit 1
fi

echo "=========================================="
echo "Building Rust library for HarmonyOS..."
echo "=========================================="

cd "$SDK_DIR"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p ppsdk --release --target "$TARGET"

echo ""
echo "=========================================="
echo "Copying library to HarmonyOS project..."
echo "=========================================="

mkdir -p "$TARGET_DIR"
cp "$SDK_DIR/target/$TARGET/release/$SO_NAME" "$TARGET_DIR/"

echo ""
echo "=========================================="
echo "Build completed successfully!"
echo "=========================================="
echo "Library copied to: $TARGET_DIR/$SO_NAME"
ls -lh "$TARGET_DIR/$SO_NAME"

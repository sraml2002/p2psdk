#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SDK_DIR="$SCRIPT_DIR/sdk"
TARGET_DIR="$SCRIPT_DIR/app-test-hmos/entry/libs/arm64-v8a"
SO_NAME="libppsdk.so"
TARGET="aarch64-unknown-linux-ohos"
CONFIG_FILE="$SCRIPT_DIR/build.jwt.path"

# 解析参数
TOKEN_FILE_ARG=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --token-file) TOKEN_FILE_ARG="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# --token-file 便利参数：将路径写入配置文件
if [[ -n "$TOKEN_FILE_ARG" ]]; then
  if [[ ! -f "$TOKEN_FILE_ARG" ]]; then
    echo "ERROR: Token file not found: $TOKEN_FILE_ARG"
    exit 1
  fi
  # 解析为绝对路径后写入配置文件（保留原有注释）
  ABS_PATH="$(cd "$(dirname "$TOKEN_FILE_ARG")" && pwd)/$(basename "$TOKEN_FILE_ARG")"
  echo "#NAT服务认证Token文件路径（线下申请）" > "$CONFIG_FILE"
  echo "$ABS_PATH" >> "$CONFIG_FILE"
  echo "Token file path saved to $CONFIG_FILE"
fi

# 读取配置文件中的 token 文件路径
if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "ERROR: Config file not found: $CONFIG_FILE"
  echo "Create it with the absolute path to your JWT token file."
  echo "  Example: echo '/path/to/build.jwt.nogit' > $CONFIG_FILE"
  exit 1
fi

TOKEN_FILE=""
while IFS= read -r line; do
  trimmed="$(echo "$line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
  [[ -z "$trimmed" || "$trimmed" == \#* ]] && continue
  TOKEN_FILE="$trimmed"
  break
done < "$CONFIG_FILE"

if [[ -z "$TOKEN_FILE" ]]; then
  echo "ERROR: No valid path found in $CONFIG_FILE"
  echo "Add the absolute path to your JWT token file (one per line, # for comments)."
  exit 1
fi

if [[ ! -f "$TOKEN_FILE" ]]; then
  echo "ERROR: Token file not found: $TOKEN_FILE (from $CONFIG_FILE)"
  exit 1
fi

TOKEN_CONTENT="$(tr -d '[:space:]' < "$TOKEN_FILE")"
if [[ -z "$TOKEN_CONTENT" || "$TOKEN_CONTENT" == "change-me-to-valid-jwt-token" ]]; then
  echo "ERROR: Token file '$TOKEN_FILE' contains placeholder or is empty."
  exit 1
fi

echo "=========================================="
echo "Building Rust library for HarmonyOS..."
echo "Token file: $TOKEN_FILE"
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

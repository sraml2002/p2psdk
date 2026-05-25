#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SDK_DIR="$SCRIPT_DIR/sdk"
DEMO_DIR="$SCRIPT_DIR/app-test-rust"
TARGET="aarch64-unknown-linux-ohos"
TOKEN_FILE="$SDK_DIR/crates/p2p-napi/build.jwt.nogit"
DIST_DIR="$SCRIPT_DIR/app-test-rust/dist"

export PATH="$HOME/.cargo/bin:$PATH"

# 解析参数
NO_SO=0
while [[ $# -gt 0 ]]; do
  case $1 in
    --token) echo "$2" > "$TOKEN_FILE"; shift 2 ;;
    --no-so) NO_SO=1; shift ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# ── 构建 libppsdk.so ─────────────────────────────────────────────
if [[ "$NO_SO" -eq 0 ]]; then
  if [[ ! -f "$TOKEN_FILE" ]] || [[ "$(tr -d '[:space:]' < "$TOKEN_FILE")" == "change-me-to-valid-jwt-token" ]]; then
    echo "ERROR: $TOKEN_FILE missing or placeholder. Use --token <JWT> to set."
    exit 1
  fi
  echo "[1/2] Building libppsdk.so..."
  cd "$SDK_DIR"
  cargo build -p ppsdk --release --target "$TARGET"
fi

# ── 构建 app-test-rust ───────────────────────────────────────────
echo "[2/2] Building app-test-rust..."
cd "$DEMO_DIR"
cargo build --release --target "$TARGET"

# ── 收集产物 ─────────────────────────────────────────────────────
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
cp "$SDK_DIR/target/$TARGET/release/libppsdk.so" "$DIST_DIR/"
cp "$DEMO_DIR/target/$TARGET/release/app-test-rust" "$DIST_DIR/"
cp "$DEMO_DIR/config.example.json" "$DIST_DIR/config.example.json"

echo ""
echo "Done! Output:"
ls -lh "$DIST_DIR/"
echo ""
echo "Deploy and run:"
echo "  hdc file send $DIST_DIR/libppsdk.so /data/local/tmp/"
echo "  hdc file send $DIST_DIR/app-test-rust /data/local/tmp/"
echo "  hdc file send config.json /data/local/tmp/"
echo '  hdc shell "cd /data/local/tmp && chmod +x app-test-rust && ./app-test-rust config.json ./libppsdk.so"'

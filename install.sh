#!/usr/bin/env sh
# Ekbasis universal installer script for Linux and macOS.
set -e

REPO="Yato-Works/Ekbasis"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

printf "\033[1;36m==>\033[0m Installing \033[1mEkbasis\033[0m (Software Timeline & Counterfactual Experiment Engine)...\n"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)
    printf "\033[1;31mError:\033[0m Unsupported architecture: %s\n" "$ARCH" >&2
    exit 1
    ;;
esac

mkdir -p "$INSTALL_DIR"

# 1. Try downloading pre-built binary from GitHub Releases
DOWNLOAD_SUCCESS=0
TAG="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)"

if [ -n "$TAG" ]; then
  ASSET_URL="https://github.com/$REPO/releases/download/$TAG/ekbasis-${TAG}-${ARCH}-${OS}.tar.gz"
  TMP_DIR="$(mktemp -d)"
  if curl -fsSL "$ASSET_URL" -o "$TMP_DIR/ekbasis.tar.gz" 2>/dev/null; then
    tar -xzf "$TMP_DIR/ekbasis.tar.gz" -C "$TMP_DIR"
    mv "$TMP_DIR/ekbasis" "$INSTALL_DIR/ekbasis"
    chmod +x "$INSTALL_DIR/ekbasis"
    rm -rf "$TMP_DIR"
    DOWNLOAD_SUCCESS=1
  fi
fi

# 2. Fall back to cargo install if release asset is not yet published
if [ "$DOWNLOAD_SUCCESS" -eq 0 ]; then
  if command -v cargo >/dev/null 2>&1; then
    printf "\033[1;33mNote:\033[0m Pre-built release binary not found for %s-%s. Building from source via Cargo...\n" "$OS" "$ARCH"
    cargo install --git "https://github.com/$REPO.git" ekbasis-cli --bin ekbasis --root "${INSTALL_DIR%/bin}"
    DOWNLOAD_SUCCESS=1
  else
    printf "\033[1;31mError:\033[0m Neither pre-built binary nor Cargo was found.\n" >&2
    printf "Please install Rust (https://rustup.rs/) or download releases manually from:\n" >&2
    printf "https://github.com/%s/releases\n" "$REPO" >&2
    exit 1
  fi
fi

printf "\033[1;32m==>\033[0m \033[1mEkbasis\033[0m successfully installed to %s/ekbasis\n" "$INSTALL_DIR"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    printf "\n\033[1;33mAction required:\033[0m Add %s to your PATH:\n" "$INSTALL_DIR"
    printf "  export PATH=\"%s:\$PATH\"\n\n" "$INSTALL_DIR"
    ;;
esac

printf "Run \033[1;32mekbasis --help\033[0m to get started!\n"

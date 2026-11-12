#!/usr/bin/env bash
#
# Build and package a release of sherpa-onnx.server.
#
# Produces a native-host folder + archive under dist/ containing:
#   - the main binary (renamed to `sherpa-onnx.server[.exe]`)
#   - llama-server[.exe] translation backend (optional)
#   - supplied llama runtime libraries, plus detected local binary dependencies
#   - README.md and LICENSE, when present
# See PACKAGING.md for loader setup, validation limits and license provenance.
#
# Usage:
#   ./package.sh [options]
#   LLAMA_BIN_DIR=/path/to/bin LLAMA_LIB_DIR=/path/to/lib ./package.sh
#   LLAMA_LIB_DIR is optional; set both to the same directory if needed.
#   Without LLAMA_BIN_DIR, only the ASR server is packaged.
#   THIRD_PARTY_NOTICES_DIR=/path/to/notices includes release license materials.
#
# Options:
#   --debug            Build/package the debug profile instead of release.
#   --no-llama         Do not bundle llama-server (ASR-only package).
#   --no-strip         Do not strip symbols from the binaries.
#   --out DIR          Output directory (default: dist).
#   -h, --help         Show this help.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PROFILE="release"
BUNDLE_LLAMA=1
STRIP=1
OUT_DIR="dist"

while [ $# -gt 0 ]; do
    case "$1" in
        --debug) PROFILE="debug"; shift ;;
        --target|--target=*) echo "ERROR: only native builds are supported" >&2; exit 1 ;;
        --no-llama) BUNDLE_LLAMA=0; shift ;;
        --no-strip) STRIP=0; shift ;;
        --out) OUT_DIR="${2:?--out needs a value}"; shift 2 ;;
        -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
done

if [ $BUNDLE_LLAMA -eq 0 ]; then
    unset LLAMA_BIN_DIR LLAMA_LIB_DIR
fi
if [ -n "${LLAMA_LIB_DIR:-}" ] && [ -z "${LLAMA_BIN_DIR:-}" ]; then
    echo "ERROR: LLAMA_LIB_DIR requires LLAMA_BIN_DIR" >&2
    exit 1
fi

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -z "$VERSION" ] && VERSION="0.0.0"

# Resolve Cargo's actual output root before building. This also handles a
# configured CARGO_TARGET_DIR without assuming a workspace-local target/.
PYTHON=()
for candidate in python3 python py; do
    python_cmd=("$candidate")
    [ "$candidate" != "py" ] || python_cmd+=(-3)
    # WindowsApps aliases can exist on PATH without a working interpreter.
    if "${python_cmd[@]}" -c 'import sys; sys.exit(sys.version_info.major != 3)' >/dev/null 2>&1; then
        PYTHON=("${python_cmd[@]}")
        break
    fi
done
if [ ${#PYTHON[@]} -eq 0 ]; then
    echo "ERROR: Python 3 is required; install it and make python3, python, or py -3 available on PATH" >&2
    exit 1
fi
CARGO_METADATA="$(cargo metadata --locked --format-version 1 --no-deps)"
TARGET_ROOT="$(printf '%s\n' "$CARGO_METADATA" | "${PYTHON[@]}" -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"

# ── Build ────────────────────────────────────────────────────────────────
BUILD_ARGS=()
[ "$PROFILE" = "release" ] && BUILD_ARGS+=(--release)
# Override Cargo configuration that could otherwise select a foreign target.
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$HOST_TRIPLE" ] || { echo "ERROR: cannot determine native Rust target" >&2; exit 1; }
BUILD_ARGS+=(--locked --target "$HOST_TRIPLE")

echo "==> cargo build ${BUILD_ARGS[*]}"
cargo build "${BUILD_ARGS[@]}"

# ── Host / target metadata ───────────────────────────────────────────────
case "$(uname -s)" in
    Linux)  HOST_OS="linux" ;;
    Darwin) HOST_OS="macos" ;;
    MINGW*|MSYS*|CYGWIN*) HOST_OS="windows" ;;
    *) HOST_OS="unknown" ;;
esac

OS="$HOST_OS"
[ "$OS" != "unknown" ] || { echo "ERROR: unsupported host OS" >&2; exit 1; }
ARCH="${HOST_TRIPLE%%-*}"

EXE_SUFFIX=""
[ "$OS" = "windows" ] && EXE_SUFFIX=".exe"

TARGET_DIR="$TARGET_ROOT/$HOST_TRIPLE/$PROFILE"
BIN_SRC="$TARGET_DIR/sherpa-onnx-server$EXE_SUFFIX"
if [ ! -f "$BIN_SRC" ]; then
    echo "ERROR: built binary not found: $BIN_SRC" >&2
    exit 1
fi

echo "==> sherpa-onnx.server packager"
echo "    version : $VERSION"
echo "    profile : $PROFILE"
echo "    os/arch : $OS/$ARCH"
echo "    llama   : $([ -n "${LLAMA_BIN_DIR:-}" ] && echo yes || echo no)"

# ── Stage the package ────────────────────────────────────────────────────
PKG_NAME="sherpa-onnx.server-$VERSION-$OS-$ARCH"
STAGE="$OUT_DIR/$PKG_NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"

MAIN_BIN="sherpa-onnx.server$EXE_SUFFIX"
install -m 0755 "$BIN_SRC" "$STAGE/$MAIN_BIN"
echo "==> staged $MAIN_BIN"

if [ $BUNDLE_LLAMA -eq 1 ] && [ -n "${LLAMA_BIN_DIR:-}" ]; then
    LLAMA_SRC="$LLAMA_BIN_DIR/llama-server$EXE_SUFFIX"
    install -m 0755 "$LLAMA_SRC" "$STAGE/llama-server$EXE_SUFFIX"
    echo "==> staged llama-server$EXE_SUFFIX"
    if [ -n "${LLAMA_LIB_DIR:-}" ]; then
        [ -d "$LLAMA_LIB_DIR" ] || { echo "ERROR: LLAMA_LIB_DIR is not a directory" >&2; exit 1; }
        # Include dynamic backends too: they may not appear in binary dependencies.
        for lib in "$LLAMA_LIB_DIR"/*; do
            [ -f "$lib" ] || continue
            case "$OS:$lib" in
                windows:*.[dD][lL][lL]|macos:*.dylib|linux:*.so|linux:*.so.*)
                    install -m 0755 "$lib" "$STAGE/$(basename "$lib")"
                    ;;
            esac
        done
    fi
fi

# ── Copy the shared libraries the staged binaries actually need ──────────
# Resolve dependency names from explicit runtime inputs and the native build
# output. System dependencies remain host requirements.
lib_deps() {
    # Print direct dependency library file names of $1.
    local f="$1"
    case "$OS" in
        windows)
            objdump -p "$f" | sed -n 's/^[[:space:]]*DLL Name: //p'
            ;;
        macos)
            otool -L "$f" | sed '1d; s/^[[:space:]]*//; s/ (compatibility version.*//; s|.*/||'
            ;;
        *)
            readelf -dW "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p'
            ;;
    esac
}

resolve_local_libs() {
    # Inspect every staged backend as well as each newly copied dependency.
    # A sentinel also avoids empty-array nounset behavior in older Bash versions.
    local queue=("$@") seen=("") name src deps f visited prior dir
    while [ ${#queue[@]} -gt 0 ]; do
        f="${queue[0]}"; queue=("${queue[@]:1}")
        visited=0
        for prior in "${seen[@]}"; do
            [ "$prior" != "$f" ] || visited=1
        done
        [ "$visited" -eq 0 ] || continue
        seen+=("$f")
        deps="$(lib_deps "$f")"
        while IFS= read -r name; do
            [ -z "$name" ] && continue
            case "$name" in */*) echo "ERROR: dependency contains a path: $name" >&2; exit 1 ;; esac
            [ ! -f "$STAGE/$name" ] || continue
            # Explicit llama inputs take precedence over possibly stale build copies.
            for dir in "${LLAMA_LIB_DIR:-}" "${LLAMA_BIN_DIR:-}" "$TARGET_DIR"; do
                [ -n "$dir" ] || continue
                src="$dir/$name"
                if [ -f "$src" ]; then
                    install -m 0755 "$src" "$STAGE/$name"
                    echo "==> staged lib $name"
                    queue+=("$STAGE/$name")
                    break
                fi
            done
        done <<< "$deps"
    done
}

resolve_local_libs "$STAGE"/*

# ── Extra runtime data ───────────────────────────────────────────────────
for doc in README.md LICENSE PACKAGING.md THIRD_PARTY.md; do
    [ ! -f "$doc" ] || cp "$doc" "$STAGE/$doc"
done
if [ -n "${THIRD_PARTY_NOTICES_DIR:-}" ]; then
    [ -d "$THIRD_PARTY_NOTICES_DIR" ] || { echo "ERROR: THIRD_PARTY_NOTICES_DIR is not a directory" >&2; exit 1; }
    cp -R "$THIRD_PARTY_NOTICES_DIR" "$STAGE/third-party-notices"
fi

# ── Strip symbols ────────────────────────────────────────────────────────
if [ $STRIP -eq 1 ] && [ "$OS" != "windows" ]; then
    command -v strip >/dev/null || { echo "ERROR: strip unavailable; use --no-strip" >&2; exit 1; }
    for b in "$STAGE/$MAIN_BIN" "$STAGE/llama-server$EXE_SUFFIX"; do
        [ -f "$b" ] || continue
        strip "$b"
    done
    echo "==> stripped binaries"
fi

# ── Sanity check (Linux only) ────────────────────────────────────────────
if [ "$OS" = "linux" ] && [ -f ./check.sh ]; then
    echo "==> running check.sh on staged package"
    cp ./check.sh "$STAGE/check.sh"
    bash ./check.sh "$STAGE"
fi

# ── Archive ──────────────────────────────────────────────────────────────
ARCHIVE="$OUT_DIR/$PKG_NAME.tar.gz"
tar -C "$OUT_DIR" -czf "$ARCHIVE" "$PKG_NAME"

echo
echo "==> done"
echo "    folder : $STAGE"
echo "    archive: $ARCHIVE"
ls -la "$STAGE"

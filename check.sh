#!/usr/bin/env bash
# Check trusted, locally built ELF files against this Linux host only.
set -euo pipefail

TARGET_DIR="${1:-.}"
[ -d "$TARGET_DIR" ] || { echo "ERROR: not a directory: $TARGET_DIR" >&2; exit 1; }
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)"
for tool in file readelf ldd sort; do
    command -v "$tool" >/dev/null || { echo "ERROR: missing tool: $tool" >&2; exit 1; }
done

ERRORS=0
shopt -s nullglob dotglob
for FULL_PATH in "$TARGET_DIR"/*; do
    [ -f "$FULL_PATH" ] || continue
    DESCRIPTION="$(file -b "$FULL_PATH")"
    case "$DESCRIPTION" in *ELF*) ;; *) continue ;; esac
    echo "Checking: $FULL_PATH"
    HEADERS="$(readelf -lW "$FULL_PATH")"
    DYNAMIC="$(readelf -dW "$FULL_PATH")"
    VERSIONS="$(readelf -VW "$FULL_PATH")"
    INTERPRETER="$(printf '%s\n' "$HEADERS" | sed -n 's/.*Requesting program interpreter: \(.*\)\].*/\1/p')"
    if [ -n "$INTERPRETER" ] && [ ! -x "$INTERPRETER" ]; then
        echo "ERROR: missing ELF interpreter: $INTERPRETER" >&2
        ERRORS=$((ERRORS + 1))
    fi
    case "$DYNAMIC" in
        *'(NEEDED)'*)
            # Include sibling libraries, including dlopen backends checked separately.
            if DEPS="$(LD_LIBRARY_PATH="$TARGET_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ldd "$FULL_PATH" 2>&1)"; then
                case "$DEPS" in
                    *'not found'*) echo "ERROR: unresolved dependencies: $FULL_PATH" >&2; echo "$DEPS" >&2; ERRORS=$((ERRORS + 1)) ;;
                esac
            else
                echo "ERROR: ldd failed: $FULL_PATH" >&2
                echo "$DEPS" >&2
                ERRORS=$((ERRORS + 1))
            fi
            ;;
    esac
    REQUIRED_GLIBC="$(printf '%s\n' "$VERSIONS" | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -Vu | tail -n 1)"
    if [ -n "$REQUIRED_GLIBC" ]; then
        CURRENT_GLIBC="$(getconf GNU_LIBC_VERSION)"
        CURRENT_GLIBC="${CURRENT_GLIBC##* }"
        case "$CURRENT_GLIBC" in ''|*[!0-9.]*) echo "ERROR: cannot determine host glibc" >&2; exit 1 ;; esac
        OLDEST="$(printf '%s\n' "$CURRENT_GLIBC" "$REQUIRED_GLIBC" | sort -V | head -n 1)"
        if [ "$OLDEST" != "$REQUIRED_GLIBC" ]; then
            echo "ERROR: requires glibc $REQUIRED_GLIBC; host has $CURRENT_GLIBC" >&2
            ERRORS=$((ERRORS + 1))
        fi
    fi
done
if [ "$ERRORS" -ne 0 ]; then
    echo "Check failed: $ERRORS error(s)." >&2
    exit 1
fi
echo "Checks passed on this host with the package directory in LD_LIBRARY_PATH."

#!/usr/bin/env bash
# Fails if the Wii staticlib needs any symbol from outside itself that devkitPPC/newlib or our C
# platform layer does not provide. Catches accidental std use, new libm calls, atomics libcalls, etc.
#   tools/check_symbols.sh target/powerpc-unknown-linux-gnu/release/libfg_game.a
set -euo pipefail
lib=${1:?usage: check_symbols.sh <libfg_game.a>}
nm_bin=${NM:-powerpc-linux-gnu-nm}

# calloc: LLVM fuses malloc + zero-fill (vec![0; n]) into one calloc call; newlib provides it.
# fg_*: the platform layer (fg_platform.h), sound included: fg_audio_wanted and fg_audio_submit (fg_audio.c,
# the audio interface's DMA). The mixer itself is Rust (game_core::sound::mix) and needs no C maths (libm crate).
allowed='^(fg_[a-z0-9_]+|malloc|calloc|free|realloc|memalign|memcpy|memmove|memset|memcmp|bcmp|fmodf|fminf|fmaxf|_Unwind_Resume|rust_eh_personality)$'

defined=$(mktemp)
undefined=$(mktemp)
trap 'rm -f "$defined" "$undefined"' EXIT
"$nm_bin" "$lib" 2>/dev/null | awk '$1 ~ /^[0-9a-f]+$/ && $2 ~ /[TDRBWVtdrbwv]/ {print $3}' | sort -u >"$defined"
"$nm_bin" -u "$lib" 2>/dev/null | awk 'NF {print $NF}' | grep -v ':$' | sort -u >"$undefined"

external=$(comm -23 "$undefined" "$defined")
echo "External symbols:"
echo "$external" | sed 's/^/  /'
bad=$(echo "$external" | grep -Ev "$allowed" || true)
if [ -n "$bad" ]; then
    echo "Unexpected external symbols (add C support or avoid the Rust feature that pulls them in):" >&2
    echo "$bad" | sed 's/^/  /' >&2
    exit 1
fi
echo "OK"

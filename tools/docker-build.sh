#!/bin/sh
# Runs inside the devkitpro/devkitppc container (build.bat and build.sh use it when devkitPro isn't
# installed). The image has no Rust, so rustup goes into .toolchain/ in the repo (git-ignored, kept
# between runs). Usage: tools/docker-build.sh [make target]
# HOST_UID / HOST_GID (set by build.sh): give the output files back to that user afterwards.
set -e
cd "$(dirname "$0")/.."
export RUSTUP_HOME="$PWD/.toolchain/rustup"
export CARGO_HOME="$PWD/.toolchain/cargo"
export PATH="$CARGO_HOME/bin:$PATH"

if [ "${1:-}" != "clean" ]; then
	# Rust build scripts (libm has one) are linked with the host C compiler.
	if ! command -v cc >/dev/null 2>&1; then
		echo "[build] Installing a host C compiler into the container (each run)..."
		apt-get update -qq && apt-get install -y -qq --no-install-recommends gcc libc6-dev >/dev/null
	fi
	if ! command -v rustup >/dev/null 2>&1; then
		echo "[rust] Installing Rust into .toolchain/ (first run only)..."
		# Download first: piping curl into sh would hide a failed download.
		mkdir -p .toolchain
		if ! curl --proto '=https' --tlsv1.2 -sSf -o .toolchain/rustup-init.sh https://sh.rustup.rs; then
			echo "[rust] Couldn't download the Rust installer (no internet?)." >&2
			exit 1
		fi
		sh .toolchain/rustup-init.sh -y --profile minimal --default-toolchain none --no-modify-path
		rm -f .toolchain/rustup-init.sh
	fi
fi
git config --global --add safe.directory "$PWD" 2>/dev/null || true

status=0
make -C wii "$@" || status=$?

if [ -n "${HOST_UID:-}" ]; then
	for d in target dist wii/build .toolchain; do
		[ -e "$d" ] && chown -R "$HOST_UID:${HOST_GID:-$HOST_UID}" "$d"
	done
fi
exit $status

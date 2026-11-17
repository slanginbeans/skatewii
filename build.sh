#!/bin/sh
# Linux/macOS: builds the Wii game inside the official devkitPro Docker image, so you don't need
# devkitPro or Rust installed. Output: dist/apps/flatground (copy it into the SD card's apps/).
# Usage: ./build.sh [make target]   (default: package; also: clean)
# If you have devkitPro and Rust installed, `make -C wii package` does the same without Docker.
set -e
cd "$(dirname "$0")"
[ $# -eq 0 ] && set -- package
exec docker run --rm -v "$PWD":/src -w /src -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
	devkitpro/devkitppc sh tools/docker-build.sh "$@"

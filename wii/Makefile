# Builds flatground.dol and the Homebrew Channel folder.
#   Needs devkitPro (devkitPPC + libogc, the "wii-dev" group) and the Rust toolchain from rust-toolchain.toml.
#   make            -> build/flatground.dol
#   make package    -> ../dist/apps/flatground/{boot.dol,meta.xml}: copy that folder to SD:/apps/
#                      plus data/ with the packs converted from your own Skate 3 disc image, when one is
#                      found (fg_import build: converts only what is missing or out of date; FG_NO_ASSETS=1 skips)
#   make package RUST_PROFILE=dolphin -> the same, built without LTO (quicker; for testing)
#   make run WIILOAD=tcp:<wii-ip>   -> send to a Wii running the Homebrew Channel

ifeq ($(strip $(DEVKITPRO)),)
$(error DEVKITPRO is not set. Install devkitPro (wii-dev) or use the devkitpro/devkitppc container)
endif

DEVKITPPC ?= $(DEVKITPRO)/devkitPPC
CC        := $(DEVKITPPC)/bin/powerpc-eabi-gcc
ELF2DOL   := $(DEVKITPRO)/tools/bin/elf2dol
WIILOAD_BIN := $(DEVKITPRO)/tools/bin/wiiload
OGC       := $(DEVKITPRO)/libogc

MACHDEP := -DGEKKO -mrvl -mcpu=750 -meabi -mhard-float
CFLAGS  := -O2 -g -Wall -Wextra -ffunction-sections -fdata-sections $(MACHDEP) -I$(OGC)/include -Iplatform
LDFLAGS := -g $(MACHDEP) -L$(OGC)/lib/wii -Wl,--gc-sections
LIBS    := -lwiiuse -lbte -lfat -logc -lm

RUST_TARGET := powerpc-unknown-linux-gnu
# Cargo profile for the game and the converter: release (default), or dolphin for quicker test builds
# (no LTO; `build.bat fast`).
RUST_PROFILE ?= release
RUSTLIB     := ../target/$(RUST_TARGET)/$(RUST_PROFILE)/libfg_game.a
BUILD_ID    ?= $(shell git rev-parse --short=8 HEAD 2>/dev/null || echo dev)

BUILD := build
OBJS  := $(BUILD)/main.o $(BUILD)/fg_platform.o $(BUILD)/fg_log.o

.PHONY: all package assets run clean rustlib
all: $(BUILD)/flatground.dol

# Cargo tracks its own dependencies; always ask it.
rustlib:
	FG_BUILD_ID=$(BUILD_ID) cargo build --profile $(RUST_PROFILE) -p fg_game --target $(RUST_TARGET)
# The empty recipe makes make look at the library's time again after cargo ran (without one it keeps
# the old time and doesn't relink).
$(RUSTLIB): rustlib
	@:

$(BUILD):
	mkdir -p $@

$(BUILD)/%.o: platform/%.c platform/fg_platform.h platform/fg_log.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/flatground.elf: $(OBJS) $(RUSTLIB)
	$(CC) $(LDFLAGS) $(OBJS) $(RUSTLIB) $(LIBS) -Wl,-Map,$@.map -o $@

$(BUILD)/flatground.dol: $(BUILD)/flatground.elf
	$(ELF2DOL) $< $@

DIST := ../dist
REPO := $(abspath ..)
# Where fg_import looks for a disc image (and fg_local.txt): this repo and the folder holding it (not /).
ISO_SEARCH := $(REPO) $(filter-out /,$(abspath $(REPO)/..))

# Converted packs are cached in imported/build (survives make clean) and copied into the app folder.
assets:
ifeq ($(FG_NO_ASSETS),1)
	@echo "[assets] FG_NO_ASSETS=1: not converting."
else
	cargo run --profile $(RUST_PROFILE) -q -p fg_import -- build $(addprefix --search ,$(ISO_SEARCH)) --cache $(REPO)/imported/build --dest $(DIST)/apps/flatground/data
endif

package: all
	mkdir -p $(DIST)/apps/flatground $(DIST)/debug
	cp $(BUILD)/flatground.dol $(DIST)/apps/flatground/boot.dol
	sed "s/@BUILD_ID@/$(BUILD_ID)/" hbc/meta.xml > $(DIST)/apps/flatground/meta.xml
	cp $(BUILD)/flatground.elf $(DIST)/debug/flatground.elf
	$(MAKE) --no-print-directory assets
	@echo "Copy dist/apps/flatground to the SD card's apps/ folder."

run: $(BUILD)/flatground.dol
	$(WIILOAD_BIN) $<

clean:
	rm -rf $(BUILD) $(DIST)

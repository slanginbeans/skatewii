# Builds flatground.dol and the Homebrew Channel folder.
#   Needs devkitPro (devkitPPC + libogc, the "wii-dev" group) and the Rust toolchain from rust-toolchain.toml.
#   make            -> build/flatground.dol
#   make package    -> ../dist/apps/flatground/{boot.dol,meta.xml}: copy that folder to SD:/apps/
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
RUSTLIB     := ../target/$(RUST_TARGET)/release/libfg_game.a
BUILD_ID    ?= $(shell git rev-parse --short=8 HEAD 2>/dev/null || echo dev)

BUILD := build
OBJS  := $(BUILD)/main.o $(BUILD)/fg_platform.o

.PHONY: all package run clean rustlib
all: $(BUILD)/flatground.dol

# Cargo tracks its own dependencies; always ask it.
rustlib:
	cargo build --release -p fg_game --target $(RUST_TARGET)
$(RUSTLIB): rustlib

$(BUILD):
	mkdir -p $@

$(BUILD)/%.o: platform/%.c platform/fg_platform.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/flatground.elf: $(OBJS) $(RUSTLIB)
	$(CC) $(LDFLAGS) $(OBJS) $(RUSTLIB) $(LIBS) -Wl,-Map,$@.map -o $@

$(BUILD)/flatground.dol: $(BUILD)/flatground.elf
	$(ELF2DOL) $< $@

DIST := ../dist
package: all
	mkdir -p $(DIST)/apps/flatground $(DIST)/debug
	cp $(BUILD)/flatground.dol $(DIST)/apps/flatground/boot.dol
	sed "s/@BUILD_ID@/$(BUILD_ID)/" hbc/meta.xml > $(DIST)/apps/flatground/meta.xml
	cp $(BUILD)/flatground.elf $(DIST)/debug/flatground.elf
	@echo "Copy dist/apps/flatground to the SD card's apps/ folder."

run: $(BUILD)/flatground.dol
	$(WIILOAD_BIN) $<

clean:
	rm -rf $(BUILD) $(DIST)

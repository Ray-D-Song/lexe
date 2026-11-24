VENDOR_SQLITE := ../../modules/lexe_sqlite/vendor
BUILD_DIR := build
TARGET := $(BUILD_DIR)/sqlite_extension

ifeq ($(OS),Windows_NT)
  EXT := .dll
  CC ?= cc
  CFLAGS := -shared -I$(VENDOR_SQLITE) -o
else
  UNAME_S := $(shell uname -s)
  ifeq ($(UNAME_S),Darwin)
    EXT := .dylib
    CFLAGS := -dynamiclib -fPIC -I$(VENDOR_SQLITE) -o
  else
    EXT := .so
    CFLAGS := -shared -fPIC -I$(VENDOR_SQLITE) -o
  endif
  CC ?= cc
endif

.PHONY: all clean

all: $(TARGET)$(EXT)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(TARGET)$(EXT): extension.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) $@ $<

clean:
	rm -rf $(BUILD_DIR)

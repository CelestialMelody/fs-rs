TARGET_NAME := easy-fs
POJECT_NAME := fs-rs
MODE := release
TARGET_DIR := $(PWD)
DEFAULT_TARGET := $(TARGET_DIR)/target/$(MODE)/$(TARGET_NAME)

build:
	cargo build --$(MODE)
	cp $(DEFAULT_TARGET) $(TARGET_DIR)

clean:
	cargo clean
	rm $(TARGET_DIR)/$(TARGET_NAME)

.PHONY: build clean
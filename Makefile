TARGET_NAME := easy-fs
POJECT_NAME := fs-rs
MODE := release
TARGET_DIR := $(PWD)
DEFAULT_TARGET := $(TARGET_DIR)/target/$(MODE)/$(TARGET_NAME)

build:
	cargo build --$(MODE)
# 如果没有 test 文件夹 则创建
	if [ ! -d "test" ]; then mkdir test; fi
	cp $(DEFAULT_TARGET) $(TARGET_DIR)

create: build
	./$(TARGET_NAME) -s src/fs/ -t test/ -w create

open: build
	./$(TARGET_NAME) -s src/fs/ -t test/ -w open

clean:
	cargo clean
# 如果有 test 文件夹 则删除
	if [ -d "test" ]; then rm -rf test; fi
	rm $(TARGET_DIR)/$(TARGET_NAME)

.PHONY: build clean
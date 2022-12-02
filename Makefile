TARGET_NAME := easy-fs
POJECT_NAME := fs-rs
MODE := debug
# MODE := release # 此时没法调试，release 会丢弃调试信息
TARGET_DIR := $(PWD)
DEFAULT_TARGET := $(TARGET_DIR)/target/$(MODE)/$(TARGET_NAME)

build:
# 如果是 mode == release
	if [ $(MODE) = "release" ]; then \
		cargo build --release; \
	else \
		cargo build; \
	fi
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
# 如果有 $(TARGET_NAME) 则删除
	if [ -f "$(TARGET_NAME)" ]; then rm $(TARGET_NAME); fi

.PHONY: build clean
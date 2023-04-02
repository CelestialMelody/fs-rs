TARGET_NAME := easy-fs
POJECT_NAME := fs-rs
MODE := debug
# MODE := release # release 会丢弃调试信息
ROOT_DIR := $(PWD)
DEFAULT_TARGET := $(ROOT_DIR)/target/$(MODE)/$(TARGET_NAME)

build:
	if [ $(MODE) = "release" ]; then \
		cargo build --release; \
	else \
		cargo build; \
	fi
	if [ ! -d "test" ]; then mkdir test; fi

create: build
	$(DEFAULT_TARGET) -s src/fs/ -t test/ -w create

open: build
	$(DEFAULT_TARGET) -s src/fs/ -t test/ -w open

debug: build
	gdb $(DEFAULT_TARGET)

clean:
	cargo clean
	if [ -d "test" ]; then rm -rf test; fi
	if [ -f "$(TARGET_NAME)" ]; then rm $(TARGET_NAME); fi

.PHONY: build clean

Files = $(shell find ./src -type f)
fmt:
	@for file in $(Files) ; do \
  		if [ -f "$$file" ]; then \
   		 	sed -i \
			-e 's/，/, /g;' \
			-e 's/。/./g;' \
			-e 's/：/: /g;' \
			-e 's/？/?/g;' \
			-e 's/！/!/g;' \
			-e 's/（/(/g;' \
			-e 's/）/)/g;' \
			-e 's/；/; /g;' \
			-e 's/“/"/g;' \
			-e 's/”/"/g;' \
			-e 's/‘/'"'"'/g;' \
			-e 's/’/'"'"'/g;' \
			-e 's/《/</g;' \
			-e 's/》/>/g;' \
			-e 's/【/[/g;' \
			-e 's/】/]/g;' \
			-e 's/、/, /g;' \
			-e 's/　/ /g;' \
			-e 's/…/.../g;' \
			-e 's/—/-/g;' \
			-e 's/——/-/g;' \
			"$$file" ; \
  		else \
			echo "$$file not found \n" ; \
  		fi \
	done
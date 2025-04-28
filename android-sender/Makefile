RTARGET = x86_64-linux-android
OUT = app/src/main/jniLibs
MAIN_UI = ui/main.slint

.PHONY: default
default: check

.PHONY: check
check:
	cargo ndk --target $(RTARGET) check

.PHONY: build
build:
	cargo ndk --target $(RTARGET) -o $(OUT) build
	./fix.sh

.PHONY: build-release
build-release:
	cargo ndk --target $(RTARGET) -o $(OUT) build -r
	./fix.sh

.PHONY: preview
preview:
	slint-viewer --style material $(MAIN_UI)

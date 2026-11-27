TARGET_linux_x86_64 = x86_64-unknown-linux-musl
TARGET_linux_arm64 = aarch64-unknown-linux-musl
TARGET_darwin_x86_64 = x86_64-apple-darwin
TARGET_darwin_arm64 = aarch64-apple-darwin
TARGET_windows_x86_64 = x86_64-pc-windows-msvc
TARGET_windows_arm64 = aarch64-pc-windows-msvc
RUST_VERSION = 1.100.0
TOOLCHAIN = +$(RUST_VERSION)
BUILD_ARG = $(TOOLCHAIN) build -r
BUILD_DIR = ./target/release
BUNDLE_DIR = bundle
# Ignore CARGO_TARGET_DIR so metadata and builds agree on the artifact path (sandbox may redirect it).
# LEXE_BIN requires jq; without jq the fallback path is wrong — install jq for compat-napi.
CARGO ?= env -u CARGO_TARGET_DIR cargo
LEXE_BIN ?= $(shell $(CARGO) metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.target_directory')/debug/lexe

TS_SOURCES = $(wildcard lexe_core/src/modules/js/*.ts) $(wildcard lexe_core/src/modules/js/@lexe/test/*.ts) $(wildcard lexe_core/src/modules/js/@lexe/*.ts) $(wildcard tests/unit/*.ts)
STD_JS_FILE = $(BUNDLE_DIR)/js/@lexe/test/index.js

RELEASE_ARCH_NAME_x64 = x86_64
RELEASE_ARCH_NAME_arm64 = arm64

RELEASE_TARGETS = arm64 x64

ifeq ($(OS),Windows_NT)
	DETECTED_OS := windows
	ARCH = x86_64
else
	DETECTED_OS := $(shell uname | tr A-Z a-z)
	ARCH = $(shell uname -m)
endif

ifeq ($(ARCH),aarch64)
	ARCH = arm64
endif

ZSTD_LIB_CC_ARGS = -s -O3 -flto
ZSTD_LIB_ARGS = -j lib-nomt UNAME=Linux ZSTD_LIB_COMPRESSION=0 ZSTD_LIB_DICTBUILDER=0 AR="zig ar"
ifeq ($(DETECTED_OS),windows)
ZSTD_LIB_CC_x64 = CC="zig cc -target x86_64-windows-gnu $(ZSTD_LIB_CC_ARGS)"
else
ZSTD_LIB_CC_arm64 = CC="zig cc -target aarch64-linux-musl $(ZSTD_LIB_CC_ARGS)"
ZSTD_LIB_CC_x64 = CC="zig cc -target x86_64-linux-musl $(ZSTD_LIB_CC_ARGS)"
endif

CURRENT_TARGET ?= $(TARGET_$(DETECTED_OS)_$(ARCH))

export CC_aarch64_unknown_linux_musl = $(CURDIR)/linker/cc-aarch64-linux-musl
export CXX_aarch64_unknown_linux_musl = $(CURDIR)/linker/cxx-aarch64-linux-musl
export AR_aarch64_unknown_linux_musl = $(CURDIR)/linker/ar
export CC_x86_64_unknown_linux_musl = $(CURDIR)/linker/cc-x86_64-linux-musl
export CXX_x86_64_unknown_linux_musl = $(CURDIR)/linker/cxx-x86_64-linux-musl
export AR_x86_64_unknown_linux_musl = $(CURDIR)/linker/ar

define alias_template
release${1}: lexe-$(DETECTED_OS)-$(ARCH)${1}.zip

lexe-linux-x86_64${1}.zip: lexe-linux-x64${1}.zip
lexe-windows-x86_64${1}.zip: lexe-windows-x64${1}.zip
lexe-darwin-x86_64${1}.zip: lexe-darwin-x64${1}.zip
endef

$(eval $(call alias_template,))

define release_template
release-${1}: | lexe-linux-${1}.zip

lexe-container-${1}: | clean-js js
	cargo $$(BUILD_ARG) --target $$(TARGET_linux_$$(RELEASE_ARCH_NAME_${1})) --features uncompressed
	mv target/$$(TARGET_linux_$$(RELEASE_ARCH_NAME_${1}))/release/lexe $$@

lexe-linux-${1}.zip: | clean-js js
	cargo $$(BUILD_ARG) --target $$(TARGET_linux_$$(RELEASE_ARCH_NAME_${1}))
	@rm -rf $$@
	zip -j $$@ target/$$(TARGET_linux_$$(RELEASE_ARCH_NAME_${1}))/release/lexe

lexe-darwin-${1}.zip: | clean-js js
	cargo $$(BUILD_ARG) --target $$(TARGET_darwin_$$(RELEASE_ARCH_NAME_${1}))
	@rm -rf $$@
	zip -j $$@ target/$$(TARGET_darwin_$$(RELEASE_ARCH_NAME_${1}))/release/lexe

lexe-windows-${1}.zip: | clean-js js
	cargo $$(BUILD_ARG) --target $$(TARGET_windows_$$(RELEASE_ARCH_NAME_${1}))
	zip -j $$@ target/$$(TARGET_windows_$$(RELEASE_ARCH_NAME_${1}))/release/lexe.exe
endef

$(foreach target,$(RELEASE_TARGETS),$(eval $(call release_template,$(target))))

build: js
	cargo $(BUILD_ARG) --target $(CURRENT_TARGET)

ifeq ($(DETECTED_OS),windows)
stdlib:
	rustup toolchain install $(RUST_VERSION) --target $(TARGET_windows_x86_64)
	rustup target add $(TARGET_windows_x86_64)
	rustup component add rust-src --toolchain $(RUST_VERSION) --target $(TARGET_windows_x86_64)
else
stdlib-x64:
	rustup toolchain install $(RUST_VERSION) --target $(TARGET_linux_x86_64)
	rustup target add $(TARGET_linux_x86_64)
	rustup component add rust-src --toolchain $(RUST_VERSION) --target $(TARGET_linux_x86_64)

stdlib-arm64:
	rustup toolchain install $(RUST_VERSION) --target $(TARGET_linux_arm64)
	rustup target add $(TARGET_linux_arm64)
	rustup component add rust-src --toolchain $(RUST_VERSION) --target $(TARGET_linux_arm64)

stdlib: | stdlib-x64 stdlib-arm64
endif

toolchain:
	rustup toolchain install $(RUST_VERSION) --target $(CURRENT_TARGET)
	rustup target add $(CURRENT_TARGET)
	rustup component add rust-src --toolchain $(RUST_VERSION) --target $(CURRENT_TARGET)

clean-js:
	rm -rf ./bundle

clean: clean-js
	rm -rf ./target
	rm -rf ./lib

js: $(STD_JS_FILE)

update-dict: clean-js js
	LEXE_UPDATE_DICT=1 $(CARGO) check -p lexe_core --features base

bundle/js/%.js: $(TS_SOURCES)
	node build.mjs

fix:
	npx pretty-quick
	cargo fix --allow-dirty
	cargo clippy --fix --allow-dirty
	cargo fmt

bloat: js
	cargo build --profile=flame --target $(CURRENT_TARGET)
	cargo bloat --profile=flame --crates

run: export _EXIT_ITERATIONS = 1
run: export JS_MINIFY = 0
run: export RUST_LOG = lexe=trace
run:
	cargo run -vv

run-ssr: js
	cargo build
	cd example/functions && yarn build && cd build && ../../../target/debug/lexe

compat: compat-api-surface compat-next compat-vite-plus compat-better-sqlite3 compat-mysql compat-node-postgres compat-napi compat-v8 compat-node-sqlite

compat-api-surface: js
	$(CARGO) build -p lexe
	LEXE_BIN=$(LEXE_BIN) node compat/run.mjs api-surface $(LEXE_BIN)
	node scripts/check-compat-matrix.mjs

compat-v8-hello: compat-v8

compat-napi-hello: compat-napi

compat-v8: js
	$(CARGO) build -p lexe --features v8-compat
	cd compat/v8-hello && npm run build
	LEXE_BIN=$(LEXE_BIN) node compat/run.mjs v8-hello $(LEXE_BIN)

compat-next: js
	cd compat/next && yarn install --frozen-lockfile
	node compat/run.mjs next $(LEXE_BIN)

compat-vite-plus: js
	$(CARGO) build -p lexe --features napi
	# Install baseline must satisfy vite-plus engines (^20.19 || ^22.18 || >=24.11).
	# Lexe process identity is 24.3.0; do not use system Node 24.3 for yarn install.
	cd compat/vite-plus && yarn install --frozen-lockfile
	node compat/run.mjs vite-plus $(LEXE_BIN)

compat-better-sqlite3: js
	$(CARGO) build -p lexe --features v8-compat
	cd compat/better-sqlite3 && yarn install --frozen-lockfile
	LEXE_BIN=$(LEXE_BIN) node compat/run.mjs better-sqlite3 $(LEXE_BIN)

compat-mysql: js
	cd compat/mysql2 && yarn install --frozen-lockfile
	node compat/run.mjs mysql2 $(LEXE_BIN)

compat-node-postgres: js
	cd compat/node-postgres && yarn install --frozen-lockfile
	node compat/run.mjs node-postgres $(LEXE_BIN)

check-v8-abi:
	bash v8_compat/tools/check_abi.sh

sanitizer-ci:
	bash v8_compat/tools/run_sanitizer_ci.sh address
	bash v8_compat/tools/run_sanitizer_ci.sh undefined

compat-napi: js
	$(CARGO) build -p lexe --features napi
	cd compat/napi-hello && yarn install
	LEXE_BIN=$(LEXE_BIN) node compat/run.mjs napi-hello $(LEXE_BIN)

compat-node-sqlite: js
	$(CARGO) build -p lexe
	LEXE_BIN=$(LEXE_BIN) node compat/run.mjs node-sqlite $(LEXE_BIN)

compat-node-sqlite-asan:
	bash compat/node-sqlite/run-asan.sh

flame:
	cargo flamegraph --profile flame -- index.mjs

run-cli: export RUST_LOG = lexe=trace
run-cli: js
	cargo run

test: export JS_MINIFY = 0
test: export TEST_SUB_DIR = unit
test: export LEXE_ASYNC_HOOKS = 1
test: js
	cargo run -- test -d bundle/js/__tests__/$(TEST_SUB_DIR)

types-pack-check:
	cd types && yarn pack:check

types-smoke:
	cd types && yarn smoke

npm-pack-check:
	cd npm && node ./scripts/pack-check.mjs

npm-test:
	cd npm && node --test ./test/npm-cli.test.mjs

init-wpt:
	cd wpt && \
	git sparse-checkout init --no-cone && \
	git sparse-checkout set \
		/README.md \
		/console \
		/docs \
		/encoding \
		/FileAPI \
		/fetch \
		/hr-time \
		/streams \
		/tools \
		/url \
		/WebCryptoAPI \
		/webidl \
		/wpt \
		/xhr

update-wpt:
	( cd wpt && git fetch origin master && git reset --hard FETCH_HEAD && git log -1 --oneline > ../tests/wpt/revision )

test-wpt: export JS_MINIFY = 0
test-wpt: export TEST_SUB_DIR = wpt
test-wpt: js
	npx pretty-quick --pattern "tests/wpt/**/*.{js,ts,json}"
	cargo run -- test -d bundle/js/__tests__/$(TEST_SUB_DIR) 2> wpt_errors.tmp

tidyup-wpt:
	sed -E 's/\x1b\[[0-9;]*m//g' wpt_errors.tmp \
	| sed '1,/^$$/d' \
	| sed -E '/^ ?[^ ]/s|^.*__tests__/|🧪/|' \
	> wpt_errors.txt

test-ci: export JS_MINIFY = 0
test-ci: export RUST_BACKTRACE = 1
test-ci: export TEST_SUB_DIR = unit
test-ci: export LEXE_ASYNC_HOOKS = 1
test-ci: clean-js | toolchain js types-pack-check types-smoke
ifdef CARGO_FEATURES
	cargo $(TOOLCHAIN) test --target $(CURRENT_TARGET) $(CARGO_FEATURES) -- --nocapture --show-output
	cargo $(TOOLCHAIN) run -r --target $(CURRENT_TARGET) $(CARGO_FEATURES) -- test -d bundle/js/__tests__/$(TEST_SUB_DIR)
else
	cargo $(TOOLCHAIN) test --target $(CURRENT_TARGET) -- --nocapture --show-output
	cargo $(TOOLCHAIN) run -r --target $(CURRENT_TARGET) -- test -d bundle/js/__tests__/$(TEST_SUB_DIR)
endif

libs-arm64: lib/arm64/libzstd.a lib/zstd.h lib/zstd_errors.h
libs-x64: lib/x64/libzstd.a lib/zstd.h lib/zstd_errors.h

libs: | libs-arm64 libs-x64

lib/zstd.h:
	cp zstd/lib/zstd.h $@

lib/zstd_errors.h:
	cp zstd/lib/zstd_errors.h $@

lib/arm64/libzstd.a:
	mkdir -p $(dir $@)
	rm -f zstd/lib/-.o
	cd zstd/lib && make clean && make $(ZSTD_LIB_ARGS) $(ZSTD_LIB_CC_arm64)
	cp zstd/lib/libzstd.a $@

lib/x64/libzstd.a:
	mkdir -p $(dir $@)
	rm -f zstd/lib/-.o
	cd zstd/lib && make clean && make $(ZSTD_LIB_ARGS) $(ZSTD_LIB_CC_x64)
	cp zstd/lib/libzstd.a $@

bench:
	cargo build -r
	hyperfine -N --warmup=100 "node fixtures/hello.js" "deno run fixtures/hello.js" "bun fixtures/hello.js" "$(BUILD_DIR)/lexe fixtures/hello.js" "qjs fixtures/hello.js"

deploy:
	cd example/infrastructure && yarn deploy --require-approval never

check:
	cargo clippy --all-targets --no-default-features --features "macro,uncompressed,crypto-rust,tls-ring" -- -D warnings

test-rs:
	cargo test --all-targets --no-default-features --features "macro,uncompressed,crypto-rust,tls-ring"

check-crates:
	cargo metadata --no-deps --format-version 1 --quiet | \
	jq -r '.packages[] | select(.manifest_path | contains("modules/")) | .name' | \
	while read crate; do \
	  echo "Checking crate: $$crate"; \
	  cargo check -p "$$crate"; \
	done

WASI_SDK ?= /opt/wasi-sdk
WASM_TARGET = wasm32-wasip1

wasm: js
	rustup target add $(WASM_TARGET)
	CC_wasm32_wasip1=$(WASI_SDK)/bin/clang \
	AR_wasm32_wasip1=$(WASI_SDK)/bin/llvm-ar \
	CFLAGS_wasm32_wasip1=--sysroot=$(WASI_SDK)/share/wasi-sysroot \
	WASI_SDK=$(WASI_SDK) \
	$(CARGO) build -p lexe --target $(WASM_TARGET) --no-default-features --features wasi

.PHONY: libs check check-all check-crates libs-arm64 libs-x64 toolchain clean-js release-linux release-darwin release-windows stdlib stdlib-x64 stdlib-arm64 test test-ci run js update-dict run-release build release clean flame deploy compat compat-next compat-vite-plus compat-better-sqlite3 compat-mysql compat-node-postgres compat-napi compat-v8 compat-v8-hello compat-napi-hello compat-node-sqlite compat-node-sqlite-asan check-v8-abi types-pack-check types-smoke npm-pack-check npm-test wasm

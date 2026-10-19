RNET_NIGHTLY ?= nightly-2026-09-21
DOTNET ?= dotnet
FUZZ_SECONDS ?= 10
FUZZ_RSS_MB ?= 2048
FUZZ_TARGET_DIR ?= $(CURDIR)/target/fuzz

.PHONY: abi-check build check cpp-test csharp-test doc-check fuzz-asan-smoke go-test loom-test miri-test package structure-check test

build:
	cargo build -p rnet-ffi --release

test:
	cargo test --workspace

doc-check:
	cargo rustdoc -p rnet-core --lib -- -D warnings
	cargo rustdoc -p rnet-protocol --lib -- -D warnings
	cargo rustdoc -p rnet-security --lib -- -D warnings
	cargo rustdoc -p rnet-observe --lib -- -D warnings
	cargo rustdoc -p rnet-transport --lib --all-features -- -D warnings
	cargo rustdoc -p rnet-game --lib --all-features -- -D warnings
	cargo test -p rnet-core -p rnet-protocol -p rnet-security -p rnet-game -p rnet-observe -p rnet-transport --doc

cpp-test:
	mkdir -p target
	g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp cpp/tests/poll_ownership.cpp -o target/rnet-cpp-poll-ownership
	./target/rnet-cpp-poll-ownership
	cargo build -p rnet-ffi
	g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/echo_smoke.cpp target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-cpp-echo
	./target/rnet-cpp-echo
	g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/secure_echo_smoke.cpp target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-cpp-secure-echo
	./target/rnet-cpp-secure-echo
	g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/game_echo_smoke.cpp target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-cpp-game-echo
	./target/rnet-cpp-game-echo
	g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/game_range_smoke.cpp target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-cpp-game-range
	./target/rnet-cpp-game-range

go-test:
	cargo build -p rnet-ffi
	mkdir -p lib
	cp target/debug/librnet.a lib/librnet.a
	# Go's cache does not track changes to the externally linked native archive.
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 ./go/rnet
	# Test-only Linux linker interception checks partial exports against real native buffers.
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 -tags rnet_test_faults -run TestGamePollFailureReleasesOnlyExportedPrefix ./go/rnet

csharp-test:
	cargo build -p rnet-ffi
	LD_LIBRARY_PATH="$(CURDIR)/target/debug:$${LD_LIBRARY_PATH:-}" $(DOTNET) run --project csharp/RNet.Smoke -c Release

loom-test:
	cargo test -p rnet-game --features loom admission::tests -- --test-threads=1

fuzz-asan-smoke:
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" frame_parser -- -max_total_time=$(FUZZ_SECONDS) -max_len=1048576 -rss_limit_mb=$(FUZZ_RSS_MB)
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" control_parser -- -max_total_time=$(FUZZ_SECONDS) -max_len=131072 -rss_limit_mb=$(FUZZ_RSS_MB)
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" datagram_preflight -- -max_total_time=$(FUZZ_SECONDS) -max_len=2048 -rss_limit_mb=$(FUZZ_RSS_MB)
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" kcp_engine -- -max_total_time=$(FUZZ_SECONDS) -max_len=65535 -rss_limit_mb=$(FUZZ_RSS_MB)
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" game_wire -- -max_total_time=$(FUZZ_SECONDS) -max_len=65535 -rss_limit_mb=$(FUZZ_RSS_MB)
	cargo +$(RNET_NIGHTLY) fuzz run -s address --target-dir "$(FUZZ_TARGET_DIR)" ffi_config -- -max_total_time=$(FUZZ_SECONDS) -max_len=64 -rss_limit_mb=$(FUZZ_RSS_MB)

miri-test:
	cargo +$(RNET_NIGHTLY) miri test -p rnet-core -p rnet-protocol

check:
	$(MAKE) structure-check
	cargo fmt --check
	$(MAKE) doc-check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	$(MAKE) loom-test
	$(MAKE) abi-check
	$(MAKE) cpp-test
	$(MAKE) go-test
	$(MAKE) csharp-test

structure-check:
	./scripts/check-structure.sh

abi-check:
	cargo build -p rnet-ffi --release
	./scripts/check-exports.sh

package:
	./scripts/package.sh

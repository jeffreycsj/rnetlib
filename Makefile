RNET_NIGHTLY ?= nightly-2026-09-21
DOTNET ?= dotnet
PYTHON ?= python3
CLANG ?= clang
CLANGXX ?= clang++
FUZZ_SECONDS ?= 10
FUZZ_RSS_MB ?= 2048
FUZZ_TARGET_DIR ?= $(CURDIR)/target/fuzz
RNET_MSRV ?= 1.85.0

.PHONY: abi-check build check c-test cpp-test csharp-test doc-check native-doc-check fuzz-asan-smoke go-test header-check header-update loom-test miri-test package structure-check test
.PHONY: dependency-check go-race msrv-check package-smoke

build: header-check
	cargo build -p rnet-ffi --release

header-update:
	$(PYTHON) -B scripts/generated_header.py --update

header-check:
	$(PYTHON) -B scripts/generated_header.py --check

test:
	cargo test --workspace

native-doc-check: header-check
	CLANG="$(CLANG)" CLANGXX="$(CLANGXX)" $(PYTHON) -B -m unittest discover -s scripts/tests -v
	CLANG="$(CLANG)" CLANGXX="$(CLANGXX)" $(PYTHON) -B scripts/check_native_docs.py

doc-check: native-doc-check
	cargo rustdoc -p rnet-core --lib -- -D warnings
	cargo rustdoc -p rnet-protocol --lib -- -D warnings
	cargo rustdoc -p rnet-security --lib -- -D warnings
	cargo rustdoc -p rnet-observe --lib -- -D warnings
	cargo rustdoc -p rnet-transport --lib --all-features -- -D warnings
	cargo rustdoc -p rnet-game --lib --all-features -- -D warnings
	cargo test -p rnet-core -p rnet-protocol -p rnet-security -p rnet-game -p rnet-observe -p rnet-transport --doc

c-test: header-check
	cargo build -p rnet-ffi
	$(CC) -std=c11 -Wall -Wextra -Werror -Iinclude examples/c/tests/poll_ownership.c target/debug/librnet.a -Wl,--wrap=rnet_game_poll_events -lpthread -ldl -lm -o target/rnet-c-poll-ownership
	./target/rnet-c-poll-ownership poll-error
	./target/rnet-c-poll-ownership handler-error
	./target/rnet-c-poll-ownership handler-error-last
	./target/rnet-c-poll-ownership empty-error
	./target/rnet-c-poll-ownership discard
	./target/rnet-c-poll-ownership success
	$(CC) -std=c11 -Wall -Wextra -Werror -Iinclude examples/c/game_echo.c target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-c-game-echo
	$(CC) -std=c11 -Wall -Wextra -Werror -Iinclude examples/c/game_echo.c examples/c/tests/send_backpressure.c target/debug/librnet.a -Wl,--wrap=rnet_game_send -lpthread -ldl -lm -o target/rnet-c-game-backpressure
	$(PYTHON) -B scripts/test_c_game_example.py -v

cpp-test: header-check
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
	$(CXX) -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp cpp/tests/game_quickstart.cpp -o target/rnet-cpp-quickstart-contracts
	./target/rnet-cpp-quickstart-contracts
	$(CXX) -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/game_quickstart.cpp target/debug/librnet.a -lpthread -ldl -lm -o target/rnet-cpp-game-quickstart
	$(PYTHON) -B scripts/test_cpp_game_example.py -v

go-test: header-check
	cargo build -p rnet-ffi
	mkdir -p lib
	cp target/debug/librnet.a lib/librnet.a
	# Go's cache does not track changes to the externally linked native archive.
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 ./go/rnet ./examples/go/game_quickstart
	GOTOOLCHAIN=local CGO_ENABLED=1 go build -a -p 1 -o target/rnet-go-game-quickstart ./examples/go/game_quickstart
	# Linux test-only helpers cover native ownership, close retries and clock conversion extremes.
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 -tags rnet_test_faults ./go/rnet

csharp-test: header-check
	cargo build -p rnet-ffi
	LD_LIBRARY_PATH="$(CURDIR)/target/debug:$${LD_LIBRARY_PATH:-}" $(DOTNET) run --project csharp/RNet.Smoke -c Release
	$(DOTNET) run --project csharp/RNet.Quickstart.Tests -c Release
	$(DOTNET) build csharp/RNet.Quickstart -c Release
	DOTNET="$(DOTNET)" LD_LIBRARY_PATH="$(CURDIR)/target/debug:$${LD_LIBRARY_PATH:-}" $(PYTHON) -B scripts/test_csharp_game_example.py -v

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
	$(MAKE) c-test
	$(MAKE) cpp-test
	$(MAKE) go-test
	$(MAKE) csharp-test

structure-check:
	./scripts/check-structure.sh

abi-check: header-check
	cargo build -p rnet-ffi --release
	./scripts/check-exports.sh

package:
	./scripts/package.sh

# Release evidence invokes the same gates as developers, with serial builds.
go-race: go-test
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 -race ./go/rnet ./examples/go/game_quickstart
	GOTOOLCHAIN=local CGO_ENABLED=1 go test -a -p 1 -count=1 -race -tags rnet_test_faults ./go/rnet

msrv-check:
	cargo +$(RNET_MSRV) check --workspace --all-targets --locked

dependency-check:
	cargo audit
	cargo deny check

package-smoke:
	bash scripts/test-package.sh

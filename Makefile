.PHONY: abi-check build check cpp-test go-test loom-test package structure-check test

build:
	cargo build -p rnet-ffi --release

test:
	cargo test --workspace

cpp-test:
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
	GOTOOLCHAIN=local CGO_ENABLED=1 go test ./go/rnet

loom-test:
	cargo test -p rnet-game --features loom admission::tests -- --test-threads=1

check:
	$(MAKE) structure-check
	cargo fmt --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	$(MAKE) loom-test
	$(MAKE) abi-check
	$(MAKE) cpp-test
	$(MAKE) go-test

structure-check:
	./scripts/check-structure.sh

abi-check:
	cargo build -p rnet-ffi --release
	./scripts/check-exports.sh

package:
	./scripts/package.sh

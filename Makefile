build:
	cargo build

run: build
	@read -p "› " msg && cargo run -- -m "$$msg"

pretty: build
	@read -p "› " msg && cargo run -- -p "$$msg"

reset:
	rm -f db/memory.db db/session.db

.PHONY: build run pretty reset

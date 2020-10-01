
RUSTV = stable

build:
	cargo build --all

test-all:
	cargo test --all

test-proxy:
	RUST_LOG=proxy=debug  cargo test test_proxy


install-fmt:
	rustup component add rustfmt --toolchain $(RUSTV)

check-fmt:
	cargo +$(RUSTV) fmt -- --check

install-clippy:
	rustup component add clippy --toolchain $(RUSTV)

check-clippy:	install-clippy
	cargo +$(RUSTV) clippy --all-targets --all-features -- -D warnings


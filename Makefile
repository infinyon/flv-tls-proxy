
RUSTV = stable

build:
	cargo build --all

test-all:
	cargo test --all

test-proxy:
	RUST_LOG=proxy=debug  cargo test test_proxy --no-default-features  --features rust_tls

# currently this does not work
test-proxy-spawn:
	RUST_LOG=proxy=debug  cargo test test_proxy --features spawn


install-fmt:
	rustup component add rustfmt --toolchain $(RUSTV)

check-fmt:
	cargo +$(RUSTV) fmt -- --check

install-clippy:
	rustup component add clippy --toolchain $(RUSTV)

check-clippy:	install-clippy
	cargo +$(RUSTV) clippy --all-targets --all-features -- -D warnings


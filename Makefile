
RUSTV = stable

build:
	cargo build --all

test-all:	test-units test-proxy-rustls test-proxy-native

test-units:
	cargo test --all

test-proxy-rustls:
	cargo test test_rustls --no-default-features  --features rust_tls

test-proxy-native:
	cargo test test_native --no-default-features --features native_tls


install-fmt:
	rustup component add rustfmt --toolchain $(RUSTV)

check-fmt:
	cargo +$(RUSTV) fmt -- --check

install-clippy:
	rustup component add clippy --toolchain $(RUSTV)

check-clippy:	install-clippy
	cargo +$(RUSTV) clippy --all-targets --all-features -- -D warnings


#!/bin/bash

cargo fmt --check && \
    cargo clippy --all-targets --all-features -- -D warnings && \
    cargo deny check && \
    cargo +nightly udeps --all-targets && \
    cargo test --release --all-features && \
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features && \
    cargo build --release && \
    cargo release patch --no-publish --execute

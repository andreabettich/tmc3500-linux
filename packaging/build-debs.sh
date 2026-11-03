#!/bin/sh
# Build static (musl) binaries and .deb packages for arm64 and amd64.
# Static binaries need no particular glibc, so one package per architecture works on Debian 11+,
# Ubuntu 20.04+ and Raspberry Pi OS. Needs: rustup targets (see rust-toolchain.toml), cargo-deb.
set -eu
cd "$(dirname "$0")/.."
for target in aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    var=CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER
    env "$var=rust-lld" cargo build --release --locked --target "$target" \
        -p rastertotmc3500 -p tmc3500-backend -p tmc3500-monitor -p tmc3500ctl
    cargo deb -p tmc3500ctl --no-build --no-strip --target "$target"
done
ls -l target/*/debian/*.deb

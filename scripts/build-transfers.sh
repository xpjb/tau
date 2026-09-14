#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:?Specify bindings, fixture, linux, windows, or android}
output=${2:?Specify output directory}
cd "$root"
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4
mkdir -p "$output"
target=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')

require_space() {
    available=$(df -Pk "$root" | awk 'END { print $4 }')
    (( available >= 1048576 )) || { echo 'Native build stopped: less than 1 GiB free; preserve existing work and free space before retrying.' >&2; exit 1; }
}
require_space

case "$mode" in
    bindings)
        cargo build --locked -p tau-transfer --features bindgen --lib --bin tau-transfer-bindgen
        "$target/debug/tau-transfer-bindgen" generate --library "$target/debug/libtau_transfer.so" \
            --config "$root/transfer/uniffi.toml" --language kotlin --no-format --out-dir "$output"
        ;;
    fixture)
        cargo build --locked -p tau-transfer --example transfer-fixture
        cp "$target/debug/examples/transfer-fixture" "$output/transfer-fixture"
        ;;
    linux)
        cargo build --locked --release -p tau-transfer
        mkdir -p "$output/linux-x86-64"
        cp "$target/release/libtau_transfer.so" "$output/linux-x86-64/"
        ;;
    windows)
        cargo xwin build --locked --release --target x86_64-pc-windows-msvc -p tau-transfer
        mkdir -p "$output/win32-x86-64"
        cp "$target/x86_64-pc-windows-msvc/release/tau_transfer.dll" "$output/win32-x86-64/"
        ;;
    android)
        sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/root/android-sdk}}
        ndk="$sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin"
        for spec in 'aarch64-linux-android arm64-v8a aarch64-linux-android' \
                    'x86_64-linux-android x86_64 x86_64-linux-android' \
                    'armv7-linux-androideabi armeabi-v7a armv7a-linux-androideabi' \
                    'i686-linux-android x86 i686-linux-android'; do
            read -r triple abi compiler <<< "$spec"
            require_space
            variable=${triple//-/_}
            env "CARGO_TARGET_${variable^^}_LINKER=$ndk/${compiler}26-clang" \
                "CARGO_TARGET_${variable^^}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384" \
                "CC_$variable=$ndk/${compiler}26-clang" "AR_$variable=$ndk/llvm-ar" \
                cargo build --locked --release --target "$triple" -p tau-transfer
            mkdir -p "$output/$abi"
            cp "$target/$triple/release/libtau_transfer.so" "$output/$abi/"
        done
        ;;
    *) echo "Unknown native target: $mode" >&2; exit 1 ;;
esac

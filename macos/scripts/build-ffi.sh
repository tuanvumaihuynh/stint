#!/bin/sh
# Builds the Rust core as a static library and writes its Swift bindings into
# macos/Generated/, where the Xcode project expects them. Run it before building in Xcode:
# `just mac-ffi` (debug) or `just mac-ffi release`.
set -eu

profile=${1:-debug}
root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/macos/Generated"

case "$profile" in
debug) cargo_profile="" ;;
release) cargo_profile="--release" ;;
*)
    echo "usage: $0 [debug|release]" >&2
    exit 2
    ;;
esac

cd "$root"
# Match the app's deployment target, or the linker warns about every object file.
export MACOSX_DEPLOYMENT_TARGET=15.0
# shellcheck disable=SC2086 # an empty profile flag must vanish
cargo build --locked -p stint-ffi $cargo_profile
lib="$root/target/$profile/libstint_ffi.a"

bindgen() {
    cargo run --quiet --locked -p stint-ffi --features bindgen --bin stint-bindgen -- "$@"
}

rm -rf "$out"
mkdir -p "$out/Swift" "$out/include" "$out/lib"
bindgen --swift-sources "$lib" "$out/Swift"
bindgen --headers "$lib" "$out/include"
bindgen --modulemap --module-name StintCoreFFI --modulemap-filename module.modulemap "$lib" "$out/include"
cp "$lib" "$out/lib/"
echo "wrote $out"

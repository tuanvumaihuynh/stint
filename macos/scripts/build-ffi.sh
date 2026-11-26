#!/bin/sh
# Builds the Rust core as a static library and writes its Swift bindings into
# macos/Generated/, where the Xcode project expects them. Run it before building in Xcode:
# `just ffi` (debug), `just ffi release`, or `just ffi universal` (release, for Apple
# silicon and Intel Macs, as shipped).
set -eu

profile=${1:-debug}
root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/macos/Generated"

case "$profile" in
debug | release | universal) ;;
*)
    echo "usage: $0 [debug|release|universal]" >&2
    exit 2
    ;;
esac

cd "$root"
# Match the app's deployment target, or the linker warns about every object file.
export MACOSX_DEPLOYMENT_TARGET=15.0
case "$profile" in
debug)
    cargo build --locked -p stint-ffi
    lib="$root/target/debug/libstint_ffi.a"
    ;;
release)
    cargo build --locked -p stint-ffi --release
    lib="$root/target/release/libstint_ffi.a"
    ;;
universal)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    # The slices go in the positional arguments, so paths with spaces stay whole.
    set --
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --locked -p stint-ffi --release --target "$target"
        set -- "$@" "$root/target/$target/release/libstint_ffi.a"
    done
    lib="$root/target/universal/release/libstint_ffi.a"
    mkdir -p "$(dirname "$lib")"
    lipo -create "$@" -output "$lib"
    ;;
esac

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

#!/bin/sh
# Builds Stint for release: one universal app for Apple silicon and Intel Macs, ad-hoc
# signed, packed as a zip (for updates) and a disk image (for first installs).
#
#   macos/scripts/release.sh 0.1.0
#
# The version is the tag without its `v`. The build number counts the commits on the
# branch, so it only grows. Everything lands in target/dist/.
#
# The app checks for updates with the EdDSA public key in macos/sparkle-public-key.txt.
# For trying updates against a test feed, these override the shipped values:
#   SPARKLE_PUBLIC_KEY  the key updates must be signed with
#   SPARKLE_FEED_URL    where the appcast is read from
#   STINT_BUNDLE_ID     a bundle id that keeps the test app's settings apart
#   STINT_BUILD_NUMBER  the build number, to build two versions from one commit
set -eu

version=${1:-}
case "$version" in
"" | *[!0-9.]* | .* | *. | *..*)
    echo "usage: $0 <major.minor.patch>" >&2
    exit 2
    ;;
esac
if [ "$(echo "$version" | tr -cd . | wc -c)" -ne 2 ]; then
    echo "version must be major.minor.patch, got $version" >&2
    exit 2
fi

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
build_number=${STINT_BUILD_NUMBER:-$(git rev-list --count HEAD)}
dist="$root/target/dist"
derived="$root/macos/build/release"
app="$derived/Build/Products/Release/Stint.app"

public_key=${SPARKLE_PUBLIC_KEY:-$(cat macos/sparkle-public-key.txt 2>/dev/null || true)}
if [ -z "$public_key" ]; then
    echo "no Sparkle public key: add it to macos/sparkle-public-key.txt" >&2
    exit 1
fi
set -- SPARKLE_PUBLIC_KEY="$public_key"
if [ -n "${SPARKLE_FEED_URL:-}" ]; then
    set -- "$@" SPARKLE_FEED_URL="$SPARKLE_FEED_URL"
fi
if [ -n "${STINT_BUNDLE_ID:-}" ]; then
    set -- "$@" PRODUCT_BUNDLE_IDENTIFIER="$STINT_BUNDLE_ID"
fi

macos/scripts/build-ffi.sh universal

rm -rf "$derived"
# Ad-hoc builds leave the hardened runtime off: its library validation refuses to load
# the embedded Sparkle framework, since ad-hoc code has no Team ID. The runtime is only
# required for notarization, which an ad-hoc build cannot have.
xcodebuild -quiet \
    -project macos/Stint.xcodeproj \
    -scheme Stint \
    -configuration Release \
    -destination "generic/platform=macOS" \
    -derivedDataPath "$derived" \
    ARCHS="arm64 x86_64" \
    ONLY_ACTIVE_ARCH=NO \
    MARKETING_VERSION="$version" \
    CURRENT_PROJECT_VERSION="$build_number" \
    ENABLE_HARDENED_RUNTIME=NO \
    "$@" \
    build

# What ships must be what was asked for.
binary="$app/Contents/MacOS/Stint"
for arch in arm64 x86_64; do
    if ! lipo -archs "$binary" | tr ' ' '\n' | grep -qx "$arch"; then
        echo "the app has no $arch slice" >&2
        exit 1
    fi
done
plist() { /usr/libexec/PlistBuddy -c "Print :$1" "$app/Contents/Info.plist"; }
if [ "$(plist CFBundleShortVersionString)" != "$version" ] ||
    [ "$(plist CFBundleVersion)" != "$build_number" ]; then
    echo "the app's Info.plist does not carry version $version ($build_number)" >&2
    exit 1
fi
if [ "$(plist SUPublicEDKey)" != "$public_key" ]; then
    echo "the app does not carry the Sparkle public key" >&2
    exit 1
fi
codesign --verify --deep --strict "$app"

rm -rf "$dist"
mkdir -p "$dist"
zip="$dist/Stint-$version.zip"
dmg="$dist/Stint-$version.dmg"
# ditto keeps the bundle's symlinks, permissions, and signature intact.
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
ditto "$app" "$staging/Stint.app"
ln -s /Applications "$staging/Applications"
hdiutil create -quiet -volname "Stint $version" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg"

(cd "$dist" && shasum -a 256 "Stint-$version.zip" "Stint-$version.dmg" >SHA256SUMS)
echo "built Stint $version ($build_number)"
ls -lh "$dist"

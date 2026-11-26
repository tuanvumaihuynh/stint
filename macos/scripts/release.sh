#!/bin/sh
# Builds Stint for release: one universal app for Apple silicon and Intel Macs, ad-hoc
# signed, packed as a zip (for updates) and a disk image (for first installs).
#
#   macos/scripts/release.sh 0.1.0
#
# The version is the tag without its `v`. The build number counts the commits on the
# branch, so it only grows. Everything lands in target/dist/.
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
build_number=$(git rev-list --count HEAD)
dist="$root/target/dist"
derived="$root/macos/build/release"
app="$derived/Build/Products/Release/Stint.app"

macos/scripts/build-ffi.sh universal

rm -rf "$derived"
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

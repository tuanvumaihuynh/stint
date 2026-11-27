#!/bin/sh
# Signs a release zip with the Sparkle EdDSA key and writes its appcast to target/dist/.
#
#   SPARKLE_PRIVATE_KEY=... macos/scripts/appcast.sh 0.1.0
#
# Run after macos/scripts/release.sh. The private key comes only from the environment,
# never from the login Keychain, and must match macos/sparkle-public-key.txt.
#   SPARKLE_PRIVATE_KEY          the base64 EdDSA private key (required)
#   SPARKLE_PUBLIC_KEY           the key the app checks updates with
#   SPARKLE_DIST_DIR             where the release zip is, target/dist by default
#   SPARKLE_BIN_DIR              where Sparkle's generate_appcast is
#   SPARKLE_DOWNLOAD_URL_PREFIX  where the zip is downloaded from
#   SPARKLE_RELEASE_URL          the release page linked from the appcast
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
dist=${SPARKLE_DIST_DIR:-$root/target/dist}
zip="$dist/Stint-$version.zip"

private_key=$(printf '%s' "${SPARKLE_PRIVATE_KEY:-}" | tr -d '[:space:]')
if [ -z "$private_key" ]; then
    echo "SPARKLE_PRIVATE_KEY is not set" >&2
    exit 1
fi
if [ ! -f "$zip" ]; then
    echo "no $zip: run macos/scripts/release.sh $version first" >&2
    exit 1
fi

public_key=${SPARKLE_PUBLIC_KEY:-$(tr -d "[:space:]" <macos/sparkle-public-key.txt 2>/dev/null || true)}
if [ -z "$public_key" ]; then
    echo "no Sparkle public key: add it to macos/sparkle-public-key.txt" >&2
    exit 1
fi

# The key is read from stdin so it never shows up in a process list.
derived_key=$(printf '%s\n' "$private_key" | swift -e '
import CryptoKit
import Foundation

let text = (readLine() ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
guard let data = Data(base64Encoded: text) else {
    FileHandle.standardError.write("SPARKLE_PRIVATE_KEY is not valid base64\n".data(using: .utf8)!)
    exit(1)
}
guard data.count == 32 else {
    FileHandle.standardError.write("SPARKLE_PRIVATE_KEY must be a 32-byte Ed25519 seed (generate_keys -x), got \(data.count) bytes\n".data(using: .utf8)!)
    exit(1)
}
guard let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: data) else {
    FileHandle.standardError.write("SPARKLE_PRIVATE_KEY is not an Ed25519 key\n".data(using: .utf8)!)
    exit(1)
}
print(key.publicKey.rawRepresentation.base64EncodedString())
')
if [ "$derived_key" != "$public_key" ]; then
    echo "SPARKLE_PRIVATE_KEY does not match the Sparkle public key the app ships with" >&2
    exit 1
fi

if [ -n "${SPARKLE_BIN_DIR:-}" ]; then
    generate_appcast="$SPARKLE_BIN_DIR/generate_appcast"
else
    generate_appcast="$root/macos/build/release/SourcePackages/artifacts/sparkle/Sparkle/bin/generate_appcast"
    if [ ! -x "$generate_appcast" ]; then
        generate_appcast="$root/macos/build/SourcePackages/artifacts/sparkle/Sparkle/bin/generate_appcast"
    fi
fi
if [ ! -x "$generate_appcast" ]; then
    echo "no generate_appcast at $generate_appcast: build the app first or set SPARKLE_BIN_DIR" >&2
    exit 1
fi

# Only the zip goes in, so the disk image and checksums never end up in the feed.
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
cp "$zip" "$staging/"

repo=${GITHUB_REPOSITORY:-tuanvumaihuynh/stint}
download_url_prefix=${SPARKLE_DOWNLOAD_URL_PREFIX:-https://github.com/$repo/releases/download/v$version/}
release_url=${SPARKLE_RELEASE_URL:-https://github.com/$repo/releases/tag/v$version}

printf '%s\n' "$private_key" | "$generate_appcast" \
    --ed-key-file - \
    --download-url-prefix "$download_url_prefix" \
    --link "$release_url" \
    --full-release-notes-url "$release_url" \
    -o "$staging/appcast.xml" \
    "$staging"

appcast="$staging/appcast.xml"
xmllint --noout "$appcast"
if ! grep -q 'sparkle:edSignature="' "$appcast"; then
    echo "the appcast has no EdDSA signature" >&2
    exit 1
fi
if ! grep -qF "${download_url_prefix}Stint-${version}.zip" "$appcast"; then
    echo "the appcast does not point at ${download_url_prefix}Stint-${version}.zip" >&2
    exit 1
fi
cp "$appcast" "$dist/appcast.xml"
echo "wrote $dist/appcast.xml"

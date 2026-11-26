#!/bin/sh
# Launches the app from a release disk image against a database from an older Stint, and
# checks that it opens and migrates. Runs every architecture this Mac can run.
#
#   macos/scripts/smoke-test.sh target/dist/Stint-0.1.0.dmg
#
# The database is built from the migrations of the oldest schema that still has users,
# so it is always what that release wrote. Nothing touches the real Stint data.
set -eu

dmg=${1:?usage: $0 <Stint.dmg>}
root=$(cd "$(dirname "$0")/../.." && pwd)
migrations="$root/crates/stint-core/src/store/sqlite/migrations"
# Version 3 is the last schema that stored Antigravity accounts as `gemini`.
old_schema=3
latest_schema=$(find "$migrations" -name '*.sql' | wc -l | tr -d ' ')

work=$(mktemp -d)
mount="$work/mount"
cleanup() {
    hdiutil detach -quiet "$mount" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount" "$dmg"
if [ ! -L "$mount/Applications" ]; then
    echo "the disk image has no Applications shortcut" >&2
    exit 1
fi
ditto "$mount/Stint.app" "$work/Stint.app"
hdiutil detach -quiet "$mount"
binary="$work/Stint.app/Contents/MacOS/Stint"

old_database() {
    data="$1"
    mkdir -p "$data"
    for migration in $(find "$migrations" -name '*.sql' | sort | head -n "$old_schema"); do
        sqlite3 "$data/stint.db" <"$migration"
    done
    sqlite3 "$data/stint.db" "PRAGMA user_version = $old_schema;
        INSERT INTO accounts (provider, label, credential_key, created_at)
        VALUES ('gemini', 'Smoke', 'smoke-test-missing-key', 0);"
}

# Runs the app's launch check, giving up after a while instead of hanging CI.
launch() {
    data="$1"
    shift
    STINT_DATA_DIR="$data" "$@" "$binary" --launch-check >"$work/out" 2>"$work/err" &
    pid=$!
    waited=0
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$waited" -ge 30 ]; then
            kill "$pid"
            echo "the app did not finish its launch check" >&2
            exit 1
        fi
        sleep 1
        waited=$((waited + 1))
    done
    if ! wait "$pid"; then
        echo "the launch check failed:" >&2
        cat "$work/err" >&2
        exit 1
    fi
}

check() {
    name="$1"
    shift
    data="$work/data-$name"
    old_database "$data"
    launch "$data" "$@"
    if ! grep -q "^launch check: .*1 account" "$work/out"; then
        echo "$name: unexpected launch check output:" >&2
        cat "$work/out" "$work/err" >&2
        exit 1
    fi
    version=$(sqlite3 "$data/stint.db" "PRAGMA user_version;")
    provider=$(sqlite3 "$data/stint.db" "SELECT provider FROM accounts;")
    if [ "$version" != "$latest_schema" ] || [ "$provider" != "antigravity" ]; then
        echo "$name: the database was not migrated (schema $version, provider $provider)" >&2
        exit 1
    fi
    echo "$name: $(cat "$work/out")"
}

check "$(uname -m)"
# Apple silicon Macs with Rosetta also run the Intel slice.
if [ "$(uname -m)" = arm64 ] && arch -x86_64 /usr/bin/true 2>/dev/null; then
    check x86_64 arch -x86_64
fi
echo "smoke test passed"

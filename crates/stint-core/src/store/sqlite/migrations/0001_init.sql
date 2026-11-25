-- Every `*_at` column holds UTC Unix time in milliseconds.

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    label TEXT NOT NULL,
    email TEXT,
    is_active INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (provider, label)
);

CREATE UNIQUE INDEX accounts_one_active_per_provider
    ON accounts (provider) WHERE is_active = 1;

-- One row per poll, so an empty poll and two polls in the same millisecond
-- stay distinct.
CREATE TABLE quota_snapshots (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    fetched_at INTEGER NOT NULL
);

CREATE INDEX quota_snapshots_account_fetched
    ON quota_snapshots (account_id, fetched_at);

CREATE TABLE quota_windows (
    id INTEGER PRIMARY KEY,
    snapshot_id INTEGER NOT NULL REFERENCES quota_snapshots (id) ON DELETE CASCADE,
    window_kind TEXT NOT NULL,
    window_scope TEXT,
    used_percent REAL NOT NULL,
    resets_at INTEGER
);

CREATE INDEX quota_windows_snapshot
    ON quota_windows (snapshot_id);

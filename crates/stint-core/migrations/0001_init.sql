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

CREATE TABLE quota_snapshots (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    window_kind TEXT NOT NULL,
    window_scope TEXT,
    used_percent REAL NOT NULL,
    resets_at INTEGER,
    fetched_at INTEGER NOT NULL
);

CREATE INDEX quota_snapshots_account_fetched
    ON quota_snapshots (account_id, fetched_at);

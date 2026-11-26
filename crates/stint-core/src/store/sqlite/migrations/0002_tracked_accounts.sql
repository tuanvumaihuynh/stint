-- Every account is tracked at once, so none is "active", and each account's login
-- lives in a Keychain item named by `credential_key`. No release has added accounts
-- yet, so the table is rebuilt instead of altered.

DROP TABLE accounts;

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL,
    label TEXT NOT NULL,
    email TEXT,
    plan TEXT,
    credential_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    UNIQUE (provider, label)
);

-- Whether the status menu lists the account; the desktop window always does.
ALTER TABLE accounts ADD COLUMN shown_in_menu INTEGER NOT NULL DEFAULT 1;

-- App preferences, such as the refresh interval, as text values by key.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

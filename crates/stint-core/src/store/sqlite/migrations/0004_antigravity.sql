-- Google renamed the provider: Gemini accounts are Antigravity accounts.
UPDATE accounts SET provider = 'antigravity' WHERE provider = 'gemini';

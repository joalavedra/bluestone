-- OAuth tokens from Bluestone's own connect flows (e.g. Faire), AES-256-GCM encrypted with a key
-- derived from BLUESTONE_SECRET_KEY; channels using them have credential_env = 'oauth'.
CREATE TABLE channel_tokens (
  channel_id INTEGER PRIMARY KEY REFERENCES channels(id),
  nonce TEXT NOT NULL,
  ciphertext TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE oauth_states (
  state TEXT PRIMARY KEY,
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- Keys pasted in Settings → Integrations, AES-256-GCM encrypted with a key derived from
-- BLUESTONE_SECRET_KEY. Channels and accounting connections reference them as credential_env = 'secret:<id>'.
CREATE TABLE secrets (
  id INTEGER PRIMARY KEY,
  nonce TEXT NOT NULL,
  ciphertext TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- One accounting system (QuickBooks, Holded, ...) per brand; documents are pushed, never pulled.
CREATE TABLE accounting_connections (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  kind TEXT NOT NULL CHECK (kind IN ('quickbooks', 'holded')),
  base_url TEXT NOT NULL,
  -- QuickBooks company id (realm); unused by Holded.
  realm TEXT,
  -- Same references as channels: NAME, env:NAME or nango:<integration>/<connection>.
  credential_env TEXT NOT NULL,
  -- QuickBooks account ids used when Bluestone has to create an item.
  income_account TEXT,
  expense_account TEXT,
  -- Replaced connections stay (inactive) so their exported documents keep their history.
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE UNIQUE INDEX accounting_connections_active ON accounting_connections(brand_id) WHERE active = 1;

-- Cache of contacts and items already found or created in the accounting system.
CREATE TABLE accounting_links (
  connection_id INTEGER NOT NULL REFERENCES accounting_connections(id),
  entity TEXT NOT NULL CHECK (entity IN ('supplier', 'customer', 'item')),
  local_key TEXT NOT NULL,
  external_id TEXT NOT NULL,
  PRIMARY KEY (connection_id, entity, local_key)
);

-- A purchase order becomes a bill, a sales order an invoice; one per connection, so retries never duplicate.
CREATE TABLE accounting_documents (
  id INTEGER PRIMARY KEY,
  connection_id INTEGER NOT NULL REFERENCES accounting_connections(id),
  source TEXT NOT NULL CHECK (source IN ('purchase_order', 'sales_order')),
  source_id INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'exported', 'failed')),
  external_id TEXT,
  error TEXT,
  claim TEXT,
  created_by TEXT,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (connection_id, source, source_id)
);

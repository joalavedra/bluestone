-- One ERP per brand (Holded, Odoo, ICG, ...). Products are pulled and matched to items by SKU,
-- invoices are read on demand, confirmed sales orders are pushed as ERP orders.
-- No CHECK on kind, so a new ERP adapter needs no table rebuild.
CREATE TABLE erp_connections (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  kind TEXT NOT NULL,
  base_url TEXT NOT NULL,
  -- Same references as channels: NAME, env:NAME, nango:<integration>/<connection> or secret:<id>.
  credential_env TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1,
  last_synced_at TEXT,
  last_sync_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE UNIQUE INDEX erp_connections_active ON erp_connections(brand_id) WHERE active = 1;

-- Latest product pull; joined to items on (brand, sku) so merges and new items need no relinking.
CREATE TABLE erp_products (
  connection_id INTEGER NOT NULL REFERENCES erp_connections(id),
  code TEXT NOT NULL,
  sku TEXT,
  name TEXT NOT NULL,
  stock REAL,
  PRIMARY KEY (connection_id, code)
);
CREATE INDEX erp_products_sku ON erp_products(connection_id, sku);

-- A confirmed sales order pushed as an ERP order; one per connection, so retries never duplicate.
CREATE TABLE erp_orders (
  id INTEGER PRIMARY KEY,
  connection_id INTEGER NOT NULL REFERENCES erp_connections(id),
  sales_order_id INTEGER NOT NULL REFERENCES sales_orders(id),
  status TEXT NOT NULL CHECK (status IN ('pending', 'exported', 'failed')),
  external_id TEXT,
  error TEXT,
  claim TEXT,
  created_by TEXT,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (connection_id, sales_order_id)
);

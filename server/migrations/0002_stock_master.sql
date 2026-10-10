-- Bluestone as stock master: per-brand mode, warehouses, an append-only stock ledger.
ALTER TABLE brands ADD COLUMN stock_mode TEXT NOT NULL DEFAULT 'mirror' CHECK (stock_mode IN ('mirror','master'));
-- Orders placed before this instant are history and never hit the ledger.
ALTER TABLE brands ADD COLUMN master_since TEXT;

CREATE TABLE warehouses (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  name TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (brand_id, name)
);

-- Channel locations stock from a warehouse; same-named locations across channels share one.
ALTER TABLE locations ADD COLUMN warehouse_id INTEGER REFERENCES warehouses(id);

CREATE TABLE stock_ledger (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES items(id),
  warehouse_id INTEGER NOT NULL REFERENCES warehouses(id),
  delta INTEGER NOT NULL,
  reason TEXT NOT NULL CHECK (reason IN ('initial','sale','adjustment','transfer','receipt','correction')),
  ref_type TEXT,
  ref_id INTEGER,
  actor TEXT NOT NULL,
  note TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX stock_ledger_item ON stock_ledger(item_id, warehouse_id);

ALTER TABLE order_lines ADD COLUMN ledgered INTEGER NOT NULL DEFAULT 0;

-- Ledger proposals target a warehouse rather than a channel listing.
CREATE TABLE proposals_new (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('stock_adjustment','price_change','listing_status','ledger_adjustment','stock_transfer')),
  item_id INTEGER REFERENCES items(id),
  listing_id INTEGER REFERENCES listings(id),
  before_json TEXT NOT NULL,
  after_json TEXT NOT NULL,
  rationale TEXT,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','rejected','applied','failed')),
  actor TEXT NOT NULL,
  decided_by TEXT,
  decided_at TEXT,
  error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
INSERT INTO proposals_new SELECT id, kind, item_id, listing_id, before_json, after_json, rationale, status, actor, decided_by, decided_at, error, created_at FROM proposals;
DROP TABLE proposals;
ALTER TABLE proposals_new RENAME TO proposals;
CREATE INDEX proposals_status ON proposals(status, created_at);

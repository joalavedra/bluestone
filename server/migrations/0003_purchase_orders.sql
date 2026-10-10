CREATE TABLE purchase_orders (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  supplier_id INTEGER REFERENCES suppliers(id),
  -- Where goods are received in master mode.
  warehouse_id INTEGER REFERENCES warehouses(id),
  status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','approved','sent','partial','received','cancelled')),
  note TEXT,
  created_by TEXT NOT NULL,
  approved_by TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE po_lines (
  id INTEGER PRIMARY KEY,
  po_id INTEGER NOT NULL REFERENCES purchase_orders(id),
  item_id INTEGER NOT NULL REFERENCES items(id),
  quantity INTEGER NOT NULL CHECK (quantity > 0),
  received INTEGER NOT NULL DEFAULT 0 CHECK (received >= 0),
  unit_cost REAL,
  UNIQUE (po_id, item_id)
);
CREATE INDEX po_lines_item ON po_lines(item_id);

-- Off-platform sales and wholesale orders (email, phone, photo of an order sheet), proposed by
-- the agent and confirmed by a human. Confirmed orders reserve stock until fulfilled.
CREATE TABLE sales_orders (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  customer TEXT NOT NULL,
  customer_email TEXT,
  external_ref TEXT,
  -- Where the agent read the order from, e.g. "email from buyer@shop.com, 3 Oct".
  source TEXT,
  warehouse_id INTEGER REFERENCES warehouses(id),
  status TEXT NOT NULL DEFAULT 'proposed' CHECK (status IN ('proposed','confirmed','fulfilled','rejected','cancelled')),
  note TEXT,
  created_by TEXT NOT NULL,
  approved_by TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE so_lines (
  id INTEGER PRIMARY KEY,
  so_id INTEGER NOT NULL REFERENCES sales_orders(id),
  item_id INTEGER NOT NULL REFERENCES items(id),
  quantity INTEGER NOT NULL CHECK (quantity > 0),
  unit_price REAL,
  UNIQUE (so_id, item_id)
);
CREATE INDEX so_lines_item ON so_lines(item_id);

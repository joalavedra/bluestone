CREATE TABLE brands (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE suppliers (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  email TEXT,
  lead_time_days INTEGER,
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- Credentials are never stored: `credential_env` names an env var holding the token/key.
CREATE TABLE channels (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  kind TEXT NOT NULL CHECK (kind IN ('shopify','prestashop')),
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  credential_env TEXT NOT NULL,
  last_synced_at TEXT,
  last_sync_error TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (brand_id, name)
);

CREATE TABLE locations (
  id INTEGER PRIMARY KEY,
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  external_id TEXT NOT NULL,
  name TEXT NOT NULL,
  UNIQUE (channel_id, external_id)
);

CREATE TABLE items (
  id INTEGER PRIMARY KEY,
  brand_id INTEGER NOT NULL REFERENCES brands(id),
  sku TEXT NOT NULL,
  name TEXT NOT NULL,
  supplier_id INTEGER REFERENCES suppliers(id),
  reorder_point INTEGER,
  target_stock INTEGER,
  lead_time_days INTEGER,
  unit_cost REAL,
  archived INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (brand_id, sku)
);

CREATE TABLE listings (
  id INTEGER PRIMARY KEY,
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  item_id INTEGER REFERENCES items(id),
  external_product_id TEXT NOT NULL,
  external_variant_id TEXT NOT NULL DEFAULT '',
  inventory_ref TEXT,
  sku TEXT,
  title TEXT NOT NULL,
  price REAL,
  status TEXT,
  image_url TEXT,
  synced_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (channel_id, external_product_id, external_variant_id)
);

CREATE TABLE listing_stock (
  listing_id INTEGER NOT NULL REFERENCES listings(id),
  location_id INTEGER NOT NULL REFERENCES locations(id),
  stock_ref TEXT,
  quantity INTEGER NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  PRIMARY KEY (listing_id, location_id)
);

CREATE TABLE stock_snapshots (
  id INTEGER PRIMARY KEY,
  listing_id INTEGER NOT NULL REFERENCES listings(id),
  quantity INTEGER NOT NULL,
  taken_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX stock_snapshots_listing ON stock_snapshots(listing_id, taken_at);

CREATE TABLE order_lines (
  id INTEGER PRIMARY KEY,
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  external_order_id TEXT NOT NULL,
  external_line_id TEXT NOT NULL,
  listing_id INTEGER REFERENCES listings(id),
  sku TEXT,
  quantity INTEGER NOT NULL,
  unit_price REAL,
  ordered_at TEXT NOT NULL,
  UNIQUE (channel_id, external_order_id, external_line_id)
);
CREATE INDEX order_lines_listing ON order_lines(listing_id, ordered_at);

CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE item_tags (
  item_id INTEGER NOT NULL REFERENCES items(id),
  tag_id INTEGER NOT NULL REFERENCES tags(id),
  PRIMARY KEY (item_id, tag_id)
);

CREATE TABLE notes (
  id INTEGER PRIMARY KEY,
  item_id INTEGER NOT NULL REFERENCES items(id),
  body TEXT NOT NULL,
  actor TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE proposals (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('stock_adjustment','price_change','listing_status')),
  item_id INTEGER REFERENCES items(id),
  listing_id INTEGER NOT NULL REFERENCES listings(id),
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
CREATE INDEX proposals_status ON proposals(status, created_at);

CREATE TABLE activity (
  id INTEGER PRIMARY KEY,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('agent','human','system')),
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  subject_type TEXT,
  subject_id INTEGER,
  summary TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX activity_created ON activity(created_at);

CREATE TABLE tokens (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('agent','human')),
  token_hash TEXT NOT NULL UNIQUE,
  scopes TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  last_used_at TEXT
);

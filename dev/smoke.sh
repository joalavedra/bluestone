#!/usr/bin/env bash
# End-to-end smoke test against the mock Shopify dev shop: sync → MCP propose → REST approve → store updated.
set -euo pipefail
cd "$(dirname "$0")/../server"
cargo build -q
BIN=target/debug
WORK=$(mktemp -d)
export BLUESTONE_DATABASE="sqlite://$WORK/smoke.db" SHOPIFY_DEV_TOKEN=shpat_dev_mock MOCK_SHOPIFY_ADDR=127.0.0.1:18788
$BIN/mock_shopify & MOCK=$!
$BIN/bluestone serve --addr 127.0.0.1:18787 & SRV=$!
trap 'kill $MOCK $SRV 2>/dev/null; rm -rf "$WORK"' EXIT
sleep 1
HUMAN=$($BIN/bluestone token create --name joan --kind human --scopes admin 2>/dev/null)
AGENT=$($BIN/bluestone token create --name smoke-agent 2>/dev/null)
$BIN/bluestone channel add --brand "Northwind Coffee" --kind shopify --name shopify-dev --base-url http://127.0.0.1:18788 --credential-env SHOPIFY_DEV_TOKEN >/dev/null
API=http://127.0.0.1:18787
H=(-sf -H "Authorization: Bearer $HUMAN" -H "Content-Type: application/json")
curl "${H[@]}" -X POST $API/api/sync | jq -c '.[] | {channel, listings, order_lines}'
curl "${H[@]}" $API/api/overview | jq -c '{items, low_stock, out_of_stock, units_30d}'

mcp() { curl -sf -X POST $API/mcp -H "Authorization: Bearer $AGENT" -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" -H "MCP-Protocol-Version: 2025-06-18" ${SID:+-H "Mcp-Session-Id: $SID"} -d "$1" -D "$WORK/h"; }
mcp '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' >/dev/null
SID=$(grep -i '^mcp-session-id' "$WORK/h" | cut -d' ' -f2 | tr -d '\r' || true)
mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' >/dev/null || true
echo "tools: $(mcp '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | sed -n 's/^data: //p' | jq '.result.tools | length')"
mcp '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tag_items","arguments":{"items":["NW-ESP-250"],"tags":["bestseller"]}}}' >/dev/null
OUT=$(mcp '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"propose_stock_adjustment","arguments":{"item":"NW-ESP-250","location":"Barcelona warehouse","delta":40,"rationale":"4/day velocity, 2 days cover"}}}' | sed -n 's/^data: //p')
PID=$(echo "$OUT" | jq -r '.result.content[0].text' | jq -r .id)
echo "proposal #$PID"
curl "${H[@]}" -X POST $API/api/proposals/$PID/approve | jq -c '{status, error, before, after}'
curl "${H[@]}" -X POST $API/api/sync >/dev/null
curl "${H[@]}" "$API/api/items?query=NW-ESP-250" | jq -c '.[0] | {sku, on_hand, tags, status}'
curl "${H[@]}" "$API/api/activity?limit=5" | jq -r '.[] | "\(.actor_kind)/\(.actor): \(.summary)"'

echo "--- master mode: Bluestone owns stock"
curl "${H[@]}" -X POST "$API/api/brands/Northwind%20Coffee/stock-mode" -d '{"mode":"master"}' | jq -c '{mode, pushed: .reconcile.pushed, errors: .reconcile.push_errors}'
IID=$(curl "${H[@]}" "$API/api/items?query=NW-ESP-250" | jq '.[0].id')
curl "${H[@]}" "$API/api/items/$IID/ledger" | jq -c '.levels'
OUT=$(mcp '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"propose_stock_adjustment","arguments":{"item":"NW-ESP-250","location":"Barcelona warehouse","quantity":60,"rationale":"cycle count"}}}' | sed -n 's/^data: //p')
PID=$(echo "$OUT" | jq -r '.result.content[0].text' | jq -r .id)
curl "${H[@]}" -X POST $API/api/proposals/$PID/approve | jq -c '{kind, status, error}'
shop() { curl -sf -X POST http://127.0.0.1:18788/admin/api/2025-07/graphql.json -H "X-Shopify-Access-Token: shpat_dev_mock" -H "Content-Type: application/json" -d "$1"; }
PRODUCTS='{"query":"query { products(first: 50) { nodes { id } } }"}'
INV=$(shop "$PRODUCTS" | jq -r '[..|objects|select(.sku? == "NW-ESP-250")][0].inventoryItem.id')
LOC=$(shop '{"query":"query { locations(first: 50) { nodes { id name } } }"}' | jq -r '[..|objects|select(.name? == "Barcelona warehouse")][0].id')
qty() { shop "$PRODUCTS" | jq --arg l "$LOC" '[..|objects|select(.sku? == "NW-ESP-250")][0] | [..|objects|select(.location?.id? == $l)][0] | .quantities[0].quantity'; }
echo "store after approval: $(qty)"
# Someone edits the store directly: the next sync reports drift and the ledger wins.
shop "{\"query\":\"mutation inventorySetQuantities(\$input: InventorySetQuantitiesInput!) { inventorySetQuantities(input: \$input) { userErrors { message } } }\",\"variables\":{\"input\":{\"quantities\":[{\"inventoryItemId\":\"$INV\",\"locationId\":\"$LOC\",\"quantity\":5}]}}}" >/dev/null
echo "store after manual edit: $(qty)"
curl "${H[@]}" -X POST $API/api/sync | jq -c '.[0].master'
echo "store after sync: $(qty)"
test "$(qty)" = 60

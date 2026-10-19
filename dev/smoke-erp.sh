#!/usr/bin/env bash
# ERP smoke test (Holded mock): connect from Settings, pull products and match by SKU, read invoices
# with customer country, push a confirmed sales order once, refuse agents and cancels.
set -euo pipefail
cd "$(dirname "$0")/../server"
cargo build -q
BIN=target/debug
WORK=$(mktemp -d)
export BLUESTONE_DATABASE="sqlite://$WORK/smoke.db" SHOPIFY_DEV_TOKEN=shpat_dev_mock MOCK_SHOPIFY_ADDR=127.0.0.1:18898 \
  MOCK_ACCOUNTING_ADDR=127.0.0.1:18899 MOCK_HOLDED_SEED=1 HOLDED_DEV_KEY=holded_dev_mock HOLDED_BAD_KEY=wrong
$BIN/mock_shopify & MOCK=$!
$BIN/mock_accounting & ERP=$!
trap 'kill $MOCK $ERP ${SRV:-} 2>/dev/null; rm -rf "$WORK"' EXIT
HUMAN=$($BIN/bluestone token create --name joan --kind human --scopes admin 2>/dev/null)
AGENT=$($BIN/bluestone token create --name smoke-agent 2>/dev/null)
$BIN/bluestone channel add --brand "Northwind Coffee" --kind shopify --name shopify-dev --base-url http://127.0.0.1:18898 --credential-env SHOPIFY_DEV_TOKEN >/dev/null
$BIN/bluestone serve --addr 127.0.0.1:18897 & SRV=$!
for _ in $(seq 50); do curl -s -o /dev/null http://127.0.0.1:18897/api/me && break; sleep 0.1; done
API=http://127.0.0.1:18897/api
H=(-sf -H "Authorization: Bearer $HUMAN" -H "Content-Type: application/json")
code() { curl -s -o "$WORK/body" -w '%{http_code}' -H "Authorization: Bearer $1" -H "Content-Type: application/json" -X POST "$API/$2" -d "${3:-{\}}"; }
dump() { curl -sf http://127.0.0.1:18899/_dump | jq -c "$1"; }
curl "${H[@]}" -X POST $API/sync >/dev/null
ERP_IN='{"connector":"holded_erp","brand":"Northwind Coffee","fields":{"base_url":"http://127.0.0.1:18899"},"credential_ref":"HOLDED_DEV_KEY"}'

echo "--- connect Holded as the ERP from Settings → Integrations"
test "$(curl "${H[@]}" $API/integrations | jq '[.connectors[] | select(.category == "erp")] | length')" -ge 2
test "$(code "$AGENT" integrations "$ERP_IN")" = 403
test "$(code "$AGENT" erp '{"brand":"Northwind Coffee","kind":"holded","credential_env":"HOLDED_DEV_KEY"}')" = 403
curl "${H[@]}" -X POST $API/integrations/test -d "${ERP_IN/HOLDED_DEV_KEY/HOLDED_BAD_KEY}" | jq -c .
test "$(curl "${H[@]}" -X POST $API/integrations/test -d "${ERP_IN/HOLDED_DEV_KEY/HOLDED_BAD_KEY}" | jq .ok)" = false
curl "${H[@]}" -X POST $API/integrations/test -d "$ERP_IN" | jq -c .
test "$(curl "${H[@]}" -X POST $API/integrations/test -d "$ERP_IN" | jq -r .message)" = "Holded (ERP): API key works · 4 products"
test "$(curl "${H[@]}" -X POST $API/integrations -d "$ERP_IN" | jq -r .kind)" = erp
curl "${H[@]}" $API/integrations | jq -c '.connections[] | select(.kind == "erp") | {connector, brand, label: .label, credential_source}'

echo "--- pull products, match to items by SKU"
curl "${H[@]}" -X POST $API/erp/sync -d '{"brand":"Northwind Coffee"}' | jq -c '{kind, products, matched}'
test "$(curl "${H[@]}" $API/erp | jq -c '.connections[0] | [.products, .matched]')" = "[4,3]"
ITEM=$(curl "${H[@]}" "$API/items?query=NW-ESP-250" | jq '.[0].id')
curl "${H[@]}" "$API/items/$ITEM" | jq -c .erp
COL=$(curl "${H[@]}" "$API/items?query=NW-COL-250" | jq '.[0].id')
test "$(curl "${H[@]}" "$API/items/$COL" | jq -r '.erp[0].code')" = hp004
test "$(curl "${H[@]}" "$API/items/$ITEM" | jq -c '.erp[0] | [.system, .code, .stock]')" = '["holded","hp001",31.0]' \
  || test "$(curl "${H[@]}" "$API/items/$ITEM" | jq -c '.erp[0] | [.system, .code, .stock]')" = '["holded","hp001",31]'

echo "--- read invoices with customer country (agents can read too)"
INV=$(curl -sf -H "Authorization: Bearer $AGENT" "$API/erp/invoices?brand=Northwind%20Coffee&from=2026-09-01&to=2026-09-30")
echo "$INV" | jq -c '.[] | {number, date, customer, country, currency, total, lines: [.lines[] | [.sku, .quantity, .amount]]}'
test "$(echo "$INV" | jq -c '.[0] | [.country, .currency, .date, (.lines | map(.amount) | add)]')" = '["FR","EUR","2026-09-15",97]'
test "$(curl "${H[@]}" "$API/erp/invoices?brand=Northwind%20Coffee&from=2026-10-01&to=2026-10-31" | jq length)" = 0

echo "--- push a confirmed sales order once; agents and cancels are refused"
SO=$(curl "${H[@]}" -X POST $API/sales-orders -d '{"brand":"Northwind Coffee","customer":"Cafe Roma","customer_email":"orders@caferoma.test","external_ref":"PO-7781","lines":[{"item":"NW-ESP-250","quantity":10,"unit_price":5.5}]}' | jq .id)
test "$(code "$HUMAN" "sales-orders/$SO/erp")" = 422
curl "${H[@]}" -X POST "$API/sales-orders/$SO/confirm" >/dev/null
test "$(code "$AGENT" "sales-orders/$SO/erp")" = 403
ORDER=$(curl "${H[@]}" -X POST "$API/sales-orders/$SO/erp" | jq -r .external_id)
test "$(curl "${H[@]}" -X POST "$API/sales-orders/$SO/erp" | jq -r .external_id)" = "$ORDER"
dump '.holded.salesorder | map({invoiceNum, notes, items})'
test "$(dump '.holded.salesorder | length')" = 1 && test "$(dump '.holded.salesorder[0].invoiceNum')" = "\"SO-$SO\""
test "$(code "$HUMAN" "sales-orders/$SO/cancel")" = 409
jq -r .error "$WORK/body"

echo "--- a bad key: sync fails, error kept for humans, hidden from agents"
curl "${H[@]}" -X POST $API/erp -d '{"brand":"Northwind Coffee","kind":"holded","base_url":"http://127.0.0.1:18899","credential_env":"HOLDED_BAD_KEY"}' >/dev/null
test "$(curl -s -o /dev/null -w '%{http_code}' "${H[@]/-sf/-s}" -X POST $API/erp/sync -d '{"brand":"Northwind Coffee"}')" = 500
curl "${H[@]}" $API/erp | jq -c '.connections[0] | {last_sync_error}'
test "$(curl -sf -H "Authorization: Bearer $AGENT" $API/erp | jq -r '.connections[0].last_sync_error')" = "sync failed (details visible to human tokens)"
AGENT_SYNC=$(curl -s -H "Authorization: Bearer $AGENT" -H "Content-Type: application/json" -X POST $API/erp/sync -d '{"brand":"Northwind Coffee"}')
echo "agent sync: $AGENT_SYNC"; echo "$AGENT_SYNC" | grep -q "details visible to human tokens" && ! echo "$AGENT_SYNC" | grep -q 401
curl "${H[@]}" "$API/activity?limit=50" | jq -r '.[] | select(.action | startswith("erp")) | .summary' | head -5
echo "erp smoke OK"

#!/usr/bin/env bash
# Accounting smoke test: approved PO → QuickBooks bill, confirmed SO → invoice, then the same orders into Holded.
set -euo pipefail
cd "$(dirname "$0")/../server"
cargo build -q
BIN=target/debug
WORK=$(mktemp -d)
export BLUESTONE_DATABASE="sqlite://$WORK/smoke.db" SHOPIFY_DEV_TOKEN=shpat_dev_mock MOCK_SHOPIFY_ADDR=127.0.0.1:18888 \
  MOCK_ACCOUNTING_ADDR=127.0.0.1:18890 QBO_DEV_TOKEN=qbo_dev_mock HOLDED_DEV_KEY=holded_dev_mock QBO_BAD_TOKEN=expired
$BIN/mock_shopify & MOCK=$!
$BIN/mock_accounting & BOOKS=$!
trap 'kill $MOCK $BOOKS ${SRV:-} 2>/dev/null; rm -rf "$WORK"' EXIT
HUMAN=$($BIN/bluestone token create --name joan --kind human --scopes admin 2>/dev/null)
AGENT=$($BIN/bluestone token create --name smoke-agent 2>/dev/null)
$BIN/bluestone channel add --brand "Northwind Coffee" --kind shopify --name shopify-dev --base-url http://127.0.0.1:18888 --credential-env SHOPIFY_DEV_TOKEN >/dev/null
$BIN/bluestone accounting set --brand "Northwind Coffee" --kind quickbooks --base-url http://127.0.0.1:18890 --realm 9130 \
  --credential-env QBO_DEV_TOKEN --income-account 79 --expense-account 80
$BIN/bluestone serve --addr 127.0.0.1:18887 & SRV=$!
for _ in $(seq 50); do curl -s -o /dev/null http://127.0.0.1:18887/api/me && break; sleep 0.1; done
API=http://127.0.0.1:18887/api
H=(-sf -H "Authorization: Bearer $HUMAN" -H "Content-Type: application/json")
code() { curl -s -o "$WORK/body" -w '%{http_code}' -H "Authorization: Bearer $1" -H "Content-Type: application/json" -X POST "$API/$2" -d "${3:-{\}}"; }
books() { curl -sf http://127.0.0.1:18890/_dump | jq -c "$1"; }
curl "${H[@]}" -X POST $API/sync >/dev/null
ITEM=$(curl "${H[@]}" "$API/items?query=NW-ESP-250" | jq '.[0].id')
curl "${H[@]}" -X POST $API/items/bulk/supplier -d "{\"item_ids\":[$ITEM],\"supplier\":\"Roastery Co\",\"email\":\"orders@roastery.test\"}" >/dev/null

echo "--- PO → QuickBooks bill"
PO=$(curl "${H[@]}" -X POST $API/purchase-orders -d '{"brand":"Northwind Coffee","supplier":"Roastery Co","lines":[{"item":"NW-ESP-250","quantity":24,"unit_cost":4.1}]}' | jq .id)
echo "draft PO: HTTP $(code "$HUMAN" "purchase-orders/$PO/export")"; test "$(code "$HUMAN" "purchase-orders/$PO/export")" = 422
curl "${H[@]}" -X POST "$API/purchase-orders/$PO/approve" >/dev/null
echo "agent export: HTTP $(code "$AGENT" "purchase-orders/$PO/export")"; test "$(code "$AGENT" "purchase-orders/$PO/export")" = 403
BILL=$(curl "${H[@]}" -X POST "$API/purchase-orders/$PO/export" | jq -r .external_id)
test "$(curl "${H[@]}" -X POST "$API/purchase-orders/$PO/export" | jq -r .external_id)" = "$BILL"
books '.quickbooks | {vendors: [.Vendor[].DisplayName], items: [.Item[].Sku], bills: [.Bill[] | {DocNumber, TotalAmt}]}'
test "$(books '.quickbooks.Bill | length')" = 1 && test "$(books '.quickbooks.Bill[0].TotalAmt')" = 98.4
echo "cancel billed PO: HTTP $(code "$HUMAN" "purchase-orders/$PO/cancel")"; test "$(code "$HUMAN" "purchase-orders/$PO/cancel")" = 409
jq -r .error "$WORK/body"

echo "--- SO → QuickBooks invoice"
SO=$(curl "${H[@]}" -X POST $API/sales-orders -d '{"brand":"Northwind Coffee","customer":"Cafe Roma","customer_email":"orders@caferoma.test","external_ref":"PO-7781","lines":[{"item":"NW-ESP-250","quantity":10,"unit_price":5.5}]}' | jq .id)
test "$(code "$HUMAN" "sales-orders/$SO/export")" = 422
curl "${H[@]}" -X POST "$API/sales-orders/$SO/confirm" >/dev/null
curl "${H[@]}" -X POST "$API/sales-orders/$SO/export" | jq -c '{system, source, status, external_id}'
books '.quickbooks | {customers: [.Customer[].DisplayName], invoices: [.Invoice[] | {DocNumber, TotalAmt, PrivateNote}]}'
test "$(books '.quickbooks.Invoice[0].TotalAmt')" = 55 && test "$(books '.quickbooks.Item | length')" = 1

echo "--- reconnecting the same company keeps its exports; failures are recorded, hidden from agents, retryable"
curl "${H[@]}" -X POST $API/accounting -d '{"brand":"Northwind Coffee","kind":"quickbooks","realm":"9130","income_account":"79","expense_account":"80","base_url":"http://127.0.0.1:18890","credential_env":"QBO_BAD_TOKEN"}' >/dev/null
test "$(curl "${H[@]}" -X POST "$API/purchase-orders/$PO/export" | jq -r .external_id)" = "$BILL"
SO2=$(curl "${H[@]}" -X POST $API/sales-orders -d '{"brand":"Northwind Coffee","customer":"Bar Nou","lines":[{"item":"NW-ESP-250","quantity":2,"unit_price":5.5}]}' | jq .id)
curl "${H[@]}" -X POST "$API/sales-orders/$SO2/confirm" >/dev/null
test "$(code "$HUMAN" "sales-orders/$SO2/export")" = 500
curl "${H[@]}" $API/accounting | jq -c '.documents[0] | {status, error}'
test "$(curl "${H[@]}" $API/accounting | jq -r '.documents[0].status')" = failed
AGENT_ERR=$(curl -sf -H "Authorization: Bearer $AGENT" $API/accounting | jq -r '.documents[0].error')
echo "agent sees: $AGENT_ERR"; test "$AGENT_ERR" = "export failed (details visible to human tokens)"
curl "${H[@]}" -X POST $API/accounting -d '{"brand":"Northwind Coffee","kind":"quickbooks","realm":"9130","income_account":"79","expense_account":"80","base_url":"http://127.0.0.1:18890","credential_env":"QBO_DEV_TOKEN"}' >/dev/null
curl "${H[@]}" -X POST "$API/sales-orders/$SO2/export" | jq -c '{status, external_id}'
test "$(books '.quickbooks.Invoice | length')" = 2
test "$(code "$HUMAN" "sales-orders/$SO2/cancel")" = 409

echo "--- a new connection to the same books adopts the existing bill instead of creating another"
curl "${H[@]}" -X POST $API/accounting -d '{"brand":"Northwind Coffee","kind":"quickbooks","realm":"9130","income_account":"79","expense_account":"80","base_url":"http://localhost:18890","credential_env":"QBO_DEV_TOKEN"}' >/dev/null
test "$(curl "${H[@]}" -X POST "$API/purchase-orders/$PO/export" | jq -r .external_id)" = "$BILL"
test "$(books '.quickbooks.Bill | length')" = 1

echo "--- switch the brand to Holded and export the same orders"
curl "${H[@]}" -X POST $API/accounting -d '{"brand":"Northwind Coffee","kind":"holded","base_url":"http://127.0.0.1:18890","credential_env":"HOLDED_DEV_KEY"}' >/dev/null
curl "${H[@]}" -X POST "$API/purchase-orders/$PO/export" | jq -c '{system, status, external_id}'
curl "${H[@]}" -X POST "$API/sales-orders/$SO/export" | jq -c '{system, status, external_id}'
books '.holded | {contacts: [.contacts[] | {name, type}], purchase: [.purchase[] | {invoiceNum, items}], invoice: [.invoice[] | .invoiceNum]}'
test "$(books '.holded.contacts | length')" = 2 && test "$(books '.holded.purchase | length')" = 1 && test "$(books '.holded.invoice | length')" = 1
test "$(curl "${H[@]}" $API/accounting | jq '[.documents[] | select(.status == "exported")] | length')" = 6
curl "${H[@]}" "$API/activity?limit=50" | jq -r '.[] | select(.action | startswith("accounting")) | .summary' | head -5
echo "accounting smoke OK"

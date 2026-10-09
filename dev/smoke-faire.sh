#!/usr/bin/env bash
# Faire smoke test against the mock Faire API: sync → propose stock/price/status → approve → store updated.
set -euo pipefail
cd "$(dirname "$0")/../server"
cargo build -q
BIN=target/debug
WORK=$(mktemp -d)
export BLUESTONE_DATABASE="sqlite://$WORK/smoke.db" FAIRE_DEV_CREDENTIAL="ZGV2OmRldg==:faire_dev_mock" MOCK_FAIRE_ADDR=127.0.0.1:18789
$BIN/mock_faire & MOCK=$!
$BIN/bluestone serve --addr 127.0.0.1:18790 & SRV=$!
trap 'kill $MOCK $SRV 2>/dev/null; rm -rf "$WORK"' EXIT
sleep 1
HUMAN=$($BIN/bluestone token create --name joan --kind human --scopes admin 2>/dev/null)
$BIN/bluestone channel add --brand "Northwind Coffee" --kind faire --name faire-wholesale --base-url http://127.0.0.1:18789 --credential-env FAIRE_DEV_CREDENTIAL >/dev/null
API=http://127.0.0.1:18790/api
H=(-sf -H "Authorization: Bearer $HUMAN" -H "Content-Type: application/json")
FH=(-sf -H "X-FAIRE-APP-CREDENTIALS: ZGV2OmRldg==" -H "X-FAIRE-OAUTH-ACCESS-TOKEN: faire_dev_mock")
curl "${H[@]}" -X POST $API/sync | jq -c '.[] | {channel, listings, order_lines}'
# Deleted products are skipped; canceled orders and items are not counted as sales.
test "$(curl "${H[@]}" "$API/items" | jq length)" = 3
test "$(curl "${H[@]}" "$API/items?query=NW-ESP-250" | jq '.[0].sold_30d')" = 18
approve() { curl "${H[@]}" -X POST $API/proposals -d "$1" | jq -r .id | xargs -I{} curl "${H[@]}" -X POST $API/proposals/{}/approve | jq -c '{kind, status, error}'; }
approve '{"kind":"stock_adjustment","item":"NW-ESP-250","quantity":40,"rationale":"restock"}'
approve '{"kind":"price_change","item":"NW-ESP-250","price":7.25}'
approve '{"kind":"listing_status","item":"NW-GIFT-L","status":"draft"}'
INV=$(curl "${FH[@]}" "http://127.0.0.1:18789/external-api/v2/product-inventory/by-product-variant-ids?ids=v_esp" | jq '.inventories.v_esp.on_hand_quantity.quantity')
PRICE=$(curl "${FH[@]}" http://127.0.0.1:18789/external-api/v2/products/p_esp | jq '.variants[0].prices[0].wholesale_price.amount_minor')
STATE=$(curl "${FH[@]}" http://127.0.0.1:18789/external-api/v2/products/p_gift | jq -r .lifecycle_state)
echo "faire store: on_hand=$INV wholesale_minor=$PRICE gift_box=$STATE"
test "$INV" = 40 && test "$PRICE" = 725 && test "$STATE" = UNPUBLISHED

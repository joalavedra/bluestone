#!/usr/bin/env bash
# Settings → Integrations smoke test: catalog, test before save, pasted keys stored encrypted, env refs, agent refusal.
set -euo pipefail
cd "$(dirname "$0")/../server"
cargo build -q
BIN=target/debug
WORK=$(mktemp -d)
export BLUESTONE_DATABASE="sqlite://$WORK/smoke.db" MOCK_SHOPIFY_ADDR=127.0.0.1:18988 MOCK_ACCOUNTING_ADDR=127.0.0.1:18990 \
  HOLDED_DEV_KEY=holded_dev_mock BLUESTONE_SECRET_KEY=smoke-only-key
$BIN/mock_shopify & MOCK=$!
$BIN/mock_accounting & BOOKS=$!
trap 'kill $MOCK $BOOKS ${SRV:-} 2>/dev/null; rm -rf "$WORK"' EXIT
HUMAN=$($BIN/bluestone token create --name joan --kind human --scopes admin 2>/dev/null)
AGENT=$($BIN/bluestone token create --name smoke-agent 2>/dev/null)
$BIN/bluestone serve --addr 127.0.0.1:18987 & SRV=$!
for _ in $(seq 50); do curl -s -o /dev/null http://127.0.0.1:18987/api/me && break; sleep 0.1; done
API=http://127.0.0.1:18987/api
H=(-sf -H "Authorization: Bearer $HUMAN" -H "Content-Type: application/json")
code() { curl -s -o "$WORK/body" -w '%{http_code}' -H "Authorization: Bearer $1" -H "Content-Type: application/json" -X POST "$API/$2" -d "${3:-{\}}"; }
try() { curl "${H[@]}" -X POST $API/integrations/test -d "$1"; }
status() { curl "${H[@]}" $API/integrations | jq -r ".connectors[] | select(.id == \"$1\") | .status"; }

echo "--- catalog"
curl "${H[@]}" $API/integrations | jq -c '[.connectors[] | "\(.id):\(.status)"]'
test "$(status shopify)" = available && test "$(status quickbooks)" = available && test "$(status tiktok_shop)" = coming_soon

SHOP='{"connector":"shopify","brand":"Northwind Coffee","fields":{"name":"shopify-dev","base_url":"http://127.0.0.1:18988"},"secret":"shpat_dev_mock"}'
echo "agent test: HTTP $(code "$AGENT" integrations/test "$SHOP")"; test "$(code "$AGENT" integrations/test "$SHOP")" = 403
test "$(code "$AGENT" integrations "$SHOP")" = 403
test "$(code "$HUMAN" integrations "$(jq -c 'del(.fields.base_url)' <<<"$SHOP")")" = 422 && jq -r .error "$WORK/body"
test "$(code "$HUMAN" integrations '{"connector":"tiktok_shop","brand":"Northwind Coffee"}')" = 422 && jq -r .error "$WORK/body"

echo "--- test before save"
try "$(jq -c '.secret = "wrong"' <<<"$SHOP")" | jq -c .
test "$(try "$(jq -c '.secret = "wrong"' <<<"$SHOP")" | jq .ok)" = false
try "$SHOP" | jq -c .
test "$(try "$SHOP" | jq .ok)" = true
test "$(curl "${H[@]}" $API/integrations | jq '.connections | length')" = 0

echo "--- save with a pasted key: stored encrypted, never returned, sync uses it"
curl "${H[@]}" -X POST $API/integrations -d "$SHOP" | jq -c .
curl "${H[@]}" -X POST $API/sync | jq -c '.[] | {channel, listings}'
test "$(curl "${H[@]}" "$API/items" | jq length)" -gt 0
curl "${H[@]}" $API/integrations | jq -c '.connections[] | {connector, brand, label: .label, credential_source}'
test "$(curl "${H[@]}" $API/integrations | jq -r '.connections[0].credential_source')" = stored
! curl "${H[@]}" $API/integrations | grep -q shpat_dev_mock
! grep -aq shpat_dev_mock "$WORK"/smoke.db*
echo "edit without a key keeps the stored one: $(try "$(jq -c 'del(.secret)' <<<"$SHOP")" | jq -c .)"
test "$(try "$(jq -c 'del(.secret)' <<<"$SHOP")" | jq .ok)" = true
test "$(code "$HUMAN" integrations/test "$(jq -c '.credential_ref = "X"' <<<"$SHOP")")" = 422 && jq -r .error "$WORK/body"
test "$(code "$HUMAN" integrations/test "$(jq -c 'del(.secret) | .credential_ref = "secret:1"' <<<"$SHOP")")" = 422
test "$(code "$HUMAN" integrations/test "$(jq -c 'del(.secret) | .fields.base_url = "http://127.0.0.1:9"' <<<"$SHOP")")" = 422 && jq -r .error "$WORK/body"
secrets() { python3 -c "import sqlite3; print(sqlite3.connect('$WORK/smoke.db').execute('SELECT group_concat(id) FROM secrets').fetchone()[0])"; }
curl "${H[@]}" -X POST $API/integrations -d "$SHOP" >/dev/null
echo "re-saved with a new key, stored keys: $(secrets)"; test "$(secrets)" = 2

echo "--- QuickBooks with a pasted token, Holded through an env var"
QB='{"connector":"quickbooks","brand":"Northwind Coffee","fields":{"realm":"9130","income_account":"79","expense_account":"80","base_url":"http://127.0.0.1:18990"},"secret":"qbo_dev_mock"}'
test "$(code "$HUMAN" integrations "$(jq -c 'del(.fields.realm)' <<<"$QB")")" = 422 && jq -r .error "$WORK/body"
try "$QB" | jq -c .
test "$(try "$QB" | jq -r .message)" = "QuickBooks Online: connected to Mock Company 9130"
curl "${H[@]}" -X POST $API/integrations -d "$QB" | jq -c .
test "$(curl "${H[@]}" $API/accounting | jq -r '.connections[0].kind')" = quickbooks
HOLDED='{"connector":"holded","brand":"Northwind Coffee","fields":{"base_url":"http://127.0.0.1:18990"},"credential_ref":"HOLDED_DEV_KEY"}'
try "$HOLDED" | jq -c .
test "$(try "$HOLDED" | jq .ok)" = true
curl "${H[@]}" -X POST $API/integrations -d "$HOLDED" >/dev/null
curl "${H[@]}" $API/integrations | jq -c '[.connections[] | {connector, credential_source}]'
test "$(curl "${H[@]}" $API/integrations | jq -r '.connections[] | select(.kind == "accounting") | .credential_source')" = env

echo "--- Faire without a key is saved for the OAuth connect flow"
curl "${H[@]}" -X POST $API/integrations -d '{"connector":"faire","brand":"Northwind Coffee","fields":{"name":"faire-wholesale"}}' | jq -c .
test "$(curl "${H[@]}" $API/integrations | jq -r '.connections[] | select(.connector == "faire") | .credential_source')" = oauth

echo "--- requesting a missing connector is logged"
curl "${H[@]}" -X POST $API/integrations/request -d '{"connector":"tiktok_shop"}' >/dev/null
curl "${H[@]}" "$API/activity?limit=20" | jq -r '.[] | select(.action == "integration_request") | .summary'
test "$(curl "${H[@]}" "$API/activity?limit=20" | jq '[.[] | select(.action == "integration_request")] | length')" = 1
echo "integrations smoke OK"

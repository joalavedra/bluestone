#!/usr/bin/env bash
# Enables the PrestaShop WebService in the dev shop and creates a key with GET/PUT on the resources Bluestone uses.
# Prints the key; export it as PRESTASHOP_DEV_KEY.
set -euo pipefail
KEY=${1:-$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n' | tr a-f A-F)}
SQL="
UPDATE ps_configuration SET value='1' WHERE name='PS_WEBSERVICE';
INSERT INTO ps_configuration (name, value, date_add, date_upd) SELECT 'PS_WEBSERVICE','1',NOW(),NOW() FROM DUAL WHERE NOT EXISTS (SELECT 1 FROM ps_configuration WHERE name='PS_WEBSERVICE');
INSERT INTO ps_webservice_account (\`key\`, description, class_name, is_module, active) VALUES ('$KEY','bluestone','WebserviceRequest',0,1);
SET @id = LAST_INSERT_ID();
INSERT INTO ps_webservice_account_shop (id_webservice_account, id_shop) VALUES (@id, 1);
INSERT INTO ps_webservice_permission (resource, method, id_webservice_account)
SELECT r, m, @id FROM (SELECT 'products' r UNION SELECT 'combinations' UNION SELECT 'stock_availables' UNION SELECT 'orders' UNION SELECT 'order_details' UNION SELECT 'images') res
CROSS JOIN (SELECT 'GET' m UNION SELECT 'PUT' UNION SELECT 'HEAD') meth;"
docker compose -f "$(dirname "$0")/docker-compose.yml" exec -T mariadb mariadb -uroot -pprestashop prestashop -e "$SQL"
echo "$KEY"

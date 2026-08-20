# explorer.kovanica.online (nginx + certbot)

Wallet needs HTTPS. Node stays on `127.0.0.1:8080`.

## Cloudflare DNS

`explorer` currently resolves to the **same Cloudflare IPs as the apex** — that is a CNAME onto `kovanica.online`, so you get the old site, not the DAG explorer.

Change it to:

| Type | Name | Content | Proxy |
| --- | --- | --- | --- |
| A | `explorer` | VPS IPv4 (`curl -4 ifconfig.me`) | **DNS only** (grey cloud) |

Grey cloud is required for certbot HTTP-01. After the cert exists you can orange-cloud and set SSL to Full (strict).

## VPS

```bash
cd ~/kovanica-ledger
cargo build --release -p kovanica-node

apt-get update
apt-get install -y nginx certbot python3-certbot-nginx
install -m 644 deploy/kovanica-explorer.service /etc/systemd/system/kovanica-explorer.service
install -m 644 deploy/nginx-explorer.conf /etc/nginx/sites-available/explorer.kovanica.online
ln -sfn /etc/nginx/sites-available/explorer.kovanica.online /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
systemctl daemon-reload
systemctl enable --now kovanica-explorer
ufw allow 80,443/tcp || true

certbot --nginx -d explorer.kovanica.online
```

Native token is **Kovanica (KVNC)**. Supply grows when the node produces an empty block (subsidy coinbase to the miner). Genesis is the premine.

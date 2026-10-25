#!/bin/sh
# Puts the website on <sub>.auneu.space: Cloudflare (the record proxied, orange) in front of the
# German VPS, whose nginx passes the site through an SSH tunnel to the game server's 8080 (open
# on its localhost only). Cloudflare reaches the VPS either way round, so both are served:
#   SSL "Flexible": plain http on port 80;
#   SSL "Full": 443, where xray takes TLS off and hands the rest to nginx on 127.0.0.1:10443.
# Cloudflare sends visitors to https itself ("Always Use HTTPS").
#   server/deploy/setup-site.sh <sub> [vps ssh alias, default: vps]      e.g. setup-site.sh rstaiko
# Safe to run again.
set -e
sub=${1:?usage: setup-site.sh <subdomain, e.g. rstaiko> [vps]}
vps=${2:-vps}
host="$sub.auneu.space"
here=$(cd "$(dirname "$0")" && pwd)

# 1. The game server: the site on its localhost, the visitor's address from nginx.
"$here/deploy-via-vps.sh" "$vps"

# 2. The VPS: tunnel and nginx.
ssh "$vps" sh -s "$host" <<'EOF'
set -e
host=$1
stamp=$(date +%Y%m%d%H%M)

# The tunnel, a copy of the timing one on another port.
sudo systemctl cat osu-timing-tunnel.service | grep -v '^#' \
    | sed -e 's/127.0.0.1:18431:127.0.0.1:8431/127.0.0.1:18080:127.0.0.1:8080/' -e 's/^Description=.*/Description=SSH tunnel to the KZ server: taiko-rs website/' \
    | sudo tee /etc/systemd/system/taiko-site-tunnel.service >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --now taiko-site-tunnel.service
sudo systemctl restart taiko-site-tunnel.service

# Cloudflare's addresses (theirs to trust for who the visitor is), today's list if it loads.
ips=$( (curl -sf --max-time 10 https://www.cloudflare.com/ips-v4; echo; curl -sf --max-time 10 https://www.cloudflare.com/ips-v6) | grep -E '^[0-9a-f:.]+/[0-9]+$' || true)
if [ "$(echo "$ips" | grep -c /)" -lt 10 ]; then
    ips="173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 104.24.0.0/14 172.64.0.0/13 131.0.72.0/22 2400:cb00::/32 2606:4700::/32 2803:f800::/32 2405:b500::/32 2405:8100::/32 2a06:98c0::/29 2c0f:f248::/32"
fi
for r in $ips; do echo "$r 1;"; done | sudo tee /etc/nginx/taiko-cf-ips.conf >/dev/null

# nginx: the site's own file, included once from nginx.conf.
sudo tee /etc/nginx/taiko-site.conf >/dev/null <<'CONF'
# taiko-rs website (server/deploy/setup-site.sh in taiko-rs).
# Requests from Cloudflare carry the visitor's address; anyone else is who they are.
geo $remote_addr $taiko_cf_plain {
    default 0;
    include /etc/nginx/taiko-cf-ips.conf;
}
geo $proxy_protocol_addr $taiko_cf_tls {
    default 0;
    include /etc/nginx/taiko-cf-ips.conf;
}
# Per visitor: 5 requests a second, bursts of 30.
limit_req_zone $taiko_visitor zone=taiko_site:10m rate=5r/s;

# Cloudflare with SSL "Flexible".
server {
    listen 80;
    listen [::]:80;
    server_name HOST;
    set $taiko_visitor $remote_addr;
    if ($taiko_cf_plain) {
        set $taiko_visitor $http_cf_connecting_ip;
    }
    location / {
        include /etc/nginx/taiko-site-proxy.conf;
    }
}
# Cloudflare with SSL "Full", through xray.
server {
    listen 127.0.0.1:10443 proxy_protocol;
    server_name HOST;
    set $taiko_visitor $proxy_protocol_addr;
    if ($taiko_cf_tls) {
        set $taiko_visitor $http_cf_connecting_ip;
    }
    location / {
        include /etc/nginx/taiko-site-proxy.conf;
    }
}
CONF
sudo tee /etc/nginx/taiko-site-proxy.conf >/dev/null <<'CONF'
limit_req zone=taiko_site burst=30 nodelay;
limit_req_status 429;
proxy_pass http://127.0.0.1:18080;
proxy_set_header Host $host;
# The visitor, for the site's own limits (whatever they sent in these is replaced).
proxy_set_header CF-Connecting-IP $taiko_visitor;
proxy_set_header X-Forwarded-For $taiko_visitor;
proxy_connect_timeout 5s;
proxy_read_timeout 15s;
CONF
sudo sed -i "s/HOST/$host/g" /etc/nginx/taiko-site.conf
if ! grep -q taiko-site.conf /etc/nginx/nginx.conf; then
    sudo cp /etc/nginx/nginx.conf "/etc/nginx/nginx.conf.bak-taiko-site-$stamp"
    sudo sed -i '0,/^http {/s//http {\n    include \/etc\/nginx\/taiko-site.conf;/' /etc/nginx/nginx.conf
fi
sudo nginx -t
sudo systemctl reload nginx

sleep 3
echo "--- through the tunnel:"
curl -s --max-time 10 http://127.0.0.1:18080/api/rankings | head -c 300
echo
echo "--- through Cloudflare:"
curl -s --max-time 15 -o /dev/null -w "%{http_code}\n" "https://$host/"
EOF
echo "https://$host"

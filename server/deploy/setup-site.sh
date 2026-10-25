#!/bin/sh
# Puts the website on <sub>.auneu.space, served by the German VPS like its other subdomains:
# xray on 443 takes TLS off with the auneu.space certificate and hands the rest to nginx on
# 127.0.0.1:10443, which passes the site through an SSH tunnel to the game server's 8080 (open
# on its localhost only).
#   server/deploy/setup-site.sh <sub> [vps ssh alias, default: vps]      e.g. setup-site.sh top
# Needs an A record <sub>.auneu.space -> the VPS first. Safe to run again.
set -e
sub=${1:?usage: setup-site.sh <subdomain, e.g. top> [vps]}
vps=${2:-vps}
host="$sub.auneu.space"
here=$(cd "$(dirname "$0")" && pwd)

# 1. The game server: the site on its localhost, the visitor's address from nginx.
"$here/deploy-via-vps.sh" "$vps"

# 2. The VPS: tunnel, nginx, certificate.
ssh "$vps" sh -s "$host" <<'EOF'
set -e
host=$1
ip=$(curl -s4 --max-time 10 ifconfig.me || true)
if ! getent hosts "$host" | grep -q "$ip"; then
    echo "$host doesn't point at this VPS ($ip) yet: add the A record and try again"
    exit 1
fi
stamp=$(date +%Y%m%d%H%M)

# The tunnel, a copy of the timing one on another port.
sudo systemctl cat osu-timing-tunnel.service | grep -v '^#' \
    | sed -e 's/127.0.0.1:18431:127.0.0.1:8431/127.0.0.1:18080:127.0.0.1:8080/' -e 's/^Description=.*/Description=SSH tunnel to the KZ server: taiko-rs website/' \
    | sudo tee /etc/systemd/system/taiko-site-tunnel.service >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --now taiko-site-tunnel.service
sudo systemctl restart taiko-site-tunnel.service

# nginx: the site's own file, included once from nginx.conf.
sudo tee /etc/nginx/taiko-site.conf >/dev/null <<'CONF'
# taiko-rs website (server/deploy/setup-site.sh in taiko-rs).
# Per visitor (their address from xray's proxy protocol): 5 requests a second, bursts of 30.
limit_req_zone $proxy_protocol_addr zone=taiko_site:10m rate=5r/s;
server {
    listen 80;
    listen [::]:80;
    server_name HOST;
    location / {
        return 301 https://$host$request_uri;
    }
}
server {
    listen 127.0.0.1:10443 proxy_protocol;
    server_name HOST;
    location / {
        limit_req zone=taiko_site burst=30 nodelay;
        limit_req_status 429;
        proxy_pass http://127.0.0.1:18080;
        proxy_set_header Host $host;
        # The visitor, for the site's own limits (whatever they sent in these is replaced).
        proxy_set_header CF-Connecting-IP $proxy_protocol_addr;
        proxy_set_header X-Forwarded-For $proxy_protocol_addr;
        proxy_connect_timeout 5s;
        proxy_read_timeout 15s;
    }
}
CONF
sudo sed -i "s/HOST/$host/g" /etc/nginx/taiko-site.conf
if ! grep -q taiko-site.conf /etc/nginx/nginx.conf; then
    sudo cp /etc/nginx/nginx.conf "/etc/nginx/nginx.conf.bak-taiko-site-$stamp"
    sudo sed -i '0,/^http {/s//http {\n    include \/etc\/nginx\/taiko-site.conf;/' /etc/nginx/nginx.conf
fi
sudo nginx -t
sudo systemctl reload nginx

# The certificate xray serves gets the new name too, then xray loads it (the VPN blinks).
names=$(sudo openssl x509 -in /etc/letsencrypt/live/auneu.space/fullchain.pem -noout -ext subjectAltName | tail -1 | tr -d ' ' | sed 's/DNS://g')
case ",$names," in
    *",$host,"*) echo "the certificate already has $host" ;;
    *)
        sudo certbot certonly --nginx --non-interactive --cert-name auneu.space --expand -d "$names,$host"
        sudo systemctl restart x-ui
        ;;
esac

sleep 3
echo "--- https://$host/api/rankings:"
curl -s --max-time 10 --resolve "$host:443:127.0.0.1" "https://$host/api/rankings" | head -c 300
echo
EOF
echo "https://$host"

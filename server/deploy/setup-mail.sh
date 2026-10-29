#!/bin/sh
# Mail for the game server's codes (sign-up, linking an email, password reset), sent from the
# German VPS as noreply@auneu.space:
#   - Postfix that only sends (listens on 127.0.0.1, takes no mail from outside), with OpenDKIM
#     signing for auneu.space (selector "mail");
#   - taiko-mailer: takes the server's letters from /api/mail/next through the site's tunnel
#     (127.0.0.1:18080, see setup-site.sh) and hands them to Postfix;
#   - the mailer's key, on the VPS and in the game server's data/mail-key (mail is on from then:
#     new accounts need an email).
# Prints the DNS records auneu.space needs (SPF, DKIM, DMARC). The PTR of the VPS's address
# (mail.auneu.space) is set at the host, NuxtCloud. Safe to run again.
#   server/deploy/setup-mail.sh [vps ssh alias, default: vps]
set -e
vps=${1:-vps}

ssh "$vps" sh -s <<'EOF'
set -e
ip=$(curl -s4 --max-time 10 ifconfig.me || hostname -I | cut -d' ' -f1)

# 1. Postfix and OpenDKIM.
if ! command -v postfix >/dev/null 2>&1 || ! command -v opendkim >/dev/null 2>&1; then
    echo "postfix postfix/mailname string auneu.space" | sudo debconf-set-selections
    echo "postfix postfix/main_mailer_type select Internet Site" | sudo debconf-set-selections
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q postfix opendkim opendkim-tools >/dev/null
fi
echo auneu.space | sudo tee /etc/mailname >/dev/null
sudo postconf -e \
    myhostname=mail.auneu.space \
    mydomain=auneu.space \
    myorigin=auneu.space \
    inet_interfaces=loopback-only \
    inet_protocols=ipv4 \
    mydestination= \
    relayhost= \
    smtp_tls_security_level=may \
    smtp_tls_loglevel=1 \
    milter_default_action=accept \
    milter_protocol=6 \
    smtpd_milters=inet:127.0.0.1:8891 \
    non_smtpd_milters=inet:127.0.0.1:8891

sudo mkdir -p /etc/opendkim/keys/auneu.space
if [ ! -f /etc/opendkim/keys/auneu.space/mail.private ]; then
    sudo opendkim-genkey -b 2048 -d auneu.space -s mail -D /etc/opendkim/keys/auneu.space
fi
sudo chown -R opendkim:opendkim /etc/opendkim/keys
sudo chmod 600 /etc/opendkim/keys/auneu.space/mail.private
[ -f /etc/opendkim.conf.bak-taiko ] || sudo cp /etc/opendkim.conf /etc/opendkim.conf.bak-taiko
sudo tee /etc/opendkim.conf >/dev/null <<'CONF'
# taiko.rs mail (server/deploy/setup-mail.sh in taiko-rs): signs what Postfix sends.
Syslog                  yes
UMask                   007
Mode                    s
Domain                  auneu.space
Selector                mail
KeyFile                 /etc/opendkim/keys/auneu.space/mail.private
Canonicalization        relaxed/simple
SignatureAlgorithm      rsa-sha256
OversignHeaders         From
Socket                  inet:8891@127.0.0.1
UserID                  opendkim
PidFile                 /run/opendkim/opendkim.pid
CONF
sudo systemctl enable -q opendkim postfix
sudo systemctl restart opendkim postfix

# 2. The mailer.
sudo mkdir -p /etc/taiko-mailer
if [ ! -s /etc/taiko-mailer/key ]; then
    openssl rand -hex 24 | sudo tee /etc/taiko-mailer/key >/dev/null
fi
sudo chmod 600 /etc/taiko-mailer/key
sudo tee /usr/local/bin/taiko-mailer >/dev/null <<'MAILER'
#!/bin/sh
# Takes the game server's letters one at a time and hands them to Postfix
# (server/deploy/setup-mail.sh in taiko-rs).
key=$(cat /etc/taiko-mailer/key)
letter=$(mktemp)
trap 'rm -f "$letter"' EXIT
while :; do
    if curl -sf --max-time 10 -H "Authorization: Bearer $key" -o "$letter" http://127.0.0.1:18080/api/mail/next && [ -s "$letter" ]; then
        /usr/sbin/sendmail -t -i -f noreply@auneu.space <"$letter" || echo "sendmail failed" >&2
        continue
    fi
    sleep 3
done
MAILER
sudo chmod 755 /usr/local/bin/taiko-mailer
sudo tee /etc/systemd/system/taiko-mailer.service >/dev/null <<'UNIT'
[Unit]
Description=taiko.rs mailer: the game server's letters to Postfix
After=network-online.target postfix.service taiko-site-tunnel.service

[Service]
ExecStart=/usr/local/bin/taiko-mailer
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable -q --now taiko-mailer.service
sudo systemctl restart taiko-mailer.service

# 3. The key on the game server, past the last hop (as deploy-via-vps.sh does it).
target=$(systemctl cat osu-timing-tunnel.service | grep -o "[^ ]*@jashincraft.space" | head -1)
sudo cat /etc/taiko-mailer/key >/tmp/mail-key
scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/mail-key "$target:taiko-server/data/mail-key"
rm -f /tmp/mail-key
ssh -F /dev/null -p 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes "$target" chmod 600 taiko-server/data/mail-key

# 4. What DNS needs.
dkim=$(sudo cat /etc/opendkim/keys/auneu.space/mail.txt | tr -d '\n' | grep -o '"[^"]*"' | tr -d '" \t\n')
echo
echo "== DNS records for auneu.space (Cloudflare, all grey / DNS only) =="
echo "A     mail                 $ip"
echo "TXT   auneu.space          v=spf1 ip4:$ip -all"
echo "TXT   mail._domainkey      $dkim"
echo "TXT   _dmarc               v=DMARC1; p=none; adkim=s; aspf=s"
echo "PTR   $ip -> mail.auneu.space  (at NuxtCloud: IP-адреса -> Домен)"
echo
echo "PTR now: $(getent hosts "$ip" | awk '{print $2}')"
systemctl is-active postfix opendkim taiko-mailer | paste -sd' '
EOF

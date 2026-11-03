#!/bin/sh
# Sends the game server's codes through Resend instead of straight from the VPS (whose host network
# is on Spamhaus ASN-DROP, so Hotmail refuses it). Puts the Resend API key into Postfix's
# /etc/postfix/sasl_passwd on the VPS without printing it, then reruns setup-mail.sh, which sees
# the file and switches Postfix to the relay. auneu.space must already be verified in Resend.
#   server/deploy/setup-mail-relay.sh [key file, default: ~/.config/resend/key] [vps alias, default: vps]
set -e
key=${1:-$HOME/.config/resend/key}
vps=${2:-vps}
[ -s "$key" ] || { echo "no Resend key in $key" >&2; exit 1; }

printf '[smtp.resend.com]:587 resend:%s\n' "$(tr -d '\n' <"$key")" |
    ssh "$vps" 'sudo sh -c "umask 077; cat >/etc/postfix/sasl_passwd"'
"$(dirname "$0")/setup-mail.sh" "$vps"
ssh "$vps" 'postconf relayhost smtp_tls_security_level non_smtpd_milters'

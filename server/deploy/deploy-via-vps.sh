#!/bin/sh
# Builds the static server and (re)starts it on jashincraft.space, going through the German VPS:
# the key for the last hop lives on the VPS (~/.ssh/xui_node), so `deploy.sh ... vps` can't do it.
#   server/deploy/deploy-via-vps.sh [vps ssh alias, default: vps]
# The running binary stays as taiko-server.bak; to roll back, copy it over taiko-server and run
# the last step again.
set -e
vps=${1:-vps}
here=$(cd "$(dirname "$0")" && pwd)
bin="$here/../target/x86_64-unknown-linux-musl/release/taiko-server"
# ring (TLS for the Discord bot) has C in it: clang builds it for musl without a musl gcc.
export CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-clang}"
export CFLAGS_x86_64_unknown_linux_musl="${CFLAGS_x86_64_unknown_linux_musl:---target=x86_64-unknown-linux-musl}"
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$here/../Cargo.toml"
sum=$(sha256sum "$bin" | cut -d' ' -f1)
scp -q "$bin" "$vps:/tmp/taiko-server.new"
scp -q "$here/Dockerfile" "$here/docker-compose.yml" "$vps:/tmp/"
# Claude's key for the Discord pings (/api/claude/...), when this machine has one.
key=${CLAUDE_KEY_FILE:-$HOME/.config/claude-discord/key}
if [ -f "$key" ]; then scp -q "$key" "$vps:/tmp/claude-key"; else ssh "$vps" rm -f /tmp/claude-key; fi
# The osu! API client for map set statuses (two lines: client ID, secret), when this machine has one.
osu=${OSU_API_FILE:-$HOME/.config/taiko-server/osu-api}
if [ -f "$osu" ]; then scp -q "$osu" "$vps:/tmp/osu-api"; else ssh "$vps" rm -f /tmp/osu-api; fi
# Plays worked out again (`taiko-rs --recalc-pp`), when this machine has them.
recalc=${RECALC_FILE:-$HOME/.config/taiko-server/recalculated.json}
if [ -f "$recalc" ]; then scp -q "$recalc" "$vps:/tmp/recalculated.json"; else ssh "$vps" rm -f /tmp/recalculated.json; fi
# The certificate for the encrypted game port and its key, when this machine has them.
tls=${TLS_DIR:-$HOME/.config/taiko-server}
if [ -f "$tls/tls-key.der" ]; then scp -q "$tls/tls-cert.der" "$tls/tls-key.der" "$vps:/tmp/"; else ssh "$vps" rm -f /tmp/tls-key.der /tmp/tls-cert.der; fi
# On the VPS: the account for the last hop is the one its tunnel service uses.
ssh "$vps" sh -s "$sum" <<'EOF'
set -e
target=$(systemctl cat osu-timing-tunnel.service | grep -o "[^ ]*@jashincraft.space" | head -1)
hop="ssh -F /dev/null -p 1337 -i $HOME/.ssh/xui_node -o IdentitiesOnly=yes"
scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/taiko-server.new /tmp/Dockerfile /tmp/docker-compose.yml "$target:taiko-server/"
if [ -f /tmp/claude-key ]; then
    scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/claude-key "$target:taiko-server/data/claude-key"
    rm -f /tmp/claude-key
    $hop "$target" chmod 600 taiko-server/data/claude-key
fi
if [ -f /tmp/osu-api ]; then
    scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/osu-api "$target:taiko-server/data/osu-api"
    rm -f /tmp/osu-api
    $hop "$target" chmod 600 taiko-server/data/osu-api
fi
if [ -f /tmp/recalculated.json ]; then
    scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/recalculated.json "$target:taiko-server/data/recalculated.json"
    rm -f /tmp/recalculated.json
fi
if [ -f /tmp/tls-key.der ]; then
    scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/tls-cert.der /tmp/tls-key.der "$target:taiko-server/data/"
    rm -f /tmp/tls-cert.der /tmp/tls-key.der
    $hop "$target" chmod 600 taiko-server/data/tls-key.der
fi
$hop "$target" "sh -s $1" <<'REMOTE'
set -e
cd ~/taiko-server
[ "$(sha256sum taiko-server.new | cut -d' ' -f1)" = "$1" ] || { echo "upload damaged"; exit 1; }
cp taiko-server taiko-server.bak
mv taiko-server.new taiko-server
if docker compose version >/dev/null 2>&1; then c="docker compose"; else c=docker-compose; fi
$c build -q && docker rm -f taiko-server >/dev/null 2>&1; $c up -d
sleep 2
docker logs --tail 5 taiko-server
sha256sum taiko-server
REMOTE
EOF
echo "deployed $sum"

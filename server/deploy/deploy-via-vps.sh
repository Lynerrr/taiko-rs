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
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$here/../Cargo.toml"
sum=$(sha256sum "$bin" | cut -d' ' -f1)
scp -q "$bin" "$vps:/tmp/taiko-server.new"
# On the VPS: the account for the last hop is the one its tunnel service uses.
ssh "$vps" sh -s "$sum" <<'EOF'
set -e
target=$(systemctl cat osu-timing-tunnel.service | grep -o "[^ ]*@jashincraft.space" | head -1)
hop="ssh -F /dev/null -p 1337 -i $HOME/.ssh/xui_node -o IdentitiesOnly=yes"
scp -q -F /dev/null -P 1337 -i ~/.ssh/xui_node -o IdentitiesOnly=yes /tmp/taiko-server.new "$target:taiko-server/taiko-server.new"
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

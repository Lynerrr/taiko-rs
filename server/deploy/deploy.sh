#!/bin/sh
# Builds the static server and (re)starts it on the server as a Docker stack in ~/taiko-server.
#   server/deploy/deploy.sh user@host [ssh port]
set -e
target=${1:?usage: deploy.sh user@host [ssh port]}
port=${2:-22}
here=$(cd "$(dirname "$0")" && pwd)
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$here/../Cargo.toml"
ssh -p "$port" "$target" 'mkdir -p ~/taiko-server/data'
scp -P "$port" "$here/../target/x86_64-unknown-linux-musl/release/taiko-server" "$here/Dockerfile" "$here/docker-compose.yml" "$target:taiko-server/"
# Compose v2 (`docker compose`) or the older standalone `docker-compose`.
ssh -p "$port" "$target" 'cd ~/taiko-server && if docker compose version >/dev/null 2>&1; then c="docker compose"; else c=docker-compose; fi && $c up -d --build && $c ps'

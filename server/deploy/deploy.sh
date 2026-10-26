#!/bin/sh
# Builds the static server and (re)starts it on the server as a Docker stack in ~/taiko-server.
#   server/deploy/deploy.sh user@host [ssh port] [jump host]
# A jump host (anything `ssh -J` takes, e.g. an alias from ~/.ssh/config) goes through it to
# the server, for when the server's ssh port isn't reachable from here directly.
set -e
target=${1:?usage: deploy.sh user@host [ssh port] [jump host]}
port=${2:-22}
jump=${3:+-J $3}
here=$(cd "$(dirname "$0")" && pwd)
# ring (TLS for the Discord bot) has C in it: clang builds it for musl without a musl gcc.
export CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-clang}"
export CFLAGS_x86_64_unknown_linux_musl="${CFLAGS_x86_64_unknown_linux_musl:---target=x86_64-unknown-linux-musl}"
cargo build --release --target x86_64-unknown-linux-musl --manifest-path "$here/../Cargo.toml"
ssh $jump -p "$port" "$target" 'mkdir -p ~/taiko-server/data'
# The running binary stays as taiko-server.bak, to roll back to.
ssh $jump -p "$port" "$target" 'cd ~/taiko-server && if [ -f taiko-server ]; then cp taiko-server taiko-server.bak; fi'
scp $jump -P "$port" "$here/../target/x86_64-unknown-linux-musl/release/taiko-server" "$here/Dockerfile" "$here/docker-compose.yml" "$target:taiko-server/"
# Compose v2 (`docker compose`) or the older standalone `docker-compose`. The image is built
# first and the old container removed before starting the new one: docker-compose 1.29 can't
# recreate containers on newer Docker (KeyError: 'ContainerConfig'), so downtime is a moment.
ssh $jump -p "$port" "$target" 'cd ~/taiko-server && if docker compose version >/dev/null 2>&1; then c="docker compose"; else c=docker-compose; fi && $c build && docker rm -f taiko-server >/dev/null 2>&1; $c up -d && $c ps'

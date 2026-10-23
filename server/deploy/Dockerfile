# The static (musl) taiko-server binary in an empty image. Build the binary first:
#   cargo build --release --target x86_64-unknown-linux-musl   (in server/)
# and put it next to this file as `taiko-server`.
FROM scratch
COPY taiko-server /taiko-server
EXPOSE 7800
ENTRYPOINT ["/taiko-server", "--port", "7800", "--data", "/data"]

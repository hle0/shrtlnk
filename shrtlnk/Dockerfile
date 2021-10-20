FROM rust:1.55-buster as builder
WORKDIR /usr/src/shrtlnk
COPY . .
RUN cargo install --path .

FROM debian:buster-slim
COPY --from=builder /usr/local/cargo/bin/shrtlnk /usr/local/bin/shrtlnk
WORKDIR /data
VOLUME [ "/data" ]
CMD [ "shrtlnk", "/data/config.toml" ]
FROM rust:1.99.1-trixie AS build

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY api ./api
COPY cli ./cli
COPY core ./core
COPY gui ./gui

RUN cargo build --locked --release -p api -p cli
RUN XDG_DATA_HOME=/assets ./target/release/furigana4epub update-dict

FROM debian:trixie-slim

RUN useradd --system --user-group --uid 10001 api \
    && install -d -o api -g api -m 0700 /data

COPY --from=build /src/target/release/api /usr/local/bin/furigana4epub-api
COPY --from=build --chown=api:api /assets/furigana4epub /opt/furigana4epub

ENV API_BIND_ADDR=0.0.0.0:8080 \
    API_DATA_DIR=/data \
    API_DICTIONARY_PATH=/opt/furigana4epub/system_full.dic \
    API_WORDLISTS_DIR=/opt/furigana4epub/wordlists

USER api
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/furigana4epub-api"]

FROM rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY test ./test
RUN cargo build --release --locked

FROM alpine:3@sha256:3f4d6b32e26703c6082839234f43769d5046b3ac7c910033911d1e71d8cd630a
RUN apk upgrade --no-cache && apk add --no-cache ca-certificates && adduser -D -u 1000 arrmcp
COPY --from=builder /src/target/release/arrmcp /usr/local/bin/arrmcp
USER arrmcp
EXPOSE 8000
# Image pensée pour le mode HTTP (jeton ARR_MCP_TOKEN exigé hors boucle locale) ; en stdio : docker run -i --rm … arrmcp --transport stdio
ENTRYPOINT ["arrmcp"]
CMD ["--transport", "http", "--host", "0.0.0.0", "--port", "8000"]

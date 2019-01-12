FROM liuchong/rustup:nightly as rust

RUN rustup target add wasm32-unknown-unknown && cargo install wasm-pack

ADD Cargo.toml .
ADD src ./src
RUN wasm-pack build

FROM node:11.6.0-slim as builder

COPY --from=rust /root/pkg/ .
RUN npm pack

FROM scratch

COPY --from=builder fazer-0.1.0.tgz .

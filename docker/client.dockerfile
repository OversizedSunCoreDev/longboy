FROM rust:latest AS base

FROM base AS sources

WORKDIR /tmp/app

COPY ./crates ./crates
COPY .rustfmt.toml .
COPY Cargo.toml .
COPY rust-toolchain.toml .
COPY crates/longboy-server-cli/test/data/ ./certs

FROM sources AS build

WORKDIR /tmp/app
RUN cargo build

FROM rust:latest AS release

WORKDIR /tmp/app

COPY --from=sources /tmp/app/certs .
COPY --from=build /tmp/app/target/debug/longboy-client* .

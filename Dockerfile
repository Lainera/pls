ARG BIN=spawn

FROM rust:1.57-bullseye as vendor
WORKDIR /code
COPY ./Cargo.toml ./Cargo.toml
RUN mkdir -p .cargo && cargo vendor > .cargo/config

FROM rust:1.57-bullseye as builder
ARG BIN
ENV USER=root

WORKDIR /code

RUN apt-get -yqq update
RUN apt-get -yqq install pkg-config ca-certificates libssl-dev
RUN rustup component add rustfmt

COPY ./Cargo.toml ./Cargo.toml
COPY ./src ./src
COPY ./build.rs ./build.rs
COPY ./proto ./proto

COPY --from=vendor /code/.cargo /code/.cargo
COPY --from=vendor /code/vendor /code/vendor  

RUN cargo build --release --offline --bin $BIN 

FROM debian:bullseye-slim 
ARG BIN
COPY --from=builder /code/target/release/$BIN /usr/local/bin/$BIN

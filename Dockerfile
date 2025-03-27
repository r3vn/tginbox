FROM rust:latest AS builder
WORKDIR /app

COPY Cargo.toml .
ADD src src

RUN cargo build --release
RUN strip target/release/tginbox

FROM debian:latest
WORKDIR /app

RUN apt clean
RUN mkdir /data

# Create an user for the application
RUN useradd -ms /bin/bash tginbox
RUN chown -R tginbox:tginbox /app

COPY --from=builder /app/target/release/tginbox .

USER tginbox
ENTRYPOINT ["./tginbox"]

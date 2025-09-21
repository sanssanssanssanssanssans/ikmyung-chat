FROM rust:1.77 as builder
WORKDIR /app
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo generate-lockfile
RUN cargo build --release
RUN rm -rf src
COPY src ./src
COPY static ./static
RUN cargo build --release --bin uchat-render
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/uchat-render /usr/local/bin/
COPY --from=builder /app/static /app/static
WORKDIR /app
EXPOSE 10000
CMD ["/usr/local/bin/uchat-render"]
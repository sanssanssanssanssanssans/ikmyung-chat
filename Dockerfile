FROM rust:1.77 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin uchat-render
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/uchat-render /usr/local/bin/
COPY --from=builder /app/static /app/static
WORKDIR /app
EXPOSE 10000
CMD ["/usr/local/bin/uchat-render"]
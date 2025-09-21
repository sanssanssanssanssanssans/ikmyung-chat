FROM rust:1.76 as builder
WORKDIR /app


COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true


COPY . .
RUN rm -f src/main.rs
RUN cargo build --release


FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/uchat-render /usr/local/bin/uchat-render
EXPOSE 10000
CMD ["/usr/local/bin/uchat-render"]
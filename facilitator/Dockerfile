FROM rust:1.90-slim AS builder

WORKDIR /app
COPY . .
RUN cargo build --release --bin x402-miden-facilitator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/x402-miden-facilitator /usr/local/bin/

ENV LOG_LEVEL=info
ENV BIND_ADDR=0.0.0.0:4020
ENV MIDEN_NETWORK=testnet
ENV MIDEN_RPC_URL=https://rpc.testnet.miden.io
ENV FAUCET_ID=0x37d5977a8e16d8205a360820f0230f

EXPOSE 4020

ENTRYPOINT ["x402-miden-facilitator"]

FROM rust:1-alpine AS chef

RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --features docker

FROM alpine
COPY --from=builder /app/target/release/gtasks-asana-bridge /
ENTRYPOINT [ "/gtasks-asana-bridge" ]
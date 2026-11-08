FROM rust:1-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static
RUN cargo build --release

FROM debian:stable-slim
COPY --from=build /app/target/release/kuis-rust /usr/local/bin/kuis-rust
ENV PORT=3000
EXPOSE 3000
CMD ["kuis-rust"]

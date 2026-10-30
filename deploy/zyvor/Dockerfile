# Builds the API, worker and sandbox provider. Final image runs as a non-root user.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p zyvor-api -p zyvor-provider-mock

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin zyvor
COPY --from=build /src/target/release/zyvor-api /src/target/release/zyvor-worker /src/target/release/zyvor-provider-mock /usr/local/bin/
USER 10001
EXPOSE 8080
CMD ["zyvor-api"]

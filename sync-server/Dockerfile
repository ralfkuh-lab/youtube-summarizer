# Build-Kontext ist das Repo-Root: docker build -f sync-server/Dockerfile .
# Dockerfile.dockerignore beschränkt den Kontext auf sync-proto/ und sync-server/.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY sync-proto/ sync-proto/
COPY sync-server/ sync-server/
RUN cd sync-server && cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --no-create-home youtube-sync \
    && mkdir /data && chown youtube-sync:youtube-sync /data
COPY --from=build /src/sync-server/target/release/sync-server /usr/local/bin/sync-server
USER youtube-sync
ENV SYNC_DB=/data/sync.db SYNC_PORT=8080
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["sync-server"]
CMD ["serve"]

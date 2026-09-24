#!/usr/bin/env bash
# Deployt den Sync-Server auf den VPS (docs/spec-sync.md, „Deployment“).
# Berührt nur /docker/youtube-sync/; Traefik und andere Stacks bleiben unverändert.
set -euo pipefail

HOST=hostinger
DIR=/docker/youtube-sync
HEALTH=https://yt-sync.srv1280390.hstgr.cloud/v1/health
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Der Container läuft als UID 10001 und schreibt nach data/.
ssh "$HOST" "mkdir -p $DIR/src $DIR/data && chown 10001:10001 $DIR/data"
rsync -az --delete --exclude target/ "$ROOT/sync-proto" "$ROOT/sync-server" "$HOST:$DIR/src/"
rsync -az "$ROOT/sync-server/deploy/docker-compose.yml" "$HOST:$DIR/docker-compose.yml"
ssh "$HOST" "cd $DIR && docker compose up -d --build"

for _ in $(seq 1 30); do
    if curl -fsS "$HEALTH"; then
        echo
        exit 0
    fi
    sleep 2
done
echo "Health-Check fehlgeschlagen: $HEALTH" >&2
exit 1

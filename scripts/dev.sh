#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -f .env ]; then printf 'Copie .env.example para .env primeiro.\n' >&2; exit 1; fi
set -a
source .env
set +a
if [ "$APP_ENV" != development ]; then printf 'dev.sh exige APP_ENV=development.\n' >&2; exit 1; fi
mkdir -p data
export APP_ORIGIN=http://localhost:5173 BIND=127.0.0.1:3000 DATABASE_URL=sqlite://data/subdisc.db
export LIVEKIT_INTERNAL_URL=http://127.0.0.1:7880 LIVEKIT_URL=ws://localhost:7880
export LIVEKIT_KEYS="$LIVEKIT_API_KEY: $LIVEKIT_API_SECRET"
subdisc_livekit=${LIVEKIT_BINARY:-$PWD/.tools/livekit/livekit-server}
subdisc_children=()
cleanup(){ for pid in "${subdisc_children[@]}"; do kill "$pid" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM
"$subdisc_livekit" --config infrastructure/livekit.yaml &
subdisc_children+=("$!")
./scripts/cargo.sh run --manifest-path backend/Cargo.toml &
subdisc_children+=("$!")
npm --prefix frontend run dev &
subdisc_children+=("$!")
wait -n "${subdisc_children[@]}"

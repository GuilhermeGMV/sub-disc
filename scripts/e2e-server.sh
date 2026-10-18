#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
subdisc_test_dir=$(mktemp -d /tmp/subdisc-e2e.XXXXXX)
subdisc_livekit=${LIVEKIT_BINARY:-$PWD/.tools/livekit/livekit-server}
subdisc_api_pid=''; subdisc_sfu_pid=''
cleanup(){ [ -z "$subdisc_api_pid" ] || kill "$subdisc_api_pid" 2>/dev/null || true; [ -z "$subdisc_sfu_pid" ] || kill "$subdisc_sfu_pid" 2>/dev/null || true; }
trap cleanup EXIT INT TERM
cat > "$subdisc_test_dir/livekit.yaml" <<'YAML'
port: 17880
bind_addresses: [127.0.0.1]
rtc:
  tcp_port: 17881
  udp_port: 17882
  node_ip: 127.0.0.1
  use_external_ip: false
  enable_loopback_candidate: true
keys:
  testkey: test-secret-at-least-thirty-two-characters
webhook:
  api_key: testkey
  urls: [http://127.0.0.1:18080/api/livekit/webhook]
logging:
  level: warn
YAML
"$subdisc_livekit" --config "$subdisc_test_dir/livekit.yaml" > "$subdisc_test_dir/livekit.log" 2>&1 &
subdisc_sfu_pid=$!
export APP_ENV=development APP_ORIGIN=http://127.0.0.1:18080 BIND=127.0.0.1:18080
export DATABASE_URL="sqlite://$subdisc_test_dir/test.db" SETUP_KEY=a-safe-setup-test-key STATIC_DIR="$PWD/frontend/dist"
export LIVEKIT_URL=ws://127.0.0.1:17880 LIVEKIT_INTERNAL_URL=http://127.0.0.1:17880 LIVEKIT_API_KEY=testkey LIVEKIT_API_SECRET=test-secret-at-least-thirty-two-characters
./backend/target/debug/subdisc > "$subdisc_test_dir/backend.log" 2>&1 &
subdisc_api_pid=$!
printf 'E2E logs: %s\n' "$subdisc_test_dir"
wait -n "$subdisc_api_pid" "$subdisc_sfu_pid"

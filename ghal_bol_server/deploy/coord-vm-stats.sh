#!/usr/bin/env bash
# Append one capacity line: disk, journal, coord DB dir, online peers, network deltas.
# Run manually or via coord-vm-monitor.timer (hourly).
set -euo pipefail

DATA_ROOT="${HOME}/.local/share/com.ghalbol.coord/ghalbol_server"
OPS_DIR="${DATA_ROOT}/ops"
STATS_LOG="${OPS_DIR}/stats.log"
NET_STATE="${OPS_DIR}/net.state"
COORD_HTTP="${COORD_STATS_URL:-http://127.0.0.1:8765}"

mkdir -p "${OPS_DIR}"

ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
df_line="$(df -h / | awk 'NR==2 {printf "%s used of %s (%s), avail %s", $3, $2, $5, $4}')"
journal="$(sudo journalctl --disk-usage 2>/dev/null | tr -d '\n' || echo "journal unknown")"
data_du="$(du -sh "${DATA_ROOT}" 2>/dev/null | awk '{print $1}' || echo "?")"

peers="?"
health="?"
tmp_peers="$(mktemp)"
if curl -fsS --max-time 3 "${COORD_HTTP}/health" >/dev/null 2>&1; then
  health="ok"
  if curl -fsS --max-time 3 "${COORD_HTTP}/v1/peers" -o "${tmp_peers}" 2>/dev/null; then
    if command -v jq >/dev/null 2>&1; then
      peers="$(jq '.peers | length' "${tmp_peers}")"
    elif command -v python3 >/dev/null 2>&1; then
      peers="$(python3 -c "import json; print(len(json.load(open('${tmp_peers}')).get('peers',[])))")"
    fi
  fi
else
  health="down"
fi
rm -f "${tmp_peers}"

iface="$(ip -o route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "dev") { print $(i + 1); exit }}')"
iface="${iface:-eth0}"
rx="$(cat "/sys/class/net/${iface}/statistics/rx_bytes" 2>/dev/null || echo 0)"
tx="$(cat "/sys/class/net/${iface}/statistics/tx_bytes" 2>/dev/null || echo 0)"
now_epoch="$(date +%s)"
rx_delta=0
tx_delta=0
if [[ -f "${NET_STATE}" ]]; then
  read -r prev_rx prev_tx _prev_ts < "${NET_STATE}" || true
  if [[ "${prev_rx}" =~ ^[0-9]+$ ]] && [[ "${prev_tx}" =~ ^[0-9]+$ ]]; then
    rx_delta=$((rx - prev_rx))
    tx_delta=$((tx - prev_tx))
  fi
fi
echo "${rx} ${tx} ${now_epoch}" > "${NET_STATE}"

# Human-readable network deltas since last sample (hourly timer ≈ per-hour egress hint).
rx_mb="$(awk "BEGIN {printf \"%.2f\", ${rx_delta}/1048576}")"
tx_mb="$(awk "BEGIN {printf \"%.2f\", ${tx_delta}/1048576}")"

line="${ts} health=${health} disk=\"${df_line}\" journal=\"${journal}\" coord_data=${data_du} peers_online=${peers} net_${iface}_rx_mb=${rx_mb} net_${iface}_tx_mb=${tx_mb}"

echo "${line}" | tee -a "${STATS_LOG}"

# Safety trim if logrotate not installed yet.
if [[ -f "${STATS_LOG}" ]]; then
  lines="$(wc -l < "${STATS_LOG}")"
  if [[ "${lines}" -gt 600 ]]; then
    tail -n 400 "${STATS_LOG}" > "${STATS_LOG}.tmp"
    mv "${STATS_LOG}.tmp" "${STATS_LOG}"
  fi
fi

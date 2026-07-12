#!/usr/bin/env bash
# Cap outbound bandwidth for libp2p relay TCP (default port 4002) using Linux tc HTB.
#
# Env:
#   GHAL_BOL_RELAY_EGRESS_MBIT  — max Mbit/s for relay egress (default 10; 0 = remove cap)
#   GHAL_BOL_RELAY_LISTEN       — host:port (default 0.0.0.0:4002) — port extracted for filter
set -euo pipefail

RATE_MBIT="${GHAL_BOL_RELAY_EGRESS_MBIT:-10}"
LISTEN="${GHAL_BOL_RELAY_LISTEN:-0.0.0.0:4002}"
PORT="${LISTEN##*:}"

DEV="$(ip -o route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "dev") { print $(i + 1); exit }}')"
DEV="${DEV:-eth0}"

sudo tc qdisc del dev "${DEV}" root 2>/dev/null || true

if [[ "${RATE_MBIT}" == "0" ]]; then
  echo "relay egress tc cap disabled (GHAL_BOL_RELAY_EGRESS_MBIT=0)"
  exit 0
fi

sudo tc qdisc add dev "${DEV}" root handle 1: htb default 20
sudo tc class add dev "${DEV}" parent 1: classid 1:1 htb rate 1000mbit
sudo tc class add dev "${DEV}" parent 1:1 classid 1:10 htb rate "${RATE_MBIT}mbit" ceil "${RATE_MBIT}mbit"
sudo tc class add dev "${DEV}" parent 1:1 classid 1:20 htb rate 1000mbit ceil 1000mbit
# Outbound relay traffic uses source port = relay listen port.
sudo tc filter add dev "${DEV}" protocol ip parent 1:0 prio 1 u32 match ip sport "${PORT}" 0xffff flowid 1:10

echo "relay egress capped: dev=${DEV} sport=${PORT} rate=${RATE_MBIT}mbit"

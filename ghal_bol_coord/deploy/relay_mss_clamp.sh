#!/usr/bin/env bash
# Clamp TCP MSS on the libp2p relay port so CGNAT/mobile paths (~1280 MTU, filtered ICMP)
# can complete identify + HOP reserve/connect. Without this, phones show bootstrap TCP ok
# but never peer_connected — relay logs "client connected" with no reservation/circuit events.
set -euo pipefail

PORT="${GHAL_BOL_RELAY_MSS_PORT:-4002}"
MSS="${GHAL_BOL_RELAY_MSS:-1200}"

add_rule() {
  local tool=$1 family=$2 dir=$3
  local match
  if [[ "$dir" == "in" ]]; then
    match="-p tcp --dport ${PORT} --tcp-flags SYN,RST SYN"
  else
    match="-p tcp --sport ${PORT} --tcp-flags SYN,RST SYN"
  fi
  if ! sudo "${tool}" -t mangle -C PREROUTING ${match} -j TCPMSS --set-mss "${MSS}" 2>/dev/null \
    && [[ "$dir" == "in" ]]; then
    sudo "${tool}" -t mangle -A PREROUTING ${match} -j TCPMSS --set-mss "${MSS}"
  elif ! sudo "${tool}" -t mangle -C OUTPUT ${match} -j TCPMSS --set-mss "${MSS}" 2>/dev/null \
    && [[ "$dir" == "out" ]]; then
    sudo "${tool}" -t mangle -A OUTPUT ${match} -j TCPMSS --set-mss "${MSS}"
  fi
}

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  echo "run with sudo: sudo $0" >&2
  exit 1
fi

add_rule iptables v4 in
add_rule iptables v4 out
if command -v ip6tables >/dev/null 2>&1; then
  add_rule ip6tables v6 in
  add_rule ip6tables v6 out
fi

echo "relay MSS clamp active port=${PORT} mss=${MSS} (v4+v6 in+out)"

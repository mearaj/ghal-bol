#!/usr/bin/env bash
# Source from other scripts:  source "$ROOT/scripts/load-ghalbol-env.sh" development
# Exports keys from GhalBol's env file and sets GHALBOL_ENV.
#
# Product env files live with each app crate (apps/<app>/env/, not the
# workspace root) so every GhalBol product owns its own ids/labels.
# Which app: GHALBOL_ENV_DIR wins, else GHALBOL_APP_DIR, else apps/ghalbol.
load_ghalbol_env() {
  local name="${1:?usage: load_ghalbol_env development|production}"
  local root="${GHALBOL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local file="${GHALBOL_ENV_DIR:-${GHALBOL_APP_DIR:-$root/apps/ghalbol}/env}/${name}.env"
  if [[ ! -f "$file" ]]; then
    echo "Missing GhalBol env file: $file" >&2
    return 1
  fi
  # Parse KEY=VALUE (don't `source` — unquoted spaces break bash, e.g. Ghal Bol Dev).
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    [[ -z "$line" || "$line" == \#* ]] && continue
    [[ "$line" != *=* ]] && continue
    local key="${line%%=*}"
    local value="${line#*=}"
    key="${key%"${key##*[![:space:]]}"}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    value="${value#\"}"
    value="${value%\"}"
    value="${value#\'}"
    value="${value%\'}"
    export "$key=$value"
  done <"$file"
  export GHALBOL_ENV="$name"
  echo "GhalBol env: $name (APP_ID=${GHALBOL_APP_ID:-?} LABEL=${GHALBOL_APP_LABEL:-?})"
}

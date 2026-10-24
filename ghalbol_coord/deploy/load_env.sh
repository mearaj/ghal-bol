# shellcheck shell=bash
# Source from run_server.sh / deploy_server.sh — do not execute directly.
load_server_env() {
  local server_dir="$1"
  local env_name="$2"
  local env_file="${server_dir}/${env_name}"
  local example="${server_dir}/${env_name}.example"

  if [[ ! -f "${env_file}" ]]; then
    if [[ -f "${example}" ]]; then
      cp "${example}" "${env_file}"
      echo "created ${env_file} from ${example} — edit it, then re-run." >&2
      exit 1
    fi
    echo "error: missing ${env_file}" >&2
    exit 1
  fi

  set -a
  # shellcheck source=/dev/null
  source "${env_file}"
  set +a
}

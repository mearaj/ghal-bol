# Resolve cargo target directory — same pattern as Kumkaj workspace.
cargo_target_dir() {
  # Try CARGO_TARGET_DIR env first, then default.
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    echo "$CARGO_TARGET_DIR"
    return
  fi
  local root="${GHALBOL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  # Cargo default: root/target when no explicit CARGO_TARGET_DIR.
  echo "$root/target"
}

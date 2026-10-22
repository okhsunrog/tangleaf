#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
project_dir=$PWD
binary="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/desktop/tangleaf"
test -x "$binary"

state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/tangleaf"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
launcher="$HOME/.local/bin/tangleaf"
mkdir -p "$state_dir" "$(dirname "$launcher")" "$data_dir/applications" "$data_dir/icons/hicolor/128x128/apps"

# This out-link is also a GC root for the launcher's GTK/WebKit runtime.
nix build "$project_dir#desktop-runtime" --out-link "$state_dir/runtime"
printf '#!/usr/bin/env bash\nexec %q %q "$@"\n' \
  "$state_dir/runtime/bin/tangleaf-local-run" "$binary" > "$launcher"
chmod +x "$launcher"
install -m644 src-tauri/icons/128x128.png "$data_dir/icons/hicolor/128x128/apps/tangleaf.png"

# Escape the desktop entry's quoted Exec argument independently of shell syntax.
desktop_launcher=${launcher//\\/\\\\}
desktop_launcher=${desktop_launcher//\"/\\\"}
desktop_launcher=${desktop_launcher//\$/\\\$}
desktop_launcher=${desktop_launcher//\`/\\\`}
desktop_launcher=${desktop_launcher//%/%%}
while IFS= read -r line; do
  if [[ $line == Exec=* ]]; then
    printf 'Exec="%s"\n' "$desktop_launcher"
  else
    printf '%s\n' "$line"
  fi
done < nix/tangleaf.desktop > "$data_dir/applications/dev.okhsunrog.tangleaf.desktop"
desktop-file-validate "$data_dir/applications/dev.okhsunrog.tangleaf.desktop"
update-desktop-database "$data_dir/applications"
printf 'Installed %s -> %s\n' "$launcher" "$binary"

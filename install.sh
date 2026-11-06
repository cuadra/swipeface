#!/bin/sh
# Builds swipeface, installs it for the current user, and makes it the
# default application for font files (so clicking one opens its specimen).
#
#   ./install.sh              build, install and register
#   ./install.sh --uninstall  remove everything again
set -eu

cd "$(dirname "$0")"

bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
desktop="$apps_dir/swipeface.desktop"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin_dir/swipeface" "$desktop"
    update-desktop-database "$apps_dir" 2>/dev/null || true
    echo "Removed swipeface."
    exit 0
fi

cargo build --release

install -Dm755 target/release/swipeface "$bin_dir/swipeface"

# File managers do not always share the shell's PATH, so point the launcher
# at the binary itself.
mkdir -p "$apps_dir"
sed "s|^Exec=swipeface|Exec=$bin_dir/swipeface|" swipeface.desktop > "$desktop"
update-desktop-database "$apps_dir" 2>/dev/null || true

mime_types=$(sed -n 's/^MimeType=//p' swipeface.desktop | tr ';' ' ')
for mime in $mime_types; do
    xdg-mime default swipeface.desktop "$mime"
done

echo "Installed $bin_dir/swipeface"
echo "Font files now open in Swipeface; run 'swipeface' to browse installed fonts."

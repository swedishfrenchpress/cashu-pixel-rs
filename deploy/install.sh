#!/bin/sh
# Install Cashu NERV as a desktop app on the Pi (run on the Pi as the desktop user):
#   ./install.sh path/to/cashu-pixel
# Adds the app menu entry, a desktop icon, and a taskbar launcher.
# Wallet data stays in ~/.local/share/cashu-pixel.
set -e
BIN="${1:?usage: install.sh <cashu-pixel binary>}"
HERE="$(cd "$(dirname "$0")" && pwd)"

sudo install -Dm755 "$BIN" /usr/local/lib/cashu-nerv/cashu-nerv
sudo install -Dm755 "$HERE/cashu-nerv" /usr/local/bin/cashu-nerv
sudo install -Dm644 "$HERE/cashu-nerv.svg" /usr/local/share/icons/hicolor/scalable/apps/cashu-nerv.svg
sudo install -Dm644 "$HERE/cashu-nerv.desktop" /usr/local/share/applications/cashu-nerv.desktop
sudo gtk-update-icon-cache -q -t -f /usr/local/share/icons/hicolor || true

# Desktop icon: a link to the menu entry, as the Pi menu's "Add to desktop" makes.
# (An executable copy makes the file manager prompt "Execute File" on every launch.)
mkdir -p "$HOME/Desktop"
rm -f "$HOME/Desktop/cashu-nerv.desktop"
ln -s /usr/local/share/applications/cashu-nerv.desktop "$HOME/Desktop/cashu-nerv.desktop"

# Launch desktop shortcuts without the file manager's "Execute File" prompt
# (File Manager > Preferences > "Don't ask options on launch executable file").
# The user config starts as a copy of the system defaults so nothing else changes.
LIBFM="$HOME/.config/libfm/libfm.conf"
mkdir -p "$(dirname "$LIBFM")"
[ -f "$LIBFM" ] || cp /etc/xdg/libfm/libfm.conf "$LIBFM" 2>/dev/null || printf '[config]\n' > "$LIBFM"
if grep -q '^quick_exec=' "$LIBFM"; then
    sed -i 's/^quick_exec=.*/quick_exec=1/' "$LIBFM"
else
    sed -i '/^\[config\]/a quick_exec=1' "$LIBFM"
fi

# Taskbar launcher: append to the panel's launcher list once
PANEL="$HOME/.config/wf-panel-pi/wf-panel-pi.ini"
mkdir -p "$(dirname "$PANEL")"
touch "$PANEL"
if ! grep -q '^launchers=.*cashu-nerv' "$PANEL"; then
    current="$(grep '^launchers=' "$PANEL" | cut -d= -f2-)"
    [ -n "$current" ] || current="$(grep '^launchers=' /etc/xdg/wf-panel-pi/wf-panel-pi.ini | cut -d= -f2-)"
    sed -i '/^launchers=/d' "$PANEL"
    grep -q '^\[panel\]' "$PANEL" || printf '[panel]\n' >> "$PANEL"
    sed -i "/^\[panel\]/a launchers=$current cashu-nerv" "$PANEL"
fi

# Reload the desktop and taskbar so the icon, launcher and setting take effect
# (lwrespawn restarts both)
pkill -x pcmanfm 2>/dev/null || true
pkill -x wf-panel-pi 2>/dev/null || true

echo "Cashu NERV installed: /usr/local/bin/cashu-nerv"

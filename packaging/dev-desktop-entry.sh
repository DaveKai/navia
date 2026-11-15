#!/usr/bin/env bash
# Installs Navia's desktop file and icons for the current user, so a build run from target/ gets
# the right icon. Desktops match a Wayland window to "<app_id>.desktop", so the file is named after
# APP_ID. It only touches Navia's own files and leaves a desktop file without X-Navia-Dev alone.
#
#   packaging/dev-desktop-entry.sh            install or refresh
#   packaging/dev-desktop-entry.sh --remove   remove what it installed
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=app.env
source "$ROOT/packaging/app.env"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor"
DESKTOP="$APPS/$APP_ID.desktop"
MARK="X-Navia-Dev=true"
SIZES="16 22 24 32 48 64 128 256 512"

ours() { [ ! -e "$DESKTOP" ] || grep -qx "$MARK" "$DESKTOP"; }

# The dev entry and icons from before the app ID changed, removed only when the entry is ours.
OLD_ID=navia
remove_old() {
    local old="$APPS/$OLD_ID.desktop"
    [ "$OLD_ID" != "$APP_ID" ] && [ -e "$old" ] && grep -qx "$MARK" "$old" || return 0
    rm -f "$old" "$ICONS/scalable/apps/$OLD_ID.svg"
    for s in $SIZES; do rm -f "$ICONS/${s}x${s}/apps/$OLD_ID.png"; done
    changed=1
}

if [ "${1:-}" = "--remove" ]; then
    ours || { echo "$DESKTOP isn't Navia's dev entry; left alone" >&2; exit 1; }
    rm -f "$DESKTOP" "$ICONS/scalable/apps/$APP_ID.svg"
    for s in $SIZES; do rm -f "$ICONS/${s}x${s}/apps/$APP_ID.png"; done
    remove_old
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
    command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
    exit 0
fi

if ! ours; then
    echo "$DESKTOP exists and isn't Navia's dev entry; left alone" >&2
    exit 0
fi

changed=0
remove_old
# Copies $1 to $2 only when it differs, and notes the change.
put() {
    if ! cmp -s "$1" "$2"; then
        install -Dm 644 "$1" "$2"
        changed=1
    fi
}

put "$ROOT/packaging/linux/icons/app.svg" "$ICONS/scalable/apps/$APP_ID.svg"
for s in $SIZES; do
    put "$ROOT/packaging/linux/icons/app-$s.png" "$ICONS/${s}x${s}/apps/$APP_ID.png"
done

# Hidden from menus, since it's only here for the icon.
entry="$(mktemp)"
trap 'rm -f "$entry"' EXIT
sed -e "s|@APP_NAME@|$APP_NAME|g" -e "s|@APP_ID@|$APP_ID|g" -e "s|@APP_DISPLAY_NAME@|$APP_DISPLAY_NAME|g" \
    -e "s|@APP_SUMMARY@|$APP_SUMMARY|g" -e "s|^Exec=.*|Exec=\"$ROOT/apps/navia-gpui/run.sh\"|" \
    "$ROOT/packaging/linux/app.desktop.in" > "$entry"
printf 'NoDisplay=true\n%s\n' "$MARK" >> "$entry"
put "$entry" "$DESKTOP"

if [ "$changed" = 1 ]; then
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
    command -v gtk-update-icon-cache >/dev/null && [ -f "$ICONS/index.theme" ] && gtk-update-icon-cache -q "$ICONS" || true
    # KDE finds desktop files through its service cache.
    command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
    echo "navia: desktop entry and icons installed for $APP_ID"
fi

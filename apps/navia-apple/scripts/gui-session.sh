#!/bin/bash
# Runs a command on the Mac inside the logged-in user's GUI session and waits for it.
# An SSH session can't use the login keychain, so codesign fails there with errSecInternalComponent. The GUI
# session's keychain is unlocked, so signing works without a prompt.
#   gui-session.sh xcodebuild -project Navia.xcodeproj ...   (keeps the current directory, PATH and HOME)
# Output comes back on stdout, and the exit code is the command's own.
set -u
[ $# -gt 0 ] || { echo "usage: $0 command [args...]" >&2; exit 2; }
uid=$(id -u)
dir=$(mktemp -d /tmp/navia-gui.XXXXXX) || exit 1
label="dev.navia.gui.$$.${RANDOM}"
{
  printf 'cd %q || exit 99\n' "$PWD"
  printf 'export PATH=%q HOME=%q\n' "$PATH" "$HOME"
  printf '%q ' "$@"; printf '\n'
  printf 'echo $? > %q\n' "$dir/rc.tmp"
  printf 'mv %q %q\n' "$dir/rc.tmp" "$dir/rc"
} > "$dir/run.sh"
cat > "$dir/job.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>$label</string>
<key>ProgramArguments</key><array><string>/bin/bash</string><string>$dir/run.sh</string></array>
<key>StandardOutPath</key><string>$dir/out</string>
<key>StandardErrorPath</key><string>$dir/out</string>
<key>RunAtLoad</key><true/>
</dict></plist>
PLIST
: > "$dir/out"
cleanup() { launchctl bootout "gui/$uid/$label" >/dev/null 2>&1; [ -n "${tailpid:-}" ] && kill "$tailpid" 2>/dev/null; rm -rf "$dir"; }
trap 'cleanup; exit 130' INT TERM HUP
if ! launchctl bootstrap "gui/$uid" "$dir/job.plist" 2>/dev/null; then
  echo "gui-session: no GUI session to run in (is the user logged in?)" >&2
  rm -rf "$dir"; exit 98
fi
tail -n +1 -f "$dir/out" & tailpid=$!
while [ ! -f "$dir/rc" ]; do sleep 1; done
sleep 0.3
rc=$(cat "$dir/rc")
cleanup
exit "$rc"

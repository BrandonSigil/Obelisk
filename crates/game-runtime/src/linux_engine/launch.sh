#!/bin/bash
# Runs a Windows game through Voile's Linux engine (voile-lkd): voile-wine built for x86-64 Linux, which
# fexdarwin-run executes in native macOS processes. Starts the window server, Neptune's render server and the input
# service, then the game, and ends them when the game ends or this script is stopped. Every guest process follows
# this script (FEXDARWIN_SUPERVISOR_PID) and ends within half a second once it is gone.
#
# Usage: launch.sh RUNTIME HOME INSTALL CWD [--env NAME=VALUE]... -- EXECUTABLE [ARGUMENT]...
#   RUNTIME     the Linux engine runtime (engine/, graphics/, wine/, root/)
#   HOME        the guest's /home/user; the prefix is its prefix folder
#   INSTALL     the game's installation, seen as /games/game
#   CWD         the guest working directory
#   EXECUTABLE  the guest path of the Windows program
# SAILOVER_APP_NAME names the game in the menu bar, the Dock and Cmd-Tab.
set -u
runtime=$1 home=$2 install=$3 cwd=$4
shift 4
guest_env=()
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
    guest_env+=("$1" "$2")
    shift 2
done
shift

# wsd and the render server bind UNIX sockets in a private folder; a relative path keeps them inside sun_path.
sockets_root=${TMPDIR:-/tmp}
sockets=sailover-engine-$$
cd "$sockets_root" || exit 1
mkdir -m 700 "$sockets" || exit 1

helpers=()
game=
finish() {
    [ -n "$game" ] && kill -TERM "$game" 2>/dev/null
    for helper in ${helpers[@]+"${helpers[@]}"}; do kill -TERM "$helper" 2>/dev/null; done
    # They end within a few milliseconds of SIGTERM; a helper still there after a second is killed.
    for _ in $(seq 1 20); do
        alive=
        for helper in ${helpers[@]+"${helpers[@]}"}; do kill -0 "$helper" 2>/dev/null && alive=1; done
        [ -z "$alive" ] && break
        sleep 0.05
    done
    for helper in ${helpers[@]+"${helpers[@]}"}; do kill -KILL "$helper" 2>/dev/null; done
    rm -rf "${sockets_root:?}/$sockets"
}
trap 'finish; trap - EXIT; exit 143' TERM INT HUP
trap finish EXIT

# wsd runs from an application bundle named after the game: macOS takes the name in the menu bar, the Dock and
# Cmd-Tab from it (the game gives the icon). The bundle keeps one place per game, so Launch Services sees one app.
app_name=${SAILOVER_APP_NAME:-Game}
app_name=${app_name//\//-}
app_id=$(printf '%s' "$app_name" | LC_ALL=C tr -c 'A-Za-z0-9' '-')
app="$sockets_root/sailover-apps/$app_id/$app_name.app"
mkdir -p "$app/Contents/MacOS" || exit 1
rm -f "$app/Contents/MacOS/wsd"
cp -c "$runtime/engine/wsd" "$app/Contents/MacOS/wsd" 2>/dev/null || cp "$runtime/engine/wsd" "$app/Contents/MacOS/wsd" || exit 1
xml_name=$(printf '%s' "$app_name" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g')
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>wsd</string>
  <key>CFBundleIdentifier</key><string>com.zagrios.sailover.game.$app_id</string>
  <key>CFBundleName</key><string>$xml_name</string>
  <key>CFBundleDisplayName</key><string>$xml_name</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.games</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

"$app/Contents/MacOS/wsd" --dir "$sockets" &
helpers+=($!)
dxmt="$runtime/graphics/dxmt-native.framework/Versions/A/dxmt-native"
RENDER_SERVER_EXEC_PATH="$runtime/graphics/libexec/virgl_render_server" NPT_BACKEND=dxmt \
    NPT_D3D11_LIBRARY_PATH="$dxmt" NPT_DXGI_LIBRARY_PATH="$dxmt" NPT_D3D12_LIBRARY_PATH="$dxmt" \
    "$runtime/graphics/bin/virgl_test_server" --no-virgl --no-fork --socket-path "$sockets/vtest.sock" &
helpers+=($!)
# The input service gives the Mac's gamepads to Wine's SailOver bus. A runtime without it starts the game without
# gamepads.
input=
if [ -x "$runtime/engine/sailover-input" ]; then
    "$runtime/engine/sailover-input" --dir "$sockets" &
    helpers+=($!)
    input=1
fi
for _ in $(seq 1 100); do
    [ -S "$sockets/wsd.sock" ] && [ -S "$sockets/vtest.sock" ] && { [ -z "$input" ] || [ -S "$sockets/input.sock" ]; } &&
        break
    sleep 0.05
done

export FEXDARWIN_LKD=1 FEXDARWIN_ROOT="$runtime/root" FEXDARWIN_ROOT_RO=1 FEXDARWIN_SUPERVISOR_PID=$$
export FEXDARWIN_BIND="/opt/wine=$runtime/wine:ro;/home/user=$home;/games/game=$install;/run/sailover=$sockets_root/$sockets"
# wineserver writes the registry only when it exits: wait for it inside the guest before the game ends. Started
# with -p0, it stops Wine's own processes as soon as the game's last process ends, instead of 3 seconds later.
"$runtime/engine/fexdarwin-run" --cwd "$cwd" ${guest_env[@]+"${guest_env[@]}"} \
    /bin/sh -c '/opt/wine/bin/wineserver -p0; /opt/wine/bin/wine "$@"; status=$?; /opt/wine/bin/wineserver -w; exit $status' sh "$@" &
game=$!
wait "$game"
status=$?
game=
exit "$status"

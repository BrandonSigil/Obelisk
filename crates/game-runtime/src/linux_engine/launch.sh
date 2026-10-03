#!/bin/bash
# Runs a Windows game through Voile's Linux engine (voile-lkd): voile-wine built for x86-64 Linux, which
# fexdarwin-run executes in native macOS processes. Starts the window server and Neptune's render server, then the
# game, and ends both when the game ends or this script is stopped. Every guest process follows this script
# (FEXDARWIN_SUPERVISOR_PID) and ends within half a second once it is gone.
#
# Usage: launch.sh RUNTIME HOME INSTALL CWD [--env NAME=VALUE]... -- EXECUTABLE [ARGUMENT]...
#   RUNTIME     the Linux engine runtime (engine/, graphics/, wine/, root/)
#   HOME        the guest's /home/user; the prefix is its prefix folder
#   INSTALL     the game's installation, seen as /games/game
#   CWD         the guest working directory
#   EXECUTABLE  the guest path of the Windows program
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
    sleep 1
    for helper in ${helpers[@]+"${helpers[@]}"}; do kill -KILL "$helper" 2>/dev/null; done
    rm -rf "${sockets_root:?}/$sockets"
}
trap 'finish; trap - EXIT; exit 143' TERM INT HUP
trap finish EXIT

"$runtime/engine/wsd" --dir "$sockets" &
helpers+=($!)
dxmt="$runtime/graphics/dxmt-native.framework/Versions/A/dxmt-native"
RENDER_SERVER_EXEC_PATH="$runtime/graphics/libexec/virgl_render_server" NPT_BACKEND=dxmt \
    NPT_D3D11_LIBRARY_PATH="$dxmt" NPT_DXGI_LIBRARY_PATH="$dxmt" NPT_D3D12_LIBRARY_PATH="$dxmt" \
    "$runtime/graphics/bin/virgl_test_server" --no-virgl --no-fork --socket-path "$sockets/vtest.sock" &
helpers+=($!)
for _ in $(seq 1 100); do
    [ -S "$sockets/wsd.sock" ] && [ -S "$sockets/vtest.sock" ] && break
    sleep 0.05
done

export FEXDARWIN_LKD=1 FEXDARWIN_ROOT="$runtime/root" FEXDARWIN_ROOT_RO=1 FEXDARWIN_SUPERVISOR_PID=$$
export FEXDARWIN_BIND="/opt/wine=$runtime/wine:ro;/home/user=$home;/games/game=$install;/run/sailover=$sockets_root/$sockets"
# wineserver writes the registry only when it exits: wait for it inside the guest before the game ends.
"$runtime/engine/fexdarwin-run" --cwd "$cwd" ${guest_env[@]+"${guest_env[@]}"} \
    /bin/sh -c '/opt/wine/bin/wine "$@"; status=$?; /opt/wine/bin/wineserver -w; exit $status' sh "$@" &
game=$!
wait "$game"
status=$?
game=
exit "$status"

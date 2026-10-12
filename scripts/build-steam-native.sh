#!/bin/bash
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if [[ "$(uname -m)" != arm64 ]]; then
    echo "The native Steam engine integration currently requires Apple silicon." >&2
    exit 1
fi
case "${CONFIGURATION:-Debug}" in
    Debug) PROFILE_NAME=dev; PROFILE_DIRECTORY=debug ;;
    Release) PROFILE_NAME=release; PROFILE_DIRECTORY=release ;;
    *) exit 1 ;;
esac
# Obelisk ships no Valve binary: the first Steam launch downloads the pinned
# packages from Valve's CDN. Remove the payload earlier builds bundled.
rm -rf "$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/SteamRuntime"
export CARGO_TARGET_DIR="$PROJECT_ROOT/build/steam-native-target"
cargo build --locked --manifest-path "$PROJECT_ROOT/crates/steam-native/Cargo.toml" \
    --workspace --bin obelisk-steam-host --lib --profile "$PROFILE_NAME" --target aarch64-apple-darwin
cargo build --locked --manifest-path "$PROJECT_ROOT/crates/steam-native/Cargo.toml" \
    -p obelisk-steam-native --bin obelisk-steam-sdk --profile "$PROFILE_NAME" --target x86_64-apple-darwin
cargo build --locked --manifest-path "$PROJECT_ROOT/crates/steam-native/Cargo.toml" \
    -p obelisk-steam-discovery-adapter --lib --profile "$PROFILE_NAME" --target x86_64-apple-darwin
ARM_BUILD="$CARGO_TARGET_DIR/aarch64-apple-darwin/$PROFILE_DIRECTORY"
INTEL_BUILD="$CARGO_TARGET_DIR/x86_64-apple-darwin/$PROFILE_DIRECTORY"
DESTINATION="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Helpers/SteamNative"
mkdir -p "$DESTINATION"
cp "$ARM_BUILD/obelisk-steam-host" "$DESTINATION/obelisk-steam-host"
# Remove the former diagnostic helper from incremental app builds.
rm -f "$DESTINATION/obelisk-steam-sdk-arm64"
cp "$INTEL_BUILD/obelisk-steam-sdk" "$DESTINATION/obelisk-steam-sdk-x86_64"
ADAPTER="$DESTINATION/libobelisk_steam_discovery_adapter.dylib"
/usr/bin/lipo -create "$ARM_BUILD/libobelisk_steam_discovery_adapter.dylib" \
    "$INTEL_BUILD/libobelisk_steam_discovery_adapter.dylib" -output "$ADAPTER"
IDENTITY="${EXPANDED_CODE_SIGN_IDENTITY:?A stable Apple signing identity is required}"
# Notarization needs a secure timestamp; Debug skips the round trip to Apple.
if [[ "${CONFIGURATION:-Debug}" == Release ]]; then TIMESTAMP=--timestamp; else TIMESTAMP=--timestamp=none; fi
/usr/bin/codesign --force --sign "$IDENTITY" "$TIMESTAMP" --options runtime \
    --identifier com.zagrios.obelisk.steam-host \
    --entitlements "$PROJECT_ROOT/packaging/steam-host.entitlements" "$DESTINATION/obelisk-steam-host"
/usr/bin/codesign --force --sign "$IDENTITY" "$TIMESTAMP" \
    --identifier com.zagrios.obelisk.steam-discovery "$ADAPTER"
/usr/bin/codesign --force --sign "$IDENTITY" "$TIMESTAMP" --options runtime \
    --identifier com.zagrios.obelisk.steam-sdk-x86_64 \
    --entitlements "$PROJECT_ROOT/packaging/steam-host.entitlements" \
    "$DESTINATION/obelisk-steam-sdk-x86_64"

# Opening this Intel-only app through LaunchServices lets macOS present its
# Rosetta installer. A direct --probe invocation never starts the app run loop.
ROSETTA_SOURCE="$PROJECT_ROOT/packaging/rosetta"
ROSETTA_BUILD="$PROJECT_ROOT/build/rosetta"
ROSETTA_APP="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Helpers/Obelisk Rosetta.app"
mkdir -p "$ROSETTA_BUILD" "$ROSETTA_APP/Contents/MacOS"
ROSETTA_EXECUTABLE="$ROSETTA_BUILD/ObeliskRosetta"
SDK_PATH="$(xcrun --sdk macosx --show-sdk-path)"
MINIMUM_MACOS="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
BUILD_SETTINGS="$SDK_PATH|$MINIMUM_MACOS"
PREVIOUS_SETTINGS="$(cat "$ROSETTA_BUILD/settings" 2>/dev/null || true)"
if [[ ! -f "$ROSETTA_EXECUTABLE" \
    || "$ROSETTA_SOURCE/main.m" -nt "$ROSETTA_EXECUTABLE" \
    || "$0" -nt "$ROSETTA_EXECUTABLE" \
    || "$BUILD_SETTINGS" != "$PREVIOUS_SETTINGS" ]]; then
    xcrun --sdk macosx clang -arch x86_64 -isysroot "$SDK_PATH" \
        "-mmacosx-version-min=$MINIMUM_MACOS" -fobjc-arc -O2 -Wall -Wextra -Werror \
        -framework AppKit "$ROSETTA_SOURCE/main.m" -o "$ROSETTA_EXECUTABLE"
    printf '%s\n' "$BUILD_SETTINGS" > "$ROSETTA_BUILD/settings"
fi
cp "$ROSETTA_EXECUTABLE" "$ROSETTA_APP/Contents/MacOS/ObeliskRosetta"
if ! cmp -s "$ROSETTA_SOURCE/Info.plist" "$ROSETTA_APP/Contents/Info.plist"; then
    cp "$ROSETTA_SOURCE/Info.plist" "$ROSETTA_APP/Contents/Info.plist"
fi
/usr/bin/codesign --force --sign "$IDENTITY" "$TIMESTAMP" --options runtime "$ROSETTA_APP"

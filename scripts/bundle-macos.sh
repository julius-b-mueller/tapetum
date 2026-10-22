#!/bin/sh
# Builds target/bundle/Tapetum.app.
#
#   scripts/bundle-macos.sh [--dmg] [--x64]
#
# With --dmg, also writes target/bundle/Tapetum-macos-arm64.dmg.
# With --x64, the app is built for Intel Macs instead (cross-compiled on any Mac):
# target/bundle/x64/Tapetum.app and target/bundle/Tapetum-macos-x64.dmg.
set -eu

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

dmg_wanted=no
arch=arm64
for option in "$@"; do
    case "$option" in
        --dmg) dmg_wanted=yes ;;
        --x64) arch=x64 ;;
        *) echo "unknown option: $option" >&2; exit 2 ;;
    esac
done

if [ "$arch" = x64 ]; then
    rustup target add x86_64-apple-darwin
    cargo build --release -p tapetum --target x86_64-apple-darwin
    binary=target/x86_64-apple-darwin/release/tapetum
    app=target/bundle/x64/Tapetum.app
else
    cargo build --release -p tapetum
    binary=target/release/tapetum
    app=target/bundle/Tapetum.app
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/Tapetum"

# App icon in the sizes macOS asks for.
iconset=target/bundle/Tapetum.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/tapetum/assets/icon-1024.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" crates/tapetum/assets/icon-1024.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Tapetum.icns"
rm -rf "$iconset"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Tapetum</string>
    <key>CFBundleDisplayName</key><string>Tapetum</string>
    <key>CFBundleIdentifier</key><string>io.github.julius-b-mueller.tapetum</string>
    <key>CFBundleExecutable</key><string>Tapetum</string>
    <key>CFBundleIconFile</key><string>Tapetum</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Sealed ad hoc: downloaded unsigned bundles are otherwise called "damaged";
# this way they are only from an unidentified developer.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

echo "Built $app (version $version, $(lipo -archs "$app/Contents/MacOS/Tapetum"))"

if [ "$dmg_wanted" = yes ]; then
    dmg=target/bundle/Tapetum-macos-$arch.dmg
    rm -f "$dmg"
    staging=target/bundle/dmg-$arch
    rm -rf "$staging"
    mkdir -p "$staging"
    cp -R "$app" "$staging/"
    ln -s /Applications "$staging/Applications"
    hdiutil create -quiet -volname Tapetum -srcfolder "$staging" -ov -format UDZO "$dmg"
    rm -rf "$staging"
    echo "Built $dmg"
fi

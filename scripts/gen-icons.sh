#!/usr/bin/env bash
# Regenerate every derived logo/icon asset from the sources:
#
#   graphics-sources/ledgeline-logo-no-word.svg   flat logo, indigo square background
#   assets/Ledgeline.icon/                         Icon Composer document (macOS 26+
#                                                  layered icon: default, dark, clear,
#                                                  tinted). Its layer SVGs were cut from
#                                                  graphics-sources/ledgeline-logo-no-word-no-bg.svg
#                                                  and the indigo (#524ED2) background is
#                                                  the document's `fill` in icon.json.
#
# Outputs (all committed):
#
#   web/static/ledgeline-icon.svg        README logo
#   web/static/ledgeline-icon-small.svg  zoomed-in variant: in-app navbar logo + browser-tab icon
#   docs/images/ledgeline-icon.svg  logo on the docs home page
#   docs/.mbr/favicon.png           docs-site favicon, from the small variant (mbr serves .mbr/ over its defaults)
#   assets/Assets.car               compiled Icon Composer icon, shipped in
#                                   Ledgeline.app/Contents/Resources (CFBundleIconName)
#   assets/ledgeline.png            1024² flattened render of the default appearance on
#                                   the macOS icon grid. flake.nix turns it into
#                                   ledgeline.icns (pre-macOS-26 fallback, CFBundleIconFile)
#                                   and the Linux hicolor icons.
#   assets/ledgeline-small.png      the same, zoomed in (the small variant); flake.nix
#                                   uses it for the 16/32/48px icns and hicolor slots.
#
# WHY COMMITTED rather than built by flake.nix: `actool` and `ictool` ship only
# inside Xcode, which the Nix sandbox cannot reach. So this runs by hand on a
# Mac with Xcode installed (`just icons`) after the sources change, and the
# outputs are reviewed and committed like the golden fixtures.
set -euo pipefail

cd "$(dirname "$0")/.."

# NOT $DEVELOPER_DIR: the nix dev shell points that at its apple-sdk, which has
# no actool. Override with XCODE_DEVELOPER_DIR for a non-default Xcode.
export DEVELOPER_DIR="${XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
ICTOOL="$DEVELOPER_DIR/../Applications/Icon Composer.app/Contents/Executables/ictool"
for tool in "$DEVELOPER_DIR/usr/bin/actool" "$ICTOOL"; do
    [ -x "$tool" ] || { echo "error: $tool not found — needs Xcode 26+ (set XCODE_DEVELOPER_DIR)" >&2; exit 1; }
done
command -v magick >/dev/null || { echo "error: imagemagick (magick) not on PATH" >&2; exit 1; }

SRC=graphics-sources/ledgeline-logo-no-word.svg
# Absolute: actool delegates to a long-lived helper process that resolves a
# relative path against the cwd of whichever earlier actool run started it.
ICON="$PWD/assets/Ledgeline.icon"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 1. Flat SVG cropped to the indigo square. The source canvas is 1140.47²
#    with the square at (91.07, 93.6) 951.65²; the top grid line deliberately
#    runs past the square and the viewBox clips it, as the artboard does.
sed -E 's/viewBox="[^"]*"/viewBox="91.07 93.6 951.65 951.65"/' "$SRC" > web/static/ledgeline-icon.svg
cp web/static/ledgeline-icon.svg docs/images/ledgeline-icon.svg

# 1b. Small variant: the same artwork zoomed in on the arrow and the right-hand
#     peaks, for anything shown at roughly 32px or less, where the full design
#     reads as a blur. The crop was matched to Patrick's mockup.
SMALL_VIEWBOX="559 199 463 463"
sed -E "s/viewBox=\"[^\"]*\"/viewBox=\"$SMALL_VIEWBOX\"/" "$SRC" > web/static/ledgeline-icon-small.svg

# 2. Docs favicon, from the small variant: the file is 256² but browsers show
#    it at 16–32px. Rasterize big, then downsample, for clean edges.
magick -background none -density 600 web/static/ledgeline-icon-small.svg -resize 256x256 \
    -depth 8 -strip docs/.mbr/favicon.png

# 3. Compiled layered icon. actool also emits a 16–256px Ledgeline.icns and a
#    partial Info.plist; neither is used — the icns is too small for Finder's
#    large sizes (flake.nix builds a full one from step 4), and the plist keys
#    are already in assets/Info.plist.in.
xcrun actool "$ICON" --compile "$tmp" --app-icon Ledgeline \
    --enable-on-demand-resources NO --development-region en \
    --target-device mac --platform macosx --minimum-deployment-target 10.15 \
    --output-partial-info-plist "$tmp/partial.plist" \
    --output-format human-readable-text 2>&1 | grep -v 'unhandled Platform key' || true
cp "$tmp/Assets.car" assets/Assets.car

# 4. Flattened default appearance on Apple's macOS grid: an 824² body centred
#    on a transparent 1024² canvas, so the pre-26 icon sits at the same size as
#    its neighbours in the Dock instead of overflowing them.
render_flat() { # <.icon> <output.png>
    "$ICTOOL" "$1" --export-image --output-file "$tmp/body.png" --platform macOS \
        --rendition Default --width 824 --height 824 --scale 1 >/dev/null
    magick "$tmp/body.png" -background none -gravity center -extent 1024x1024 \
        -depth 8 -strip "$2"
}
render_flat "$ICON" assets/ledgeline.png

# 5. The same render of the small variant: a throwaway copy of the .icon with
#    every layer cropped to SMALL_VIEWBOX, so it keeps the identical glass and
#    squircle. flake.nix uses it for the ≤48px icns and hicolor slots.
#    Deliberately NOT in Assets.car: Icon Composer has no per-size variants,
#    and actool lets a .icon override any same-named .appiconset outright, so
#    macOS 26+ draws the full design at every size. Swapping the .icon for a
#    classic per-size icon set would buy legible list-view icons at the cost
#    of the tuned dark/clear/tinted appearances; we chose to keep those.
cp -R "$ICON" "$tmp/LedgelineSmall.icon"
# perl, not `sed -i`: BSD and GNU sed disagree on -i, and either may be first
# on PATH depending on whether the nix shell is active.
perl -pi -e "s/viewBox=\"[^\"]*\"/viewBox=\"$SMALL_VIEWBOX\"/" "$tmp"/LedgelineSmall.icon/Assets/*.svg
render_flat "$tmp/LedgelineSmall.icon" assets/ledgeline-small.png

echo "regenerated: web/static/ledgeline-icon{,-small}.svg docs/images/ledgeline-icon.svg docs/.mbr/favicon.png assets/Assets.car assets/ledgeline{,-small}.png"

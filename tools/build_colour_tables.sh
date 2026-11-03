#!/bin/sh
# Builds every colour table from the session-2 Windows chart captures.
#   tools/build_colour_tables.sh <captures-dir> [out-dir]   (default out-dir: packaging/colour)
# Measured: the Preferred tables of every coating/quality, and Matte Speed Standard/gamma 1.8/Off.
# Derived (not measured): the other modes, from their coating/quality's Preferred table plus the
# change the mode makes on Matte Speed (xtask derive-colour-mode).
set -eu
cap=${1:?captures dir}
out=${2:-packaging/colour}
here=$(cd "$(dirname "$0")/.." && pwd)
png=$here/testdata/colour/c1_colour_chart_session2.png
layout=$here/testdata/colour/c1_colour_chart.layout.json
cargo build -q --release -p xtask --manifest-path "$here/Cargo.toml"
x=$here/target/release/xtask
mkdir -p "$out"
fit() { # capture-id table-name
    printf '%s <- %s: ' "$2" "$1"
    "$x" fit-colour "$cap/$1".prn "$png" "$layout" "$out/$2.lut" | tail -n 1
}
fit 700_chart_plain_speed plain-speed-preferred
fit 701_chart_matte_speed matte-speed-preferred
fit 702_chart_matte_q1 matte-q1-preferred
fit 703_chart_matte_q2 matte-q2-preferred
fit 704_chart_synthetic_speed synthetic-speed-preferred
fit 705_chart_synthetic_q1 synthetic-q1-preferred
fit 706_chart_synthetic_q2 synthetic-q2-preferred
fit 707_chart_glossy_speed glossy-speed-preferred
fit 708_chart_glossy_q1 glossy-q1-preferred
fit 709_chart_glossy_q2 glossy-q2-preferred
fit 710_chart_wristband_speed wristband-speed-preferred
fit 711_chart_wristband_q1 wristband-q1-preferred
fit 720_chart_color_standard matte-speed-standard
fit 721_chart_gamma_18 matte-speed-gamma18
fit 722_chart_color_off matte-speed-off
for base in plain-speed matte-q1 matte-q2 synthetic-speed synthetic-q1 synthetic-q2 \
            glossy-speed glossy-q1 glossy-q2 wristband-speed wristband-q1; do
    for mode in standard gamma18 off; do
        "$x" derive-colour-mode "$out/$base-preferred.lut" "$out/matte-speed-preferred.lut" \
            "$out/matte-speed-$mode.lut" "$out/$base-$mode.lut"
    done
done
ls "$out"/*.lut | wc -l

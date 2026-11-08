#!/bin/sh
# メーター画面用フォント（firmware/main/components/meter_fonts）を再生成する。
#
# 事前に次を用意する（新しい空のディレクトリで作業し、そのディレクトリからは実行しない）。
#   - Noto Sans JP の TTF（SIL OFL 1.1）。例：
#       https://github.com/google/fonts/raw/main/ofl/notosansjp/NotoSansJP[wght].ttf
#   - lv_font_conv（npm）。例：npm install --prefix <作業ディレクトリ> lv_font_conv
#
# 使い方：tools/gen_fonts.sh <NotoSansJP.ttf> <lv_font_conv> <出力ディレクトリ>
set -eu

TTF=${1:?TTFのパス}
LV_FONT_CONV=${2:?lv_font_convのパス}
OUT=${3:?出力ディレクトリ}

# 画面で使う日本語の文字。追加するときはここに足す。
SYMBOLS='総合温度重視レースギアアクセル開度水油記録停止速緯経時刻日本間判定不能画面仮値取得更新°'

for size in 20 28; do
  "$LV_FONT_CONV" --font "$TTF" -r 0x20-0x7E --font "$TTF" --symbols "$SYMBOLS" \
    --size "$size" --bpp 4 --format lvgl --lv-include lvgl.h --no-kerning -o "$OUT/jp_$size.c"
done
"$LV_FONT_CONV" --font "$TTF" -r 0x30-0x39,0x2D,0x2E,0x3A \
  --size 72 --bpp 4 --format lvgl --lv-include lvgl.h --no-kerning -o "$OUT/num_72.c"

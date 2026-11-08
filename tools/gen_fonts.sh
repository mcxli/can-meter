#!/bin/sh
# メーター画面用フォント（firmware/main/components/meter_fonts）を再生成する。
#
# 事前に、新しい空の作業ディレクトリ <作業> を用意し、そこに次を置く（作業ディレクトリからは実行しない）。
#   1. Noto Sans JP の可変フォント NotoSansJP.ttf（SIL OFL 1.1）
#        https://github.com/google/fonts/raw/main/ofl/notosansjp/NotoSansJP[wght].ttf
#   2. Material Symbols Outlined の可変フォント MaterialSymbols.ttf（Apache 2.0）
#        https://github.com/google/material-design-icons/raw/master/variablefont/MaterialSymbolsOutlined[FILL,GRAD,opsz,wght].ttf
#   3. lv_font_conv（npm）：npm install --prefix <作業> lv_font_conv
#   4. fonttools（Python）：python3 -m venv <作業>/venv && <作業>/venv/bin/pip install fonttools
#
# 使い方：tools/gen_fonts.sh <作業> <出力ディレクトリ>
set -eu

WORK=${1:?作業ディレクトリ}
OUT=${2:?出力ディレクトリ}
LV_FONT_CONV="$WORK/node_modules/.bin/lv_font_conv"

# 太字（wght=700）のインスタンスを、可変フォントから取り出す
"$WORK/venv/bin/fonttools" varLib.instancer "$WORK/NotoSansJP.ttf" wght=700 -o "$WORK/NotoSansJP-Bold.ttf"
BOLD="$WORK/NotoSansJP-Bold.ttf"
ICONS="$WORK/MaterialSymbols.ttf"

# 画面で使う日本語の文字。追加するときはここに足す。
SYMBOLS='総合温度重視レースギアアクセル開度水油記録停止速緯経時刻日本間判定不能画面仮値取得更新°'

# アイコン（Material Symbols のコードポイント）
#   e9e4 speed（総合）          f076 thermostat（温度）      f06e sports_score（レース）
#   f1db location_on（GPS）     e047 stop（記録停止）         e1ff device_thermostat（水温）
#   ec15 oil_barrel（油温）     f337 car_gear（ギア）         e63e wifi
#   e623 sd_card                e55c my_location（GPS測位）   eff7 directions_car（CAN）
ICON_CODEPOINTS=0xe9e4,0xf076,0xf06e,0xf1db,0xe047,0xe1ff,0xec15,0xf337,0xe63e,0xe623,0xe55c,0xeff7

for size in 20 28; do
  "$LV_FONT_CONV" --font "$BOLD" -r 0x20-0x7E --font "$BOLD" --symbols "$SYMBOLS" \
    --size "$size" --bpp 4 --format lvgl --lv-include lvgl.h --no-kerning -o "$OUT/jp_$size.c"
done
for size in 56 72 96; do
  "$LV_FONT_CONV" --font "$BOLD" -r 0x30-0x39,0x2D,0x2E,0x3A \
    --size "$size" --bpp 4 --format lvgl --lv-include lvgl.h --no-kerning -o "$OUT/num_$size.c"
done
for size in 28 36; do
  "$LV_FONT_CONV" --font "$ICONS" -r "$ICON_CODEPOINTS" \
    --size "$size" --bpp 4 --format lvgl --lv-include lvgl.h --no-kerning -o "$OUT/icons_$size.c"
done

#pragma once

#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

// 日本語（画面で使う文字のみ）と ASCII（0x20-0x7E）。太字。20px・28px
LV_FONT_DECLARE(jp_20);
LV_FONT_DECLARE(jp_28);
// 大きな数値用。太字。0-9、-、.、:、N のみ（Nはギアのニュートラル）。56px・72px・96px
LV_FONT_DECLARE(num_56);
LV_FONT_DECLARE(num_96);
// ギア用。太字。0-9、-、N のみ。128px
LV_FONT_DECLARE(num_128);
LV_FONT_DECLARE(num_72);
// アイコン（Material Symbols から必要なものだけ）。28px・36px。コードポイントは tools/gen_fonts.sh を参照
LV_FONT_DECLARE(icons_28);
LV_FONT_DECLARE(icons_36);

#ifdef __cplusplus
}
#endif

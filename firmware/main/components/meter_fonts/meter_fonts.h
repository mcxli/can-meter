#pragma once

#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

// 日本語（画面で使う文字のみ）と ASCII（0x20-0x7E）。20px・28px
LV_FONT_DECLARE(jp_20);
LV_FONT_DECLARE(jp_28);
// 大きな数値用。0-9、-、.、: のみ。72px
LV_FONT_DECLARE(num_72);

#ifdef __cplusplus
}
#endif

#pragma once

#include "lvgl.h"

#ifdef __cplusplus
extern "C" {
#endif

/** 0xRRGGBB から lv_color_t を作る（lv_color_hex は static inline でバインディングに生成されないため） */
lv_color_t ui_color(uint32_t hex);

#ifdef __cplusplus
}
#endif

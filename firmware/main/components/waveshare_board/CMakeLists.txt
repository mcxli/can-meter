# Waveshare ESP32-S3-Touch-LCD-4.3B のLCD・タッチ・LVGLポート
# 出典: Waveshare公式サンプル 08_lvgl_Porting（原本は ~/esp/waveshare-demo に保持）
idf_component_register(
    SRCS "waveshare_rgb_lcd_port.c" "lvgl_port.c" "board_sd.c"
    INCLUDE_DIRS "."
    REQUIRES driver fatfs sdmmc vfs esp_lcd esp_psram esp_timer freertos lvgl__lvgl espressif__esp_lcd_touch_gt911)

idf_component_get_property(lvgl_lib lvgl__lvgl COMPONENT_LIB)
target_compile_options(${lvgl_lib} PRIVATE -Wno-format)
target_compile_options(${COMPONENT_LIB} PRIVATE -Wno-attributes)

# メーター画面用のフォント（Noto Sans JP から必要な文字だけを抜き出したLVGLのフォント）。
# 再生成の手順は tools/gen_fonts.sh を参照。
idf_component_register(
    SRCS "jp_20.c" "jp_28.c" "num_56.c" "num_72.c" "num_96.c" "num_128.c" "icons_28.c" "icons_36.c"
    INCLUDE_DIRS "."
    REQUIRES lvgl__lvgl)

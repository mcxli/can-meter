# meter_fonts

メーター画面用のLVGLフォント。Noto Sans JP（SIL Open Font License 1.1、`OFL.txt`）から、画面で使う文字だけを抜き出して生成している。

- `jp_20.c`・`jp_28.c`：日本語（画面で使う文字）と ASCII。20px・28px、4bpp、無圧縮
- `num_72.c`：数値用（0-9、`-`、`.`、`:`）。72px

文字を追加するときは、`tools/gen_fonts.sh` の `SYMBOLS` に足して再生成する。

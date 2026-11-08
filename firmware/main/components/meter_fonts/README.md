# meter_fonts

メーター画面用のLVGLフォント。画面で使う文字・アイコンだけを抜き出して生成している。

| ファイル | 内容 | 元のフォント |
| --- | --- | --- |
| `jp_20.c`・`jp_28.c` | 日本語（画面で使う文字）と ASCII。太字。20px・28px | Noto Sans JP Bold（SIL OFL 1.1） |
| `num_56.c`・`num_72.c`・`num_96.c` | 数値用（0-9、`-`、`.`、`:`、`N`）。太字。56px・72px・96px | Noto Sans JP Bold |
| `num_128.c` | ギア用（0-9、`-`、`N`）。太字。128px | Noto Sans JP Bold |
| `icons_28.c`・`icons_36.c` | アイコン12個。28px・36px | Material Symbols Outlined（Apache 2.0） |

4bpp、無圧縮。ライセンス文は `LICENSE-*.txt`。

文字・アイコンを追加するときは、`tools/gen_fonts.sh` を更新して再生成する。

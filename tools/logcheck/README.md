# logcheck

SDに保存したログ（[ログ形式](../../docs/実装仕様/ログ形式.md)）を、Macで確認するツール。

```sh
cd tools/logcheck
cargo run --release -- /Volumes/SDCARD/LOG_0010_DEMO
```

表示する内容：

- CAN：行数、記録時間、フレーム/秒、時刻の逆転の件数
- IDごと：件数、平均の受信周期 [Hz]、最大の受信間隔 [ms]
- GPS：行数、連番の欠番の件数
- イベントの件数、キューあふれで捨てた件数（`QUEUE_OVERFLOW` の合計）
- 復号の候補値の範囲（`firmware/shared` の `decode`。公開情報による候補で、実車で照合するまで確定ではない）
- 形式の誤り（ヘッダー、列数、16進、DLCとデータの長さ、末尾の改行）。誤りがあれば終了コード1

`<保存先のディレクトリ>` は説明用で、実際のパスに置き換える。テストは `cargo test`。

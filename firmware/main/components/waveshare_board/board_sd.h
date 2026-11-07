#pragma once

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * microSD（TF）カードをSPIモードでマウントし、/sdcard に登録する。
 * LCD・タッチの初期化（I2Cドライバのインストール）の後に呼ぶこと。
 * フォーマットは行わない（マウントできなければエラーを返す）。
 */
esp_err_t board_sd_mount(void);

/** /sdcard をアンマウントし、SPIバスを解放する。 */
void board_sd_unmount(void);

#ifdef __cplusplus
}
#endif

// microSD（TF）カードのマウント。出典: Waveshare公式サンプル 03_SD_Test（sd_card.c）を元に、
// LCD・タッチと同時に使えるよう、I2Cの再初期化とバックライトを消す書き込みを除いた。
#include "board_sd.h"

#include <stdio.h>

#include "driver/i2c.h"
#include "driver/sdspi_host.h"
#include "driver/spi_common.h"
#include "esp_log.h"
#include "esp_vfs_fat.h"
#include "freertos/FreeRTOS.h"
#include "sdmmc_cmd.h"

static const char *TAG = "board_sd";

#define SD_MOUNT_POINT "/sdcard"
#define SD_PIN_MOSI 11
#define SD_PIN_MISO 13
#define SD_PIN_CLK 12

// CH422G（I/Oエキスパンダ）。0x24=出力モード設定、0x38=出力ビット
#define I2C_PORT 0
#define CH422G_ADDR_MODE 0x24
#define CH422G_ADDR_OUT 0x38
#define I2C_TIMEOUT_MS 1000
// 出力ビット（bit n = EXIO n）：bit1=TP_RST、bit2=バックライト、bit3=LCD_RST、bit4=SD_CS（Lowで選択）、bit5=USB_SEL
// 全て「通常動作」の状態：TP_RST解除・バックライトon・LCD_RST解除・SD_CS選択・USB_SEL=1
#define CH422G_OUT_NORMAL 0x2E

static sdmmc_card_t *s_card = NULL;
static sdmmc_host_t s_host = SDSPI_HOST_DEFAULT();

esp_err_t board_sd_mount(void)
{
    uint8_t buf = 0x01;
    i2c_master_write_to_device(I2C_PORT, CH422G_ADDR_MODE, &buf, 1, I2C_TIMEOUT_MS / portTICK_PERIOD_MS);
    buf = CH422G_OUT_NORMAL; // SD_CSをLowにしつつ、バックライトはonのまま
    esp_err_t ret = i2c_master_write_to_device(I2C_PORT, CH422G_ADDR_OUT, &buf, 1, I2C_TIMEOUT_MS / portTICK_PERIOD_MS);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "CH422G write failed: %s", esp_err_to_name(ret));
        return ret;
    }

    spi_bus_config_t bus_cfg = {
        .mosi_io_num = SD_PIN_MOSI,
        .miso_io_num = SD_PIN_MISO,
        .sclk_io_num = SD_PIN_CLK,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = 4000,
    };
    ret = spi_bus_initialize(s_host.slot, &bus_cfg, SDSPI_DEFAULT_DMA);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "spi_bus_initialize failed: %s", esp_err_to_name(ret));
        return ret;
    }

    sdspi_device_config_t slot_config = SDSPI_DEVICE_CONFIG_DEFAULT();
    slot_config.gpio_cs = GPIO_NUM_NC; // CSはCH422G経由
    slot_config.host_id = s_host.slot;

    esp_vfs_fat_sdmmc_mount_config_t mount_config = {
        .format_if_mount_failed = false, // 自動フォーマットはしない
        .max_files = 5,
        .allocation_unit_size = 16 * 1024,
    };
    ret = esp_vfs_fat_sdspi_mount(SD_MOUNT_POINT, &s_host, &slot_config, &mount_config, &s_card);
    if (ret != ESP_OK) {
        ESP_LOGE(TAG, "mount failed: %s", esp_err_to_name(ret));
        spi_bus_free(s_host.slot);
        return ret;
    }
    sdmmc_card_print_info(stdout, s_card);
    return ESP_OK;
}

void board_sd_unmount(void)
{
    if (s_card) {
        esp_vfs_fat_sdcard_unmount(SD_MOUNT_POINT, s_card);
        spi_bus_free(s_host.slot);
        s_card = NULL;
    }
}

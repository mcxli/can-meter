use esp_idf_svc::sys::*;

mod logger;
mod ui;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("CAN meter main (Rust)");

    unsafe {
        // LCD・タッチ・LVGLタスクの初期化（C側のボード用ポート）。
        // 内部のタッチリセットで、CH422Gの出力は通常状態（バックライトon・SD_CS選択）になる。
        // wavesahre_rgb_lcd_bl_on() は書き込む値でSD_CSをHighにするため、SDと併用する場合は呼ばない。
        waveshare_esp32_s3_rgb_lcd_init();

        // LVGLのAPIはスレッドセーフではないため、ロックしてから操作する
        if lvgl_port_lock(-1) {
            ui::build_screens();
            lvgl_port_unlock();
        }

        // 画面表示中にSDをマウントする。失敗しても、画面は表示し続ける（記録だけができない）
        let ret = board_sd_mount();
        if ret != ESP_OK as esp_err_t {
            log::error!("SD mount failed: {ret}");
            logger::set_error();
            return;
        }
    }

    // SD初期化の完了後に、自動で記録を開始する（GPSの測位や画面の状態を待たない）。
    // CAN・GPSは、実データが来るまで（段階4・6）、仮データを記録する。
    match logger::start("/sdcard", ui::SHOW_DEMO_TAG) {
        Ok(tx) => logger::spawn_demo_producer(tx),
        Err(e) => {
            log::error!("記録を開始できない: {e}");
            logger::set_error();
        }
    }
}

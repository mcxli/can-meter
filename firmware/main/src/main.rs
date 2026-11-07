use esp_idf_svc::sys::*;
use std::io::{Read, Write};

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("CAN meter main (Rust): display + SD trial");

    unsafe {
        // LCD・タッチ・LVGLタスクの初期化（C側のボード用ポート）。
        // 内部のタッチリセットで、CH422Gの出力は通常状態（バックライトon・SD_CS選択）になる。
        // wavesahre_rgb_lcd_bl_on() は書き込む値でSD_CSをHighにするため、SDと併用する場合は呼ばない。
        waveshare_esp32_s3_rgb_lcd_init();

        // LVGLのAPIはスレッドセーフではないため、ロックしてから操作する
        if lvgl_port_lock(-1) {
            lv_demo_widgets();
            let label = lv_label_create(lv_disp_get_layer_top(lv_disp_get_default()));
            lv_label_set_text(label, c"Rust + LVGL + SD".as_ptr());
            lv_obj_align(label, LV_ALIGN_TOP_LEFT as u8, 8, 4);
            lvgl_port_unlock();
        }

        // 画面表示中にSDをマウントする
        let ret = board_sd_mount();
        if ret != ESP_OK as esp_err_t {
            log::error!("SD mount failed: {ret}");
            return;
        }
    }

    // RustのstdでSDを読み書きする
    let path = "/sdcard/rust.txt";
    match std::fs::File::create(path).and_then(|mut f| f.write_all(b"Hello from Rust!")) {
        Ok(()) => log::info!("SD write ok: {path}"),
        Err(e) => log::error!("SD write failed: {e}"),
    }
    let mut text = String::new();
    match std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut text)) {
        Ok(_) => log::info!("SD read ok: '{text}'"),
        Err(e) => log::error!("SD read failed: {e}"),
    }

    log::info!("display + SD trial: done");
}

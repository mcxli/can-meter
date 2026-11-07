use esp_idf_svc::sys::*;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("CAN meter main (Rust): display trial");

    unsafe {
        // LCD・タッチ・LVGLタスクの初期化（C側のボード用ポート）
        waveshare_esp32_s3_rgb_lcd_init();
        // バックライトは自動ではオンにならない（段階2の知見）。明示的にオンにする
        wavesahre_rgb_lcd_bl_on();

        // LVGLのAPIはスレッドセーフではないため、ロックしてから操作する
        if lvgl_port_lock(-1) {
            lv_demo_widgets();
            // Rustから作ったラベルを最前面に表示する
            let label = lv_label_create(lv_disp_get_layer_top(lv_disp_get_default()));
            lv_label_set_text(label, c"Rust + LVGL".as_ptr());
            lv_obj_align(label, LV_ALIGN_TOP_LEFT as u8, 8, 4);
            lvgl_port_unlock();
        }
    }

    log::info!("display trial: init done");
}

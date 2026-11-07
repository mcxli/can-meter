use esp_idf_svc::sys::{heap_caps_get_free_size, heap_caps_get_total_size, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM};

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("CAN meter main: boot");

    // PSRAM・内部RAMの容量を起動ログに出す（段階1の確認用）
    let (psram_total, psram_free, internal_total, internal_free) = unsafe {
        (
            heap_caps_get_total_size(MALLOC_CAP_SPIRAM),
            heap_caps_get_free_size(MALLOC_CAP_SPIRAM),
            heap_caps_get_total_size(MALLOC_CAP_INTERNAL),
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        )
    };
    log::info!("PSRAM: total={psram_total} free={psram_free}");
    log::info!("Internal RAM: total={internal_total} free={internal_free}");
}

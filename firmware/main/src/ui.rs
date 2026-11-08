//! メーター画面（総合画面）。LVGLのC APIを `unsafe` で呼ぶ。
//!
//! 表示するデータは `shared` の仮データ生成（`DemoGenerator`）から得る。
//! 値の判定（無効値の「--」、回転数通知）は `shared` のロジックに任せ、ここでは描画だけを行う。
//! LVGLのAPIはスレッドセーフではないため、呼び出し側が `lvgl_port_lock` を保持した状態で使う
//! （タイマーのコールバックはLVGLのタスク内で実行されるため、ロック済み）。

use esp_idf_svc::sys::*;
use shared::demo::{DemoGenerator, DemoMode};
use shared::shift_notice::{GearDisplay, ShiftNotice, ShiftNoticeConfig};
use shared::vehicle::{value_or_dash, Limits, VehicleData};
use std::ffi::{CStr, CString};
use std::ptr::null_mut;
use std::sync::Mutex;

// 配色（サンプル画像に合わせた仮の値）
const BG: u32 = 0x0B151D;
const TILE: u32 = 0x122230;
const TRACK: u32 = 0x2A3A48;
const WHITE: u32 = 0xFFFFFF;
const TEXT_DIM: u32 = 0xB8C4CE;
const ORANGE: u32 = 0xF5A524;
const CYAN: u32 = 0x1CC8F0;
const GREEN: u32 = 0x35D07F;
const RED: u32 = 0xE5333A;
const GEAR_BRIGHT: u32 = 0xFF3B30;
const GEAR_DIM: u32 = 0x7A1A16;
const GEAR_INVALID: u32 = 0x6B7785;

// アイコン（Material Symbols のコードポイント。meter_fonts の icons_28・icons_36 に含まれるもの）
const ICON_SPEED: &CStr = c"\u{e9e4}";
const ICON_THERMOSTAT: &CStr = c"\u{f076}";
const ICON_RACE: &CStr = c"\u{f06e}";
const ICON_LOCATION: &CStr = c"\u{f1db}";
const ICON_STOP: &CStr = c"\u{e047}";
const ICON_WATER_TEMP: &CStr = c"\u{e1ff}";
const ICON_OIL: &CStr = c"\u{ec15}";
const ICON_GEAR: &CStr = c"\u{f337}";

// 回転数通知の設定。仮の値（表示確認用。実車の限界回転数やエンジン保護を保証しない）
const NOTICE_ACTIVATE_RPM: u16 = 6500;
const NOTICE_RELEASE_RPM: u16 = 6000;
const NOTICE_BLINK_MS: u32 = 400;

/// 画面の部品へのポインタと、表示する仮データの状態。
struct Ui {
    rpm_arc: *mut lv_obj_t,
    rpm_label: *mut lv_obj_t,
    speed_label: *mut lv_obj_t,
    gear_label: *mut lv_obj_t,
    water_label: *mut lv_obj_t,
    oil_label: *mut lv_obj_t,
    accel_label: *mut lv_obj_t,
    accel_mask: *mut lv_obj_t,
    data: VehicleData,
    generator: DemoGenerator,
    notice: ShiftNotice,
    // 変化したときだけ描画を更新するための直前の表示内容
    last_rpm: String,
    last_speed: String,
    last_gear: String,
    last_water: String,
    last_oil: String,
    last_accel: String,
    last_gear_color: Option<GearDisplay>,
}

// LVGLのオブジェクトは、LVGLのロックを保持して操作するため、スレッド間で移してよい
unsafe impl Send for Ui {}

static UI: Mutex<Option<Ui>> = Mutex::new(None);

const ACCEL_BAR_W: i32 = 290;

// 回転数メーターの形状（画面上の位置・大きさ）
const ARC_X: i32 = 16;
const ARC_Y: i32 = 44;
const ARC_SIZE: i32 = 290;
const ARC_W: i32 = 12;
const ARC_R: f32 = ARC_SIZE as f32 / 2.0;
const ARC_CX: f32 = ARC_X as f32 + ARC_R;
const ARC_CY: f32 = ARC_Y as f32 + ARC_R;
const ARC_START_DEG: i32 = 135; // 0rpm（左下）。LVGLの角度は3時方向が0度で時計回り
const ARC_END_DEG: i32 = 45; // 8000rpm（右下）。270度の範囲
const RPM_MAX: i16 = 8000;
// 赤い帯の開始（仮の値。実車の限界回転数やエンジン保護を保証しない）
const RPM_REDLINE_FROM: i32 = 7000;

/// 回転数を、LVGLの円弧の角度（0〜359度）に換算する。
fn rpm_to_deg(rpm: i32) -> i32 {
    (ARC_START_DEG + 270 * rpm / RPM_MAX as i32) % 360
}

fn deg_cos_sin(deg: i32) -> (f32, f32) {
    let rad = (deg as f32).to_radians();
    (rad.cos(), rad.sin())
}

unsafe fn color(hex: u32) -> lv_color_t {
    ui_color(hex)
}

unsafe fn no_pad_no_scroll(o: *mut lv_obj_t) {
    lv_obj_set_style_pad_left(o, 0, 0);
    lv_obj_set_style_pad_right(o, 0, 0);
    lv_obj_set_style_pad_top(o, 0, 0);
    lv_obj_set_style_pad_bottom(o, 0, 0);
    lv_obj_clear_flag(o, LV_OBJ_FLAG_SCROLLABLE as lv_obj_flag_t);
}

/// 背景色つきの四角形（余白・枠・スクロールなし）。
unsafe fn rect(parent: *mut lv_obj_t, x: i32, y: i32, w: i32, h: i32, bg: u32, radius: i32) -> *mut lv_obj_t {
    let o = lv_obj_create(parent);
    lv_obj_set_pos(o, x as lv_coord_t, y as lv_coord_t);
    lv_obj_set_size(o, w as lv_coord_t, h as lv_coord_t);
    lv_obj_set_style_bg_color(o, color(bg), 0);
    lv_obj_set_style_bg_opa(o, 255, 0);
    lv_obj_set_style_border_width(o, 0, 0);
    lv_obj_set_style_radius(o, radius as lv_coord_t, 0);
    no_pad_no_scroll(o);
    o
}

unsafe fn label(
    parent: *mut lv_obj_t,
    text: &CStr,
    font: *const lv_font_t,
    fg: u32,
    x: i32,
    y: i32,
) -> *mut lv_obj_t {
    let l = lv_label_create(parent);
    lv_label_set_text(l, text.as_ptr());
    lv_obj_set_style_text_font(l, font, 0);
    lv_obj_set_style_text_color(l, color(fg), 0);
    lv_obj_set_pos(l, x as lv_coord_t, y as lv_coord_t);
    l
}

/// 幅を決めて中央揃えにしたラベル（値の桁数が変わっても位置が動かない）。
unsafe fn centered_label(
    parent: *mut lv_obj_t,
    text: &CStr,
    font: *const lv_font_t,
    fg: u32,
    x: i32,
    y: i32,
    w: i32,
) -> *mut lv_obj_t {
    let l = label(parent, text, font, fg, x, y);
    lv_obj_set_width(l, w as lv_coord_t);
    lv_obj_set_style_text_align(l, LV_TEXT_ALIGN_CENTER as lv_text_align_t, 0);
    l
}

/// 回転数メーター用の円弧（つまみなし・操作不可）。
unsafe fn make_arc(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    let arc = lv_arc_create(parent);
    lv_obj_set_pos(arc, ARC_X as lv_coord_t, ARC_Y as lv_coord_t);
    lv_obj_set_size(arc, ARC_SIZE as lv_coord_t, ARC_SIZE as lv_coord_t);
    lv_obj_remove_style(arc, null_mut(), LV_PART_KNOB as lv_style_selector_t);
    lv_obj_clear_flag(arc, LV_OBJ_FLAG_CLICKABLE as lv_obj_flag_t);
    for part in [LV_PART_MAIN, LV_PART_INDICATOR] {
        lv_obj_set_style_arc_width(arc, ARC_W as lv_coord_t, part as lv_style_selector_t);
        lv_obj_set_style_arc_rounded(arc, false, part as lv_style_selector_t);
    }
    arc
}

/// 2点を結ぶ線（目盛り）。線の座標は、LVGLが保持し続けるため、解放しない（画面と同じ寿命）。
unsafe fn tick(parent: *mut lv_obj_t, p0: (f32, f32), p1: (f32, f32), width: i32, fg: u32) {
    let (min_x, min_y) = (p0.0.min(p1.0).floor(), p0.1.min(p1.1).floor());
    let points: &'static mut [lv_point_t; 2] = Box::leak(Box::new([
        lv_point_t { x: (p0.0 - min_x) as lv_coord_t, y: (p0.1 - min_y) as lv_coord_t },
        lv_point_t { x: (p1.0 - min_x) as lv_coord_t, y: (p1.1 - min_y) as lv_coord_t },
    ]));
    let line = lv_line_create(parent);
    lv_line_set_points(line, points.as_ptr(), 2);
    lv_obj_set_pos(line, min_x as lv_coord_t, min_y as lv_coord_t);
    lv_obj_set_style_line_width(line, width as lv_coord_t, 0);
    lv_obj_set_style_line_color(line, color(fg), 0);
    lv_obj_set_style_line_rounded(line, false, 0);
}

unsafe fn set_text(l: *mut lv_obj_t, s: &str) {
    if let Ok(c) = CString::new(s) {
        lv_label_set_text(l, c.as_ptr());
    }
}

/// 総合画面を作り、100msごとに仮データで更新するタイマーを開始する。
///
/// `lvgl_port_lock` を保持した状態で呼ぶこと。
pub unsafe fn build_general_screen() {
    let jp20: *const lv_font_t = &jp_20;
    let jp28: *const lv_font_t = &jp_28;
    let num72: *const lv_font_t = &num_72;
    let num56: *const lv_font_t = &num_56;
    let num96: *const lv_font_t = &num_96;
    let icon28: *const lv_font_t = &icons_28;
    let icon36: *const lv_font_t = &icons_36;

    let screen = lv_disp_get_scr_act(lv_disp_get_default());
    lv_obj_set_style_bg_color(screen, color(BG), 0);
    lv_obj_set_style_bg_opa(screen, 255, 0);
    no_pad_no_scroll(screen);

    // 状態表示（仮データのため、すべて正常として表示する）
    for (i, name) in [c"CAN RX", c"GPS FIX", c"Wi-Fi", c"SD REC"].iter().enumerate() {
        let x = 16 + i as i32 * 150;
        rect(screen, x, 14, 12, 12, GREEN, 6);
        label(screen, name, jp20, TEXT_DIM, x + 20, 5);
    }
    let demo_tag = rect(screen, 690, 4, 96, 30, BG, 4);
    lv_obj_set_style_border_width(demo_tag, 2, 0);
    lv_obj_set_style_border_color(demo_tag, color(CYAN), 0);
    centered_label(demo_tag, c"DEMO", jp20, CYAN, 0, 2, 92);

    // 回転数（円弧）
    // 回転数メーター（0〜8000rpm、270度）。下から順に、背景、7000rpm以上の赤い帯、現在値の円弧を重ねる
    let track = make_arc(screen);
    lv_arc_set_bg_angles(track, ARC_START_DEG as u16, ARC_END_DEG as u16);
    lv_obj_set_style_arc_color(track, color(TRACK), LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_arc_opa(track, 0, LV_PART_INDICATOR as lv_style_selector_t);
    let redline = make_arc(screen);
    lv_arc_set_bg_angles(redline, rpm_to_deg(RPM_REDLINE_FROM) as u16, ARC_END_DEG as u16);
    lv_obj_set_style_arc_color(redline, color(RED), LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_arc_opa(redline, 0, LV_PART_INDICATOR as lv_style_selector_t);
    let rpm_arc = make_arc(screen);
    lv_arc_set_bg_angles(rpm_arc, ARC_START_DEG as u16, ARC_END_DEG as u16);
    lv_arc_set_range(rpm_arc, 0, RPM_MAX);
    lv_arc_set_value(rpm_arc, 0);
    lv_obj_set_style_arc_opa(rpm_arc, 0, LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_arc_color(rpm_arc, color(CYAN), LV_PART_INDICATOR as lv_style_selector_t);

    // 目盛り（500rpmごとの線、1000rpmごとに長い線と数字）
    for i in 0..=(RPM_MAX as i32 / 500) {
        let rpm = i * 500;
        let major = rpm % 1000 == 0;
        let (cos, sin) = deg_cos_sin(rpm_to_deg(rpm));
        let r_out = ARC_R - ARC_W as f32 - 3.0;
        let r_in = r_out - if major { 14.0 } else { 7.0 };
        let p0 = (ARC_CX + r_out * cos, ARC_CY + r_out * sin);
        let p1 = (ARC_CX + r_in * cos, ARC_CY + r_in * sin);
        tick(screen, p0, p1, if major { 3 } else { 2 }, if major { WHITE } else { TEXT_DIM });
        if major {
            let r_label = r_in - 14.0;
            let text = CString::new((rpm / 1000).to_string()).unwrap();
            let (lx, ly) = (ARC_CX + r_label * cos, ARC_CY + r_label * sin);
            // 1桁の数字（jp_20）の中心を目盛りの位置に合わせる
            centered_label(screen, &text, jp20, WHITE, lx as i32 - 10, ly as i32 - 14, 20);
        }
    }
    // 中央の数値は、目盛りの数字と重ならないよう、小さいフォントで少し下に置く
    let rpm_label = centered_label(screen, c"--", num56, WHITE, 16, 148, 290);
    centered_label(screen, c"rpm", jp20, TEXT_DIM, 16, 212, 290);

    // 車速
    let speed_tile = rect(screen, 330, 44, 230, 150, TILE, 10);
    let speed_label = centered_label(speed_tile, c"--", num96, WHITE, 0, 14, 230);
    centered_label(speed_tile, c"km/h", jp28, TEXT_DIM, 0, 104, 230);

    // ギア
    let gear_tile = rect(screen, 570, 44, 214, 150, TILE, 10);
    label(gear_tile, ICON_GEAR, icon36, TEXT_DIM, 44, 6);
    label(gear_tile, c"ギア", jp28, TEXT_DIM, 90, 8);
    let gear_label = centered_label(gear_tile, c"--", num96, GEAR_INVALID, 0, 56, 214);

    // 水温・油温
    let water_tile = rect(screen, 330, 204, 230, 150, TILE, 10);
    label(water_tile, ICON_WATER_TEMP, icon36, TEXT_DIM, 52, 6);
    label(water_tile, c"水温", jp28, TEXT_DIM, 98, 8);
    let water_label = centered_label(water_tile, c"--", num72, ORANGE, 0, 50, 230);
    centered_label(water_tile, c"°C", jp28, TEXT_DIM, 0, 116, 230);
    let oil_tile = rect(screen, 570, 204, 214, 150, TILE, 10);
    label(oil_tile, ICON_OIL, icon36, TEXT_DIM, 44, 6);
    label(oil_tile, c"油温", jp28, TEXT_DIM, 90, 8);
    let oil_label = centered_label(oil_tile, c"--", num72, ORANGE, 0, 50, 214);
    centered_label(oil_tile, c"°C", jp28, TEXT_DIM, 0, 116, 214);

    // アクセル開度：0%青・50%黄・100%赤の固定グラデーションを、実際の開度まで表示する。
    // 全幅のグラデーションの上に、未充填部分を暗色で覆う（塗り幅の中に全色を圧縮しない）
    label(screen, c"アクセル開度", jp20, TEXT_DIM, 16, 344);
    let accel_label = centered_label(screen, c"--", jp28, WHITE, 206, 338, 100);
    let bar_y = 372;
    let half = ACCEL_BAR_W / 2;
    let left = rect(screen, 16, bar_y, half, 22, 0x2F6BFF, 0);
    lv_obj_set_style_bg_grad_color(left, color(0xFFD23F), 0);
    lv_obj_set_style_bg_grad_dir(left, LV_GRAD_DIR_HOR as lv_grad_dir_t, 0);
    let right = rect(screen, 16 + half, bar_y, half, 22, 0xFFD23F, 0);
    lv_obj_set_style_bg_grad_color(right, color(RED), 0);
    lv_obj_set_style_bg_grad_dir(right, LV_GRAD_DIR_HOR as lv_grad_dir_t, 0);
    let accel_mask = rect(screen, 16, bar_y, ACCEL_BAR_W, 22, TRACK, 0);

    // 切り替えボタン・記録停止（この段階では見た目のみ。切り替えは次の作業）
    let nav: [(&CStr, &CStr, bool); 4] = [
        (ICON_SPEED, c"総合", true),
        (ICON_THERMOSTAT, c"温度", false),
        (ICON_RACE, c"レース", false),
        (ICON_LOCATION, c"GPS", false),
    ];
    for (i, (icon, name, active)) in nav.iter().enumerate() {
        let x = 16 + i as i32 * 144;
        let (bg, fg) = if *active { (CYAN, BG) } else { (TILE, TEXT_DIM) };
        let b = rect(screen, x, 410, 136, 56, bg, 8);
        label(b, icon, icon28, fg, 10, 13);
        label(b, name, jp28, fg, 44, 10);
    }
    let stop = rect(screen, 608, 410, 176, 56, RED, 8);
    label(stop, ICON_STOP, icon28, WHITE, 14, 13);
    label(stop, c"記録停止", jp28, WHITE, 50, 10);

    let notice_cfg = ShiftNoticeConfig::new(NOTICE_ACTIVATE_RPM, NOTICE_RELEASE_RPM, NOTICE_BLINK_MS)
        .expect("回転数通知の設定が不正");
    *UI.lock().unwrap() = Some(Ui {
        rpm_arc,
        rpm_label,
        speed_label,
        gear_label,
        water_label,
        oil_label,
        accel_label,
        accel_mask,
        data: VehicleData::default(),
        generator: DemoGenerator::new(DemoMode::Sweep),
        notice: ShiftNotice::new(notice_cfg),
        last_rpm: String::new(),
        last_speed: String::new(),
        last_gear: String::new(),
        last_water: String::new(),
        last_oil: String::new(),
        last_accel: String::new(),
        last_gear_color: None,
    });
    lv_timer_create(Some(on_timer), 100, null_mut());
}

/// 100msごとに、仮データを生成して画面を更新する（LVGLのタスク内で実行される）。
unsafe extern "C" fn on_timer(_timer: *mut lv_timer_t) {
    let mut guard = UI.lock().unwrap();
    let Some(ui) = guard.as_mut() else { return };
    let now_ms = u64::from(lv_tick_get());

    ui.generator.tick(now_ms, &mut ui.data);
    let snap = ui.data.snapshot(now_ms, &Limits::provisional());

    // 回転数
    let rpm_text = value_or_dash(snap.rpm);
    if rpm_text != ui.last_rpm {
        set_text(ui.rpm_label, &rpm_text);
        lv_arc_set_value(ui.rpm_arc, snap.rpm.unwrap_or(0) as i16);
        ui.last_rpm = rpm_text;
    }
    // 車速
    update_text(ui.speed_label, value_or_dash(snap.speed_kmh), &mut ui.last_speed);
    update_text(ui.water_label, value_or_dash(snap.water_temp_c), &mut ui.last_water);
    update_text(ui.oil_label, value_or_dash(snap.oil_temp_c), &mut ui.last_oil);

    // ギアと回転数通知
    let gear_text = value_or_dash(snap.gear);
    update_text(ui.gear_label, gear_text, &mut ui.last_gear);
    let display = ui.notice.update(snap.rpm, snap.gear.is_some(), now_ms);
    if ui.last_gear_color != Some(display) {
        let hex = match display {
            GearDisplay::Normal => WHITE,
            GearDisplay::NoticeBright => GEAR_BRIGHT,
            GearDisplay::NoticeDim => GEAR_DIM,
            GearDisplay::Invalid => GEAR_INVALID,
        };
        lv_obj_set_style_text_color(ui.gear_label, color(hex), 0);
        ui.last_gear_color = Some(display);
    }

    // アクセル開度
    let accel_text = match snap.accel_pct {
        Some(p) => format!("{p}%"),
        None => "--".to_string(),
    };
    if accel_text != ui.last_accel {
        set_text(ui.accel_label, &accel_text);
        // 未充填部分（右側）の幅。無効なときは全体を暗色にする
        let filled = i32::from(snap.accel_pct.unwrap_or(0));
        let mask_w = ACCEL_BAR_W * (100 - filled) / 100;
        lv_obj_set_pos(ui.accel_mask, (16 + ACCEL_BAR_W - mask_w) as lv_coord_t, 372);
        lv_obj_set_width(ui.accel_mask, mask_w as lv_coord_t);
        ui.last_accel = accel_text;
    }
}

unsafe fn update_text(l: *mut lv_obj_t, text: String, last: &mut String) {
    if text != *last {
        set_text(l, &text);
        *last = text;
    }
}

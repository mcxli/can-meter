//! メーター画面（総合・温度重視・レース・GPS）。LVGLのC APIを `unsafe` で呼ぶ。
//!
//! 表示するデータは `shared` の仮データ生成（`DemoGenerator`）から得る。
//! 値の判定（無効値の「--」、回転数通知）は `shared` のロジックに任せ、ここでは描画だけを行う。
//! LVGLのAPIはスレッドセーフではないため、呼び出し側が `lvgl_port_lock` を保持した状態で使う
//! （タイマー・イベントのコールバックはLVGLのタスク内で実行されるため、ロック済み）。

use esp_idf_svc::sys::*;
use shared::demo::{DemoGenerator, DemoMode};
use shared::shift_notice::{GearDisplay, ShiftNotice, ShiftNoticeConfig};
use shared::vehicle::{
    format_degrees, format_time, value_or_dash, Limits, VehicleData, VehicleSnapshot,
};
use std::ffi::{c_void, CStr, CString};
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
const BLUE: u32 = 0x2F6BFF;
const YELLOW: u32 = 0xFFD23F;
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

/// 仮データで動作していることを画面に出す（右上のタグ。タップで仮データの種類を切り替える）。
/// 実データに切り替えるとき（段階7）は `false` にする。
const SHOW_DEMO_TAG: bool = true;

const PAGE_COUNT: usize = 4;
const PAGE_GENERAL: usize = 0;

const ACCEL_BAR_W: i32 = 290;

// 回転数メーター（総合画面）の形状（画面上の位置・大きさ）
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

// 温度メーター（温度重視画面）の形状。上半分の半円（180度）
const TEMP_ARC_SIZE: i32 = 356;
const TEMP_ARC_W: i32 = 16;
const TEMP_CY: i32 = 282;
// 左右のメーターの中心のx座標（画面を左右に分けた各領域の中央）
const TEMP_CX_WATER: i32 = 202;
const TEMP_CX_OIL: i32 = 594;

/// 温度メーターの範囲と、色の区切り。
///
/// 値は仮のもの（一般的な目安に合わせた）。警告閾値の確定仕様ではない。
/// 段階7で、実車のCANデータで取りうる範囲を確認して確定する。
#[derive(Clone, Copy)]
struct TempScale {
    min: i32,
    max: i32,
    orange_from: i32,
    red_from: i32,
}

/// 水温：平常は90〜100℃前後、高負荷でも110〜120℃程度までを想定した範囲
const WATER_SCALE: TempScale = TempScale { min: 50, max: 130, orange_from: 105, red_from: 115 };
/// 油温：平常は90〜110℃前後、高負荷で130〜140℃を超えることがある範囲
const OIL_SCALE: TempScale = TempScale { min: 50, max: 150, orange_from: 115, red_from: 130 };

// レース画面の回転数バー
const BAR_X: i32 = 16;
const BAR_Y: i32 = 62;
const BAR_W: i32 = 768;
const BAR_H: i32 = 34;

/// 回転数を、LVGLの円弧の角度（0〜359度）に換算する。
fn rpm_to_deg(rpm: i32) -> i32 {
    (ARC_START_DEG + 270 * rpm / RPM_MAX as i32) % 360
}

/// 温度を、半円の角度（180〜360度）に換算する。範囲外は端に丸める。
fn temp_to_deg(scale: TempScale, temp: i32) -> i32 {
    let t = temp.clamp(scale.min, scale.max);
    180 + 180 * (t - scale.min) / (scale.max - scale.min)
}

fn deg_cos_sin(deg: i32) -> (f32, f32) {
    let rad = (deg as f32).to_radians();
    (rad.cos(), rad.sin())
}

/// ラベルと、直前に表示した文字列（変化したときだけ描画を更新するため）。
struct Txt {
    obj: *mut lv_obj_t,
    last: String,
}

impl Txt {
    fn new(obj: *mut lv_obj_t) -> Self {
        Self { obj, last: String::new() }
    }

    unsafe fn set(&mut self, text: String) {
        if text != self.last {
            set_text(self.obj, &text);
            self.last = text;
        }
    }
}

/// 針（2点の線）。点の配列はLVGLが参照し続けるため、解放しない。
struct Needle {
    line: *mut lv_obj_t,
    points: *mut [lv_point_t; 2],
}

struct NavButton {
    rect: *mut lv_obj_t,
    icon: *mut lv_obj_t,
    text: *mut lv_obj_t,
}

struct GeneralPage {
    rpm_arc: *mut lv_obj_t,
    rpm: Txt,
    speed: Txt,
    gear: Txt,
    water: Txt,
    oil: Txt,
    accel: Txt,
    accel_mask: *mut lv_obj_t,
}

struct TempPage {
    water: Txt,
    oil: Txt,
    water_needle: Needle,
    oil_needle: Needle,
}

struct RacePage {
    rpm_bar: *mut lv_obj_t,
    rpm: Txt,
    gear: Txt,
    speed: Txt,
    water: Txt,
    oil: Txt,
}

struct GpsPage {
    speed: Txt,
    lat: Txt,
    lon: Txt,
    time: Txt,
}

/// 画面の部品へのポインタと、表示する仮データの状態。
struct Ui {
    pages: [*mut lv_obj_t; PAGE_COUNT],
    nav: [NavButton; PAGE_COUNT],
    current: usize,
    demo_tag: Option<Txt>,
    general: GeneralPage,
    temp: TempPage,
    race: RacePage,
    gps: GpsPage,
    data: VehicleData,
    generator: DemoGenerator,
    notice: ShiftNotice,
    /// 総合・レースのギア表示に最後に適用した色の状態（変化したときだけ更新する）
    gear_applied: [Option<GearDisplay>; 2],
}

// LVGLのオブジェクトは、LVGLのロックを保持して操作するため、スレッド間で移してよい
unsafe impl Send for Ui {}

static UI: Mutex<Option<Ui>> = Mutex::new(None);

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

/// 画面全体の透明なコンテナ（ページ）。クリックは下の部品へ通さず、何も受けない。
unsafe fn page(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    let o = lv_obj_create(parent);
    lv_obj_set_pos(o, 0, 0);
    lv_obj_set_size(o, 800, 480);
    lv_obj_set_style_bg_opa(o, 0, 0);
    lv_obj_set_style_border_width(o, 0, 0);
    no_pad_no_scroll(o);
    lv_obj_clear_flag(o, LV_OBJ_FLAG_CLICKABLE as lv_obj_flag_t);
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

/// 円弧（つまみなし・操作不可）。
unsafe fn make_arc(parent: *mut lv_obj_t, x: i32, y: i32, size: i32, width: i32) -> *mut lv_obj_t {
    let arc = lv_arc_create(parent);
    lv_obj_set_pos(arc, x as lv_coord_t, y as lv_coord_t);
    lv_obj_set_size(arc, size as lv_coord_t, size as lv_coord_t);
    lv_obj_remove_style(arc, null_mut(), LV_PART_KNOB as lv_style_selector_t);
    lv_obj_clear_flag(arc, LV_OBJ_FLAG_CLICKABLE as lv_obj_flag_t);
    for part in [LV_PART_MAIN, LV_PART_INDICATOR] {
        lv_obj_set_style_arc_width(arc, width as lv_coord_t, part as lv_style_selector_t);
        lv_obj_set_style_arc_rounded(arc, false, part as lv_style_selector_t);
    }
    arc
}

/// 背景だけを描く円弧（現在値の円弧は透明）。`start`〜`end` 度の範囲を `fg` で塗る。
unsafe fn band_arc(parent: *mut lv_obj_t, x: i32, y: i32, size: i32, width: i32, start: i32, end: i32, fg: u32) {
    let arc = make_arc(parent, x, y, size, width);
    lv_arc_set_bg_angles(arc, start as u16, end as u16);
    lv_obj_set_style_arc_color(arc, color(fg), LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_arc_opa(arc, 0, LV_PART_INDICATOR as lv_style_selector_t);
}

/// 2点を結ぶ線。点の座標は、LVGLが保持し続けるため、解放しない（画面と同じ寿命）。
unsafe fn line(parent: *mut lv_obj_t, p0: (f32, f32), p1: (f32, f32), width: i32, fg: u32) -> Needle {
    let points: *mut [lv_point_t; 2] = Box::leak(Box::new([lv_point_t { x: 0, y: 0 }; 2]));
    let obj = lv_line_create(parent);
    lv_obj_set_style_line_width(obj, width as lv_coord_t, 0);
    lv_obj_set_style_line_color(obj, color(fg), 0);
    lv_obj_set_style_line_rounded(obj, false, 0);
    let n = Needle { line: obj, points };
    move_line(&n, p0, p1);
    n
}

/// 線の端点を動かす。
unsafe fn move_line(n: &Needle, p0: (f32, f32), p1: (f32, f32)) {
    let (min_x, min_y) = (p0.0.min(p1.0).floor(), p0.1.min(p1.1).floor());
    (*n.points)[0] = lv_point_t { x: (p0.0 - min_x) as lv_coord_t, y: (p0.1 - min_y) as lv_coord_t };
    (*n.points)[1] = lv_point_t { x: (p1.0 - min_x) as lv_coord_t, y: (p1.1 - min_y) as lv_coord_t };
    lv_line_set_points(n.line, (*n.points).as_ptr(), 2);
    lv_obj_set_pos(n.line, min_x as lv_coord_t, min_y as lv_coord_t);
}

unsafe fn tick(parent: *mut lv_obj_t, p0: (f32, f32), p1: (f32, f32), width: i32, fg: u32) {
    line(parent, p0, p1, width, fg);
}

unsafe fn set_text(l: *mut lv_obj_t, s: &str) {
    if let Ok(c) = CString::new(s) {
        lv_label_set_text(l, c.as_ptr());
    }
}

/// 全画面で共通のフォントへのポインタ。
struct Fonts {
    jp20: *const lv_font_t,
    jp28: *const lv_font_t,
    num56: *const lv_font_t,
    num72: *const lv_font_t,
    num96: *const lv_font_t,
    num128: *const lv_font_t,
    icon28: *const lv_font_t,
    icon36: *const lv_font_t,
}

impl Fonts {
    unsafe fn get() -> Self {
        Self {
            jp20: &jp_20,
            jp28: &jp_28,
            num56: &num_56,
            num72: &num_72,
            num96: &num_96,
            num128: &num_128,
            icon28: &icons_28,
            icon36: &icons_36,
        }
    }
}

/// 画面を作り、100msごとに仮データで更新するタイマーを開始する。
///
/// `lvgl_port_lock` を保持した状態で呼ぶこと。
pub unsafe fn build_screens() {
    let f = Fonts::get();
    let screen = lv_disp_get_scr_act(lv_disp_get_default());
    lv_obj_set_style_bg_color(screen, color(BG), 0);
    lv_obj_set_style_bg_opa(screen, 255, 0);
    no_pad_no_scroll(screen);

    // ページは、共通部分（状態表示・ボタン）より先に作り、共通部分を手前に置く
    let pages = [page(screen), page(screen), page(screen), page(screen)];
    let general = build_general(pages[0], &f);
    let temp = build_temp(pages[1], &f);
    let race = build_race(pages[2], &f);
    let gps = build_gps(pages[3], &f);
    for p in &pages[1..] {
        lv_obj_add_flag(*p, LV_OBJ_FLAG_HIDDEN as lv_obj_flag_t);
    }

    // 状態表示（仮データのため、すべて正常として表示する）
    for (i, name) in [c"CAN RX", c"GPS FIX", c"Wi-Fi", c"SD REC"].iter().enumerate() {
        let x = 16 + i as i32 * 150;
        rect(screen, x, 14, 12, 12, GREEN, 6);
        label(screen, name, f.jp20, TEXT_DIM, x + 20, 5);
    }
    let demo_tag = if SHOW_DEMO_TAG {
        let tag = rect(screen, 616, 4, 170, 30, BG, 4);
        lv_obj_set_style_border_width(tag, 2, 0);
        lv_obj_set_style_border_color(tag, color(CYAN), 0);
        let l = centered_label(tag, c"DEMO", f.jp20, CYAN, 0, 2, 166);
        lv_obj_add_event_cb(
            tag,
            Some(on_demo_tag),
            lv_event_code_t_LV_EVENT_CLICKED,
            null_mut(),
        );
        Some(Txt::new(l))
    } else {
        None
    };

    // 切り替えボタン・記録停止
    let nav_defs: [(&CStr, &CStr); PAGE_COUNT] = [
        (ICON_SPEED, c"総合"),
        (ICON_THERMOSTAT, c"温度"),
        (ICON_RACE, c"レース"),
        (ICON_LOCATION, c"GPS"),
    ];
    let nav_buttons: [NavButton; PAGE_COUNT] = std::array::from_fn(|i| {
        let (icon, name) = nav_defs[i];
        let x = 16 + i as i32 * 144;
        let rect_obj = rect(screen, x, 410, 136, 56, TILE, 8);
        let icon_obj = label(rect_obj, icon, f.icon28, TEXT_DIM, 10, 13);
        let text_obj = label(rect_obj, name, f.jp28, TEXT_DIM, 44, 10);
        lv_obj_add_event_cb(
            rect_obj,
            Some(on_nav),
            lv_event_code_t_LV_EVENT_CLICKED,
            i as *mut c_void,
        );
        NavButton { rect: rect_obj, icon: icon_obj, text: text_obj }
    });
    // 記録停止（この段階では見た目のみ。記録の実装は段階5）
    let stop = rect(screen, 608, 410, 176, 56, RED, 8);
    label(stop, ICON_STOP, f.icon28, WHITE, 14, 13);
    label(stop, c"記録停止", f.jp28, WHITE, 50, 10);

    let notice_cfg = ShiftNoticeConfig::new(NOTICE_ACTIVATE_RPM, NOTICE_RELEASE_RPM, NOTICE_BLINK_MS)
        .expect("回転数通知の設定が不正");
    let mut ui = Ui {
        pages,
        nav: nav_buttons,
        current: PAGE_GENERAL,
        demo_tag,
        general,
        temp,
        race,
        gps,
        data: VehicleData::default(),
        generator: DemoGenerator::new(DemoMode::Sweep),
        notice: ShiftNotice::new(notice_cfg),
        gear_applied: [None; 2],
    };
    show_page(&mut ui, PAGE_GENERAL);
    update_demo_tag(&mut ui);
    *UI.lock().unwrap() = Some(ui);
    lv_timer_create(Some(on_timer), 100, null_mut());
}

/// 総合画面：回転数メーター、車速、ギア、水温、油温、アクセル開度。
unsafe fn build_general(p: *mut lv_obj_t, f: &Fonts) -> GeneralPage {
    // 回転数メーター（0〜8000rpm、270度）。下から順に、背景、7000rpm以上の赤い帯、現在値の円弧を重ねる
    band_arc(p, ARC_X, ARC_Y, ARC_SIZE, ARC_W, ARC_START_DEG, ARC_END_DEG, TRACK);
    band_arc(p, ARC_X, ARC_Y, ARC_SIZE, ARC_W, rpm_to_deg(RPM_REDLINE_FROM), ARC_END_DEG, RED);
    let rpm_arc = make_arc(p, ARC_X, ARC_Y, ARC_SIZE, ARC_W);
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
        tick(p, p0, p1, if major { 3 } else { 2 }, if major { WHITE } else { TEXT_DIM });
        if major {
            let r_label = r_in - 14.0;
            let text = CString::new((rpm / 1000).to_string()).unwrap();
            let (lx, ly) = (ARC_CX + r_label * cos, ARC_CY + r_label * sin);
            // 1桁の数字（jp_20）の中心を目盛りの位置に合わせる
            centered_label(p, &text, f.jp20, WHITE, lx as i32 - 10, ly as i32 - 14, 20);
        }
    }
    // 中央の数値は、目盛りの数字と重ならないよう、小さいフォントで少し下に置く
    let rpm = Txt::new(centered_label(p, c"--", f.num56, WHITE, 16, 148, 290));
    centered_label(p, c"rpm", f.jp20, TEXT_DIM, 16, 212, 290);

    // 車速
    let speed_tile = rect(p, 330, 44, 230, 150, TILE, 10);
    let speed = Txt::new(centered_label(speed_tile, c"--", f.num96, WHITE, 0, 14, 230));
    centered_label(speed_tile, c"km/h", f.jp28, TEXT_DIM, 0, 104, 230);

    // ギア
    let gear_tile = rect(p, 570, 44, 214, 150, TILE, 10);
    label(gear_tile, ICON_GEAR, f.icon36, TEXT_DIM, 44, 6);
    label(gear_tile, c"ギア", f.jp28, TEXT_DIM, 90, 8);
    let gear = Txt::new(centered_label(gear_tile, c"--", f.num96, GEAR_INVALID, 0, 56, 214));

    // 水温・油温
    let water_tile = rect(p, 330, 204, 230, 150, TILE, 10);
    label(water_tile, ICON_WATER_TEMP, f.icon36, TEXT_DIM, 52, 6);
    label(water_tile, c"水温", f.jp28, TEXT_DIM, 98, 8);
    let water = Txt::new(centered_label(water_tile, c"--", f.num72, ORANGE, 0, 50, 230));
    centered_label(water_tile, c"°C", f.jp28, TEXT_DIM, 0, 116, 230);
    let oil_tile = rect(p, 570, 204, 214, 150, TILE, 10);
    label(oil_tile, ICON_OIL, f.icon36, TEXT_DIM, 44, 6);
    label(oil_tile, c"油温", f.jp28, TEXT_DIM, 90, 8);
    let oil = Txt::new(centered_label(oil_tile, c"--", f.num72, ORANGE, 0, 50, 214));
    centered_label(oil_tile, c"°C", f.jp28, TEXT_DIM, 0, 116, 214);

    // アクセル開度：0%青・50%黄・100%赤の固定グラデーションを、実際の開度まで表示する。
    // 全幅のグラデーションの上に、未充填部分を暗色で覆う（塗り幅の中に全色を圧縮しない）
    label(p, c"アクセル開度", f.jp20, TEXT_DIM, 16, 344);
    let accel = Txt::new(centered_label(p, c"--", f.jp28, WHITE, 206, 338, 100));
    let half = ACCEL_BAR_W / 2;
    let left = rect(p, 16, 372, half, 22, BLUE, 0);
    lv_obj_set_style_bg_grad_color(left, color(YELLOW), 0);
    lv_obj_set_style_bg_grad_dir(left, LV_GRAD_DIR_HOR as lv_grad_dir_t, 0);
    let right = rect(p, 16 + half, 372, half, 22, YELLOW, 0);
    lv_obj_set_style_bg_grad_color(right, color(RED), 0);
    lv_obj_set_style_bg_grad_dir(right, LV_GRAD_DIR_HOR as lv_grad_dir_t, 0);
    let accel_mask = rect(p, 16, 372, ACCEL_BAR_W, 22, TRACK, 0);

    GeneralPage { rpm_arc, rpm, speed, gear, water, oil, accel, accel_mask }
}

/// 温度メーター1つ（半円）。色は、青・橙・赤の3つの帯。針で現在値を示す。
unsafe fn build_temp_gauge(
    p: *mut lv_obj_t,
    f: &Fonts,
    cx: i32,
    scale: TempScale,
    icon: &CStr,
    title: &CStr,
) -> (Txt, Needle) {
    // 見出し（アイコンと名前）
    label(p, icon, f.icon36, TEXT_DIM, cx - 72, 50);
    label(p, title, f.jp28, WHITE, cx - 26, 52);

    // 色分けの帯（半円の上半分）
    let (x, y) = (cx - TEMP_ARC_SIZE / 2, TEMP_CY - TEMP_ARC_SIZE / 2);
    let zones = [
        (scale.min, scale.orange_from, BLUE),
        (scale.orange_from, scale.red_from, ORANGE),
        (scale.red_from, scale.max, RED),
    ];
    for (from, to, c) in zones {
        band_arc(p, x, y, TEMP_ARC_SIZE, TEMP_ARC_W, temp_to_deg(scale, from), temp_to_deg(scale, to), c);
    }

    // 目盛り（10℃ごとの線、20℃ごとに長い線と数字）
    let r_out = TEMP_ARC_SIZE as f32 / 2.0 - TEMP_ARC_W as f32 - 3.0;
    for t in (scale.min..=scale.max).step_by(10) {
        let major = (t - scale.min) % 20 == 0;
        let (cos, sin) = deg_cos_sin(temp_to_deg(scale, t));
        let r_in = r_out - if major { 14.0 } else { 7.0 };
        let (cxf, cyf) = (cx as f32, TEMP_CY as f32);
        tick(
            p,
            (cxf + r_out * cos, cyf + r_out * sin),
            (cxf + r_in * cos, cyf + r_in * sin),
            if major { 3 } else { 2 },
            if major { WHITE } else { TEXT_DIM },
        );
        if major {
            let r_label = r_in - 18.0;
            let text = CString::new(t.to_string()).unwrap();
            // 2〜3桁の数字（jp_20）の中心を目盛りの位置に合わせる
            centered_label(
                p,
                &text,
                f.jp20,
                WHITE,
                (cxf + r_label * cos) as i32 - 24,
                (cyf + r_label * sin) as i32 - 14,
                48,
            );
        }
    }

    // 針（現在値）。位置は更新時に決める
    let needle = line(p, (0.0, 0.0), (1.0, 1.0), 5, WHITE);
    // 数値と単位
    let value = Txt::new(centered_label(p, c"--", f.num96, WHITE, cx - 120, TEMP_CY - 44, 240));
    centered_label(p, c"°C", f.jp28, TEXT_DIM, cx - 120, TEMP_CY + 46, 240);
    (value, needle)
}

/// 温度重視画面：水温・油温を大きく表示する。
unsafe fn build_temp(p: *mut lv_obj_t, f: &Fonts) -> TempPage {
    let (water, water_needle) = build_temp_gauge(p, f, TEMP_CX_WATER, WATER_SCALE, ICON_WATER_TEMP, c"水温");
    let (oil, oil_needle) = build_temp_gauge(p, f, TEMP_CX_OIL, OIL_SCALE, ICON_OIL, c"油温");
    // 左右の区切り線
    rect(p, 398, 60, 2, 300, TRACK, 0);
    TempPage { water, oil, water_needle, oil_needle }
}

/// レース画面：横長の回転数バー、回転数・ギア・車速、水温・油温。
unsafe fn build_race(p: *mut lv_obj_t, f: &Fonts) -> RacePage {
    // 回転数バー。背景・赤い帯は別の四角形で描き、バーの背景は透明にして、現在値だけを重ねる
    rect(p, BAR_X, BAR_Y, BAR_W, BAR_H, TRACK, 0);
    let red_x = BAR_X + BAR_W * RPM_REDLINE_FROM / RPM_MAX as i32;
    rect(p, red_x, BAR_Y, BAR_X + BAR_W - red_x, BAR_H, RED, 0);
    let rpm_bar = lv_bar_create(p);
    lv_obj_set_pos(rpm_bar, BAR_X as lv_coord_t, BAR_Y as lv_coord_t);
    lv_obj_set_size(rpm_bar, BAR_W as lv_coord_t, BAR_H as lv_coord_t);
    lv_bar_set_range(rpm_bar, 0, RPM_MAX as i32);
    lv_bar_set_value(rpm_bar, 0, 0);
    lv_obj_set_style_bg_opa(rpm_bar, 0, LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_radius(rpm_bar, 0, LV_PART_MAIN as lv_style_selector_t);
    lv_obj_set_style_radius(rpm_bar, 0, LV_PART_INDICATOR as lv_style_selector_t);
    lv_obj_set_style_bg_color(rpm_bar, color(CYAN), LV_PART_INDICATOR as lv_style_selector_t);
    lv_obj_set_style_bg_opa(rpm_bar, 255, LV_PART_INDICATOR as lv_style_selector_t);
    // 目盛り（500rpmごとの線、1000rpmごとに長い線と数字）
    for i in 0..=(RPM_MAX as i32 / 500) {
        let rpm = i * 500;
        let major = rpm % 1000 == 0;
        let x = (BAR_X + BAR_W * rpm / RPM_MAX as i32) as f32;
        let y0 = (BAR_Y + BAR_H + 4) as f32;
        tick(p, (x, y0), (x, y0 + if major { 12.0 } else { 6.0 }), if major { 3 } else { 2 }, if major { WHITE } else { TEXT_DIM });
        if major {
            let text = CString::new((rpm / 1000).to_string()).unwrap();
            centered_label(p, &text, f.jp20, WHITE, x as i32 - 10, BAR_Y + BAR_H + 18, 20);
        }
    }
    label(p, c"x1000 rpm", f.jp20, TEXT_DIM, 660, BAR_Y + BAR_H + 44);

    // 回転数・ギア・車速（大きく）。列の間に区切り線。
    // 目盛りの数字・「x1000 rpm」（y=114〜170）と重ならないよう、y=176から置く
    let rpm = Txt::new(centered_label(p, c"--", f.num96, WHITE, 16, 176, 250));
    centered_label(p, c"rpm", f.jp28, TEXT_DIM, 16, 254, 250);
    rect(p, 280, 176, 2, 112, TRACK, 0);
    // ギアは、他の数値より一段大きいフォントで表示する
    let gear = Txt::new(centered_label(p, c"--", f.num128, GEAR_INVALID, 290, 178, 220));
    rect(p, 518, 176, 2, 112, TRACK, 0);
    let speed = Txt::new(centered_label(p, c"--", f.num96, WHITE, 530, 176, 254));
    centered_label(p, c"km/h", f.jp28, TEXT_DIM, 530, 254, 254);

    // 水温・油温（横並び）。タイルの高さ96pxの中心（y=48）に、各要素の見た目の中心を合わせる：
    //   アイコン(36px)は上端から中心まで17px → y=31
    //   名前(jp_28)の日本語は中心まで約15.5px → y=33
    //   数値(num_56)は中心まで約21.5px → y=27
    //   単位「°C」は、数値の下端（基準線）に揃える。jp_28の基準線は上端から26px、num_56は42px → y=43
    let water_tile = rect(p, 16, 300, 380, 96, TILE, 10);
    label(water_tile, ICON_WATER_TEMP, f.icon36, TEXT_DIM, 16, 31);
    label(water_tile, c"水温", f.jp28, TEXT_DIM, 62, 33);
    let water = Txt::new(centered_label(water_tile, c"--", f.num56, ORANGE, 150, 27, 150));
    label(water_tile, c"°C", f.jp28, TEXT_DIM, 306, 43);
    let oil_tile = rect(p, 404, 300, 380, 96, TILE, 10);
    label(oil_tile, ICON_OIL, f.icon36, TEXT_DIM, 16, 31);
    label(oil_tile, c"油温", f.jp28, TEXT_DIM, 62, 33);
    let oil = Txt::new(centered_label(oil_tile, c"--", f.num56, ORANGE, 150, 27, 150));
    label(oil_tile, c"°C", f.jp28, TEXT_DIM, 306, 43);

    RacePage { rpm_bar, rpm, gear, speed, water, oil }
}

/// GPS画面：GPS速度、緯度・経度、日本時間。
unsafe fn build_gps(p: *mut lv_obj_t, f: &Fonts) -> GpsPage {
    let speed_tile = rect(p, 16, 60, 360, 300, TILE, 10);
    label(speed_tile, c"GPS速度", f.jp28, WHITE, 24, 16);
    let speed = Txt::new(centered_label(speed_tile, c"--", f.num96, WHITE, 0, 100, 360));
    centered_label(speed_tile, c"km/h", f.jp28, TEXT_DIM, 0, 220, 360);

    let info = rect(p, 392, 60, 392, 300, TILE, 10);
    label(info, c"緯度", f.jp20, TEXT_DIM, 28, 20);
    let lat = Txt::new(label(info, c"--", f.jp28, WHITE, 28, 48));
    label(info, c"経度", f.jp20, TEXT_DIM, 28, 112);
    let lon = Txt::new(label(info, c"--", f.jp28, WHITE, 28, 140));
    rect(info, 28, 196, 336, 2, TRACK, 0);
    label(info, c"時刻 (JST)", f.jp20, TEXT_DIM, 28, 212);
    let time = Txt::new(label(info, c"--", f.jp28, WHITE, 28, 240));

    GpsPage { speed, lat, lon, time }
}

/// 表示するページを切り替え、ボタンの強調を更新する。
unsafe fn show_page(ui: &mut Ui, index: usize) {
    ui.current = index;
    for i in 0..PAGE_COUNT {
        if i == index {
            lv_obj_clear_flag(ui.pages[i], LV_OBJ_FLAG_HIDDEN as lv_obj_flag_t);
        } else {
            lv_obj_add_flag(ui.pages[i], LV_OBJ_FLAG_HIDDEN as lv_obj_flag_t);
        }
        let (bg, fg) = if i == index { (CYAN, BG) } else { (TILE, TEXT_DIM) };
        let b = &ui.nav[i];
        lv_obj_set_style_bg_color(b.rect, color(bg), 0);
        lv_obj_set_style_text_color(b.icon, color(fg), 0);
        lv_obj_set_style_text_color(b.text, color(fg), 0);
    }
}

unsafe fn update_demo_tag(ui: &mut Ui) {
    let name = ui.generator.mode().name();
    if let Some(tag) = ui.demo_tag.as_mut() {
        tag.set(format!("DEMO:{name}"));
    }
}

/// 画面切り替えボタンが押された。
unsafe extern "C" fn on_nav(e: *mut lv_event_t) {
    let index = lv_event_get_user_data(e) as usize;
    if index >= PAGE_COUNT {
        return;
    }
    if let Some(ui) = UI.lock().unwrap().as_mut() {
        show_page(ui, index);
    }
}

/// 右上のタグが押された：仮データの種類を、次へ切り替える（無効値・異常値・更新停止の表示確認用）。
unsafe extern "C" fn on_demo_tag(_e: *mut lv_event_t) {
    if let Some(ui) = UI.lock().unwrap().as_mut() {
        ui.generator = DemoGenerator::new(ui.generator.mode().next());
        ui.data = VehicleData::default(); // 前の種類の値を引き継がない
        update_demo_tag(ui);
    }
}

/// 100msごとに、仮データを生成して画面を更新する（LVGLのタスク内で実行される）。
unsafe extern "C" fn on_timer(_timer: *mut lv_timer_t) {
    let mut guard = UI.lock().unwrap();
    let Some(ui) = guard.as_mut() else { return };
    let now_ms = u64::from(lv_tick_get());

    ui.generator.tick(now_ms, &mut ui.data);
    let snap = ui.data.snapshot(now_ms, &Limits::provisional());

    // 表示中のページだけ更新する（非表示のページは、表示したときに最新の状態へ更新される）
    match ui.current {
        0 => update_general(&mut ui.general, &snap),
        1 => update_temp(&mut ui.temp, &snap),
        2 => update_race(&mut ui.race, &snap),
        _ => update_gps(&mut ui.gps, &snap),
    }

    // ギアの色（回転数通知）。総合・レースの両画面に同じ状態を使い、切り替えでリセットしない
    let display = ui.notice.update(snap.rpm, snap.gear.is_some(), now_ms);
    let hex = match display {
        GearDisplay::Normal => WHITE,
        GearDisplay::NoticeBright => GEAR_BRIGHT,
        GearDisplay::NoticeDim => GEAR_DIM,
        GearDisplay::Invalid => GEAR_INVALID,
    };
    for (i, obj) in [ui.general.gear.obj, ui.race.gear.obj].into_iter().enumerate() {
        if ui.gear_applied[i] != Some(display) {
            lv_obj_set_style_text_color(obj, color(hex), 0);
            ui.gear_applied[i] = Some(display);
        }
    }
}

unsafe fn update_general(g: &mut GeneralPage, s: &VehicleSnapshot) {
    let rpm_text = value_or_dash(s.rpm);
    if rpm_text != g.rpm.last {
        lv_arc_set_value(g.rpm_arc, s.rpm.unwrap_or(0) as i16);
    }
    g.rpm.set(rpm_text);
    g.speed.set(value_or_dash(s.speed_kmh));
    g.water.set(value_or_dash(s.water_temp_c));
    g.oil.set(value_or_dash(s.oil_temp_c));
    g.gear.set(value_or_dash(s.gear));
    let accel_text = match s.accel_pct {
        Some(p) => format!("{p}%"),
        None => "--".to_string(),
    };
    if accel_text != g.accel.last {
        // 未充填部分（右側）の幅。無効なときは全体を暗色にする
        let filled = i32::from(s.accel_pct.unwrap_or(0));
        let mask_w = ACCEL_BAR_W * (100 - filled) / 100;
        lv_obj_set_pos(g.accel_mask, (16 + ACCEL_BAR_W - mask_w) as lv_coord_t, 372);
        lv_obj_set_width(g.accel_mask, mask_w as lv_coord_t);
    }
    g.accel.set(accel_text);
}

unsafe fn update_temp(t: &mut TempPage, s: &VehicleSnapshot) {
    t.water.set(value_or_dash(s.water_temp_c));
    t.oil.set(value_or_dash(s.oil_temp_c));
    update_needle(&t.water_needle, TEMP_CX_WATER, WATER_SCALE, s.water_temp_c);
    update_needle(&t.oil_needle, TEMP_CX_OIL, OIL_SCALE, s.oil_temp_c);
}

/// 針を現在値の位置へ動かす。無効なときは隠す。
unsafe fn update_needle(n: &Needle, cx: i32, scale: TempScale, temp: Option<i16>) {
    let Some(temp) = temp else {
        lv_obj_add_flag(n.line, LV_OBJ_FLAG_HIDDEN as lv_obj_flag_t);
        return;
    };
    lv_obj_clear_flag(n.line, LV_OBJ_FLAG_HIDDEN as lv_obj_flag_t);
    let (cos, sin) = deg_cos_sin(temp_to_deg(scale, i32::from(temp)));
    let r_out = TEMP_ARC_SIZE as f32 / 2.0 - TEMP_ARC_W as f32 - 3.0;
    let (cxf, cyf) = (cx as f32, TEMP_CY as f32);
    move_line(
        n,
        (cxf + (r_out - 32.0) * cos, cyf + (r_out - 32.0) * sin),
        (cxf + (r_out + 4.0) * cos, cyf + (r_out + 4.0) * sin),
    );
}

unsafe fn update_race(r: &mut RacePage, s: &VehicleSnapshot) {
    lv_bar_set_value(r.rpm_bar, i32::from(s.rpm.unwrap_or(0)), 0);
    r.rpm.set(value_or_dash(s.rpm));
    r.gear.set(value_or_dash(s.gear));
    r.speed.set(value_or_dash(s.speed_kmh));
    r.water.set(value_or_dash(s.water_temp_c));
    r.oil.set(value_or_dash(s.oil_temp_c));
}

unsafe fn update_gps(g: &mut GpsPage, s: &VehicleSnapshot) {
    match s.gps {
        Some(fix) => {
            g.speed.set(fix.speed_kmh.to_string());
            g.lat.set(format_degrees(fix.lat_e6));
            g.lon.set(format_degrees(fix.lon_e6));
            g.time.set(format_time(fix.time_s));
        }
        None => {
            g.speed.set("--".to_string());
            g.lat.set("--".to_string());
            g.lon.set("--".to_string());
            g.time.set("--".to_string());
        }
    }
}

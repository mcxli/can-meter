//! CANフレームから車両情報を取り出す（復号）。**候補の実装**。
//!
//! ID・ビット位置・換算は、公開情報の調査（`docs/実装仕様/CAN情報の調査.md`、要件 CAN-03）による候補で、
//! 実車（2014年式BRZ ZC6・MT）の生ログで照合するまで、確定ではない（要件 CAN-07）。照合の結果に合わせて、
//! この表を直す。換算が出典間で食い違う項目・意味が不明確な項目は、各関数のコメントに記す。
//!
//! 受信専用。ここはフレームを読むだけで、送信・診断要求は行わない（要件 CAN-04）。

use crate::can::{CanFrame, FrameKind, IdFormat};
use crate::vehicle::VehicleData;

/// 回転数・アクセル開度・クラッチ（100Hz、出典：ft86 gen1.md）
pub const ID_ENGINE: u32 = 0x140;
/// 油温・水温（20Hz）
pub const ID_TEMPS: u32 = 0x360;
/// 車速・ブレーキ圧（50Hz）
pub const ID_SPEED: u32 = 0xD1;
/// 舵角・ヨーレート・加速度（50Hz）
pub const ID_DYNAMICS: u32 = 0xD0;
/// ブレーキペダル（50Hz）
pub const ID_BRAKE: u32 = 0x144;
/// ギアの生の値・回転数（別系統）（100Hz）
pub const ID_ENGINE_ALT: u32 = 0x141;

/// 1フレームから取り出した値。そのフレームに含まれない項目は `None`。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Decoded {
    pub rpm: Option<u16>,
    pub accel_pct: Option<u8>,
    pub clutch_pressed: Option<bool>,
    pub oil_temp_c: Option<i16>,
    pub water_temp_c: Option<i16>,
    pub speed_kmh: Option<u16>,
    pub steering_deg_x10: Option<i16>,
    pub brake_pressed: Option<bool>,
    /// ギアの生の値（0x141 バイト6の下位4ビット）。意味が未確定のため、ギアには変換しない
    pub gear_raw: Option<u8>,
}

/// リトルエンディアンで、`start` ビットから `len` ビットを取り出す（`len` ≦ 32）。データが足りなければ `None`。
pub fn bits_le(data: &[u8], start: usize, len: usize) -> Option<u32> {
    if len == 0 || len > 32 || start + len > data.len() * 8 {
        return None;
    }
    let mut v: u64 = 0;
    for (i, b) in data
        .iter()
        .enumerate()
        .skip(start / 8)
        .take((start % 8 + len).div_ceil(8))
    {
        v |= u64::from(*b) << (8 * (i - start / 8));
    }
    Some(((v >> (start % 8)) & ((1u64 << len) - 1)) as u32)
}

/// 符号つき16ビット（リトルエンディアン）。
fn i16_le(data: &[u8], byte: usize) -> Option<i16> {
    Some(i16::from_le_bytes([*data.get(byte)?, *data.get(byte + 1)?]))
}

/// 1フレームを復号する。対象外のID、データフレーム以外、拡張フレーム、DLCが短いフレームは、何も取り出さない。
pub fn decode(frame: &CanFrame) -> Decoded {
    let mut d = Decoded::default();
    if frame.kind != FrameKind::Data
        || frame.format != IdFormat::Standard
        || !frame.is_well_formed()
    {
        return d;
    }
    let p = frame.payload();
    match frame.id {
        ID_ENGINE => {
            // 回転数：ビット16から14ビット、換算なし
            d.rpm = bits_le(p, 16, 14).map(|v| v as u16);
            // アクセル開度：バイト0 ÷ 2.55（0〜255 → 0〜100%）。四捨五入
            d.accel_pct = p.first().map(|&a| ((u32::from(a) * 100 + 127) / 255) as u8);
            // クラッチ：バイト1 の最上位ビット（ON/OFFのみ）
            d.clutch_pressed = p.get(1).map(|&b| b & 0x80 != 0);
        }
        ID_TEMPS => {
            // 油温：バイト2 − 40、水温：バイト3 − 40 [℃]。
            // 油温は「raw/2 − 40」とする投稿もある（食い違い）。単位（℃/℉）も出典に記載がない。実車で確認する
            d.oil_temp_c = p.get(2).map(|&c| i16::from(c) - 40);
            d.water_temp_c = p.get(3).map(|&w| i16::from(w) - 40);
        }
        ID_SPEED => {
            // 車速：バイト0〜1（LE）× 0.015694 [km/h]。別の係数（0.05747）の投稿もある。GPSと照合する
            d.speed_kmh = bits_le(p, 0, 16).map(|v| ((v as f32) * 0.015694).round() as u16);
        }
        ID_DYNAMICS => {
            // 舵角：バイト0〜1（LE、符号つき）× −0.1 [度]（正が左）。0.1度単位で保持する。符号は実車で確認する
            d.steering_deg_x10 = i16_le(p, 0).map(|v| v.saturating_neg());
        }
        ID_BRAKE => {
            // ブレーキペダル：バイト6 の 0x08
            d.brake_pressed = p.get(6).map(|&g| g & 0x08 != 0);
        }
        ID_ENGINE_ALT => {
            d.gear_raw = p.get(6).map(|&g| g & 0x0F);
        }
        _ => {}
    }
    d
}

/// 復号した値を、受信時刻 `now_ms` で `VehicleData` に反映する。表示する項目だけを反映する。
/// ギアは、生の値の意味が未確定のため、反映しない（手順書07：推定だけでN・Rを確定しない）。
pub fn apply(d: &Decoded, data: &mut VehicleData, now_ms: u64) {
    if let Some(v) = d.rpm {
        data.rpm.set(v, now_ms);
    }
    if let Some(v) = d.accel_pct {
        data.accel_pct.set(v, now_ms);
    }
    if let Some(v) = d.oil_temp_c {
        data.oil_temp_c.set(v, now_ms);
    }
    if let Some(v) = d.water_temp_c {
        data.water_temp_c.set(v, now_ms);
    }
    if let Some(v) = d.speed_kmh {
        data.speed_kmh.set(v, now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle::Limits;

    fn frame(id: u32, data: &[u8]) -> CanFrame {
        let mut d = [0u8; 8];
        d[..data.len()].copy_from_slice(data);
        CanFrame {
            id,
            format: IdFormat::Standard,
            kind: FrameKind::Data,
            dlc: data.len() as u8,
            data: d,
        }
    }

    #[test]
    fn ビット取り出し() {
        let d = [0x12, 0x34, 0x56, 0x78];
        assert_eq!(bits_le(&d, 0, 8), Some(0x12));
        assert_eq!(bits_le(&d, 8, 16), Some(0x5634));
        assert_eq!(bits_le(&d, 4, 8), Some(0x41));
        assert_eq!(bits_le(&d, 0, 32), Some(0x7856_3412));
        assert_eq!(bits_le(&d, 25, 8), None); // データ不足
        assert_eq!(bits_le(&d, 0, 0), None);
    }

    #[test]
    fn 回転数_公開資料の例_8813は5000rpm() {
        // 出典：canlab-zn6 definitions/evidence.toml「88 13 in bytes 2..3 ... => 5000 rpm」
        let d = decode(&frame(ID_ENGINE, &[0, 0, 0x88, 0x13, 0, 0, 0, 0]));
        assert_eq!(d.rpm, Some(5000));
    }

    #[test]
    fn 回転数は上位2ビットを除く() {
        // 14ビットなので、バイト3の上位2ビット（0xC0）は回転数に含めない
        let d = decode(&frame(ID_ENGINE, &[0, 0, 0x88, 0x13 | 0xC0, 0, 0, 0, 0]));
        assert_eq!(d.rpm, Some(5000));
    }

    #[test]
    fn アクセルとクラッチ() {
        let d = decode(&frame(ID_ENGINE, &[255, 0x80, 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.accel_pct, Some(100));
        assert_eq!(d.clutch_pressed, Some(true));
        let d = decode(&frame(ID_ENGINE, &[0, 0x7F, 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.accel_pct, Some(0));
        assert_eq!(d.clutch_pressed, Some(false));
        let d = decode(&frame(ID_ENGINE, &[61, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.accel_pct, Some(24)); // 61/2.55 = 23.9
    }

    #[test]
    fn 油温と水温() {
        let d = decode(&frame(ID_TEMPS, &[0, 0, 136, 128, 0, 0, 0, 0]));
        assert_eq!(d.oil_temp_c, Some(96));
        assert_eq!(d.water_temp_c, Some(88));
        let d = decode(&frame(ID_TEMPS, &[0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.oil_temp_c, Some(-40));
    }

    #[test]
    fn 車速() {
        // 60km/h ÷ 0.015694 ≒ 3823
        let raw: u16 = 3823;
        let b = raw.to_le_bytes();
        let d = decode(&frame(ID_SPEED, &[b[0], b[1], 0, 0]));
        assert_eq!(d.speed_kmh, Some(60));
        assert_eq!(decode(&frame(ID_SPEED, &[0, 0, 0, 0])).speed_kmh, Some(0));
    }

    #[test]
    fn 舵角は符号つきで向きを反転() {
        let b = (-123i16).to_le_bytes();
        let d = decode(&frame(ID_DYNAMICS, &[b[0], b[1], 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.steering_deg_x10, Some(123)); // −0.1倍：12.3度
        let b = i16::MIN.to_le_bytes();
        let d = decode(&frame(ID_DYNAMICS, &[b[0], b[1], 0, 0, 0, 0, 0, 0]));
        assert_eq!(d.steering_deg_x10, Some(i16::MAX)); // あふれない
    }

    #[test]
    fn ブレーキとギアの生の値() {
        let d = decode(&frame(ID_BRAKE, &[0, 0, 0, 0, 0, 0, 0x08, 0]));
        assert_eq!(d.brake_pressed, Some(true));
        let d = decode(&frame(ID_ENGINE_ALT, &[0, 0, 0, 0, 0, 0, 0xA3, 0]));
        assert_eq!(d.gear_raw, Some(3));
    }

    #[test]
    fn 短いdlcは取り出せる分だけ() {
        // DLC 2：回転数（バイト2〜3）は取り出せないが、アクセル・クラッチは取り出せる
        let d = decode(&frame(ID_ENGINE, &[50, 0x80]));
        assert_eq!(d.rpm, None);
        assert_eq!(d.accel_pct, Some(20));
        assert_eq!(decode(&frame(ID_TEMPS, &[0, 0, 100])).water_temp_c, None);
    }

    #[test]
    fn 対象外は何も取り出さない() {
        assert_eq!(
            decode(&frame(0x123, &[1, 2, 3, 4, 5, 6, 7, 8])),
            Decoded::default()
        );
        let mut f = frame(ID_ENGINE, &[0, 0, 0x88, 0x13, 0, 0, 0, 0]);
        f.kind = FrameKind::Remote;
        assert_eq!(decode(&f), Decoded::default());
        f.kind = FrameKind::Data;
        f.format = IdFormat::Extended;
        assert_eq!(decode(&f), Decoded::default());
        f.format = IdFormat::Standard;
        f.dlc = 9;
        assert_eq!(decode(&f), Decoded::default());
    }

    #[test]
    fn 車両情報に反映し_ギアは反映しない() {
        let mut data = VehicleData::default();
        for f in [
            frame(ID_ENGINE, &[61, 0, 0x88, 0x13, 0, 0, 0, 0]),
            frame(ID_TEMPS, &[0, 0, 136, 128, 0, 0, 0, 0]),
            frame(ID_ENGINE_ALT, &[0, 0, 0, 0, 0, 0, 3, 0]),
        ] {
            apply(&decode(&f), &mut data, 100);
        }
        let s = data.snapshot(150, &Limits::provisional());
        assert_eq!(s.rpm, Some(5000));
        assert_eq!(s.accel_pct, Some(24));
        assert_eq!(s.oil_temp_c, Some(96));
        assert_eq!(s.water_temp_c, Some(88));
        assert_eq!(s.gear, None);
        assert_eq!(s.speed_kmh, None); // 0xD1を受信していない
    }
}

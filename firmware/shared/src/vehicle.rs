//! 車両情報（回転数・車速・水温・油温・アクセル開度・ギア）と、表示用のスナップショット。
//!
//! 未受信・更新停止・範囲外の値は無効（`None`）とし、画面では「--」を表示する（要件 CAN-08）。
//! 取得済みの0と区別するため、無効値を0で表さない。

use crate::reading::Reading;

/// 受信した車両情報（項目ごとに値・有効性・最終受信時刻を持つ）。
#[derive(Debug, Clone, Copy, Default)]
pub struct VehicleData {
    /// エンジン回転数 [rpm]
    pub rpm: Reading<u16>,
    /// CAN車速 [km/h]
    pub speed_kmh: Reading<u16>,
    /// 水温 [℃]
    pub water_temp_c: Reading<i16>,
    /// 油温 [℃]
    pub oil_temp_c: Reading<i16>,
    /// アクセル開度 [%]
    pub accel_pct: Reading<u8>,
    /// ギア（1〜6）。判定不能のときは値を持たない
    pub gear: Reading<u8>,
}

/// 範囲とタイムアウトの設定。
///
/// 値は仮のもの。実車のCANデータで確認し、段階7で確定する（要件 CAN-08）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// 最終受信からこの時間を超えて更新がない項目は無効にする [ms]
    pub timeout_ms: u64,
    pub rpm_max: u16,
    pub speed_max_kmh: u16,
    pub temp_min_c: i16,
    pub temp_max_c: i16,
}

impl Limits {
    /// 仮の設定。
    pub const fn provisional() -> Self {
        Self {
            timeout_ms: 1000,
            rpm_max: 9000,
            speed_max_kmh: 300,
            temp_min_c: -40,
            temp_max_c: 200,
        }
    }
}

/// 表示用のスナップショット。無効な項目は `None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleSnapshot {
    pub rpm: Option<u16>,
    pub speed_kmh: Option<u16>,
    pub water_temp_c: Option<i16>,
    pub oil_temp_c: Option<i16>,
    pub accel_pct: Option<u8>,
    pub gear: Option<u8>,
}

impl VehicleData {
    /// 現在時刻で有効な値だけを取り出す。範囲外の値は無効にする（異常値を表示しない）。
    pub fn snapshot(&self, now_ms: u64, limits: &Limits) -> VehicleSnapshot {
        let t = limits.timeout_ms;
        VehicleSnapshot {
            rpm: self.rpm.get(now_ms, t).filter(|&v| v <= limits.rpm_max),
            speed_kmh: self
                .speed_kmh
                .get(now_ms, t)
                .filter(|&v| v <= limits.speed_max_kmh),
            water_temp_c: self
                .water_temp_c
                .get(now_ms, t)
                .filter(|&v| (limits.temp_min_c..=limits.temp_max_c).contains(&v)),
            oil_temp_c: self
                .oil_temp_c
                .get(now_ms, t)
                .filter(|&v| (limits.temp_min_c..=limits.temp_max_c).contains(&v)),
            accel_pct: self.accel_pct.get(now_ms, t).filter(|&v| v <= 100),
            gear: self.gear.get(now_ms, t).filter(|&v| (1..=6).contains(&v)),
        }
    }
}

/// 値を文字列にする。無効（`None`）は「--」。取得済みの0は「0」になる。
pub fn value_or_dash<T: core::fmt::Display>(value: Option<T>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "--".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits::provisional()
    }

    #[test]
    fn 未受信は全項目が無効() {
        let s = VehicleData::default().snapshot(0, &limits());
        assert_eq!(s.rpm, None);
        assert_eq!(s.speed_kmh, None);
        assert_eq!(s.water_temp_c, None);
        assert_eq!(s.oil_temp_c, None);
        assert_eq!(s.accel_pct, None);
        assert_eq!(s.gear, None);
    }

    #[test]
    fn 受信した値はそのまま取り出せる() {
        let mut d = VehicleData::default();
        d.rpm.set(3200, 100);
        d.speed_kmh.set(60, 100);
        d.water_temp_c.set(88, 100);
        d.oil_temp_c.set(96, 100);
        d.accel_pct.set(24, 100);
        d.gear.set(3, 100);
        let s = d.snapshot(200, &limits());
        assert_eq!(s.rpm, Some(3200));
        assert_eq!(s.speed_kmh, Some(60));
        assert_eq!(s.water_temp_c, Some(88));
        assert_eq!(s.oil_temp_c, Some(96));
        assert_eq!(s.accel_pct, Some(24));
        assert_eq!(s.gear, Some(3));
    }

    #[test]
    fn 更新が止まった項目だけが無効になる() {
        let mut d = VehicleData::default();
        d.rpm.set(3200, 0);
        d.speed_kmh.set(60, 1500);
        let s = d.snapshot(2000, &limits()); // タイムアウト1000ms
        assert_eq!(s.rpm, None);
        assert_eq!(s.speed_kmh, Some(60));
    }

    #[test]
    fn 範囲外の値は無効にする() {
        let mut d = VehicleData::default();
        d.rpm.set(65535, 0);
        d.speed_kmh.set(999, 0);
        d.water_temp_c.set(-100, 0);
        d.oil_temp_c.set(500, 0);
        d.accel_pct.set(200, 0);
        d.gear.set(9, 0);
        let s = d.snapshot(0, &limits());
        assert_eq!(s.rpm, None);
        assert_eq!(s.speed_kmh, None);
        assert_eq!(s.water_temp_c, None);
        assert_eq!(s.oil_temp_c, None);
        assert_eq!(s.accel_pct, None);
        assert_eq!(s.gear, None);
    }

    #[test]
    fn 範囲の境界値は有効() {
        let l = limits();
        let mut d = VehicleData::default();
        d.rpm.set(l.rpm_max, 0);
        d.water_temp_c.set(l.temp_min_c, 0);
        d.oil_temp_c.set(l.temp_max_c, 0);
        d.accel_pct.set(100, 0);
        d.gear.set(1, 0);
        let s = d.snapshot(0, &l);
        assert_eq!(s.rpm, Some(l.rpm_max));
        assert_eq!(s.water_temp_c, Some(l.temp_min_c));
        assert_eq!(s.oil_temp_c, Some(l.temp_max_c));
        assert_eq!(s.accel_pct, Some(100));
        assert_eq!(s.gear, Some(1));
    }

    #[test]
    fn 取得済みの0は有効で表示は0_無効は二本線() {
        let mut d = VehicleData::default();
        d.speed_kmh.set(0, 0);
        let s = d.snapshot(0, &limits());
        assert_eq!(value_or_dash(s.speed_kmh), "0");
        assert_eq!(value_or_dash(s.rpm), "--");
    }
}

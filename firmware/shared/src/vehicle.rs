//! 車両情報（回転数・車速・水温・油温・アクセル開度・ギア）と、表示用のスナップショット。
//!
//! 未受信・更新停止・範囲外の値は無効（`None`）とし、画面では「--」を表示する（要件 CAN-08）。
//! 取得済みの0と区別するため、無効値を0で表さない。

use crate::reading::Reading;

/// ギアの状態。
///
/// 判定不能・無効は、値を持たないこと（`Option::None`、画面では「--」）で表す。
/// Rは、受信だけで確定できる信号を実車で確認してから追加する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gear {
    /// ニュートラル（N）
    Neutral,
    /// 前進（1〜6速）
    Forward(u8),
}

impl core::fmt::Display for Gear {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Gear::Neutral => write!(f, "N"),
            Gear::Forward(n) => write!(f, "{n}"),
        }
    }
}

impl Gear {
    /// 画面に出せるギアか（前進は1〜6速）。
    pub const fn is_valid(self) -> bool {
        match self {
            Gear::Neutral => true,
            Gear::Forward(n) => n >= 1 && n <= 6,
        }
    }
}

/// GPSの測位結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsFix {
    /// 緯度 [1e-6度]（北緯が正）
    pub lat_e6: i32,
    /// 経度 [1e-6度]（東経が正）
    pub lon_e6: i32,
    /// GPS速度 [km/h]
    pub speed_kmh: u16,
    /// 日本時間の0時からの秒数（0〜86399）
    pub time_s: u32,
}

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
    /// ギア。判定不能のときは値を持たない
    pub gear: Reading<Gear>,
    /// GPSの測位結果
    pub gps: Reading<GpsFix>,
}

/// 範囲とタイムアウトの設定。
///
/// 値は仮のもの。実車のCANデータで確認し、段階7で確定する（要件 CAN-08）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// 最終受信からこの時間を超えて更新がない項目は無効にする [ms]
    pub timeout_ms: u64,
    /// GPSは更新が遅いため、別のタイムアウトを持つ [ms]
    pub gps_timeout_ms: u64,
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
            gps_timeout_ms: 3000,
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
    pub gear: Option<Gear>,
    pub gps: Option<GpsFix>,
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
            gear: self.gear.get(now_ms, t).filter(|g| g.is_valid()),
            gps: self
                .gps
                .get(now_ms, limits.gps_timeout_ms)
                .filter(|g| gps_in_range(g, limits)),
        }
    }
}

fn gps_in_range(g: &GpsFix, limits: &Limits) -> bool {
    (-90_000_000..=90_000_000).contains(&g.lat_e6)
        && (-180_000_000..=180_000_000).contains(&g.lon_e6)
        && g.speed_kmh <= limits.speed_max_kmh
        && g.time_s < 86_400
}

/// 緯度・経度（1e-6度）を「35.681236°」の形にする。
pub fn format_degrees(e6: i32) -> String {
    let sign = if e6 < 0 { "-" } else { "" };
    let abs = e6.unsigned_abs();
    format!("{sign}{}.{:06}°", abs / 1_000_000, abs % 1_000_000)
}

/// 日本時間の0時からの秒数を「12:34:56」の形にする。
pub fn format_time(time_s: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        time_s / 3600 % 24,
        time_s / 60 % 60,
        time_s % 60
    )
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
        d.gear.set(Gear::Forward(3), 100);
        let s = d.snapshot(200, &limits());
        assert_eq!(s.rpm, Some(3200));
        assert_eq!(s.speed_kmh, Some(60));
        assert_eq!(s.water_temp_c, Some(88));
        assert_eq!(s.oil_temp_c, Some(96));
        assert_eq!(s.accel_pct, Some(24));
        assert_eq!(s.gear, Some(Gear::Forward(3)));
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
        d.gear.set(Gear::Forward(9), 0);
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
        d.gear.set(Gear::Forward(1), 0);
        let s = d.snapshot(0, &l);
        assert_eq!(s.rpm, Some(l.rpm_max));
        assert_eq!(s.water_temp_c, Some(l.temp_min_c));
        assert_eq!(s.oil_temp_c, Some(l.temp_max_c));
        assert_eq!(s.accel_pct, Some(100));
        assert_eq!(s.gear, Some(Gear::Forward(1)));
    }

    fn fix() -> GpsFix {
        GpsFix {
            lat_e6: 35_681_236,
            lon_e6: 139_767_125,
            speed_kmh: 59,
            time_s: 12 * 3600 + 34 * 60 + 56,
        }
    }

    #[test]
    fn ニュートラルは有効で_nと表示される() {
        let mut d = VehicleData::default();
        d.gear.set(Gear::Neutral, 0);
        let s = d.snapshot(0, &limits());
        assert_eq!(s.gear, Some(Gear::Neutral));
        assert_eq!(value_or_dash(s.gear), "N");
    }

    #[test]
    fn ギアの表示() {
        assert_eq!(Gear::Neutral.to_string(), "N");
        assert_eq!(Gear::Forward(1).to_string(), "1");
        assert_eq!(Gear::Forward(6).to_string(), "6");
    }

    #[test]
    fn 前進ギアの範囲外は無効() {
        let mut d = VehicleData::default();
        for bad in [0u8, 7, 255] {
            d.gear.set(Gear::Forward(bad), 0);
            assert_eq!(d.snapshot(0, &limits()).gear, None, "{bad}");
        }
    }

    #[test]
    fn gpsは受信した値を取り出せる() {
        let mut d = VehicleData::default();
        d.gps.set(fix(), 100);
        assert_eq!(d.snapshot(200, &limits()).gps, Some(fix()));
    }

    #[test]
    fn gpsは別のタイムアウトで無効になる() {
        let mut d = VehicleData::default();
        d.gps.set(fix(), 0);
        d.rpm.set(3000, 0);
        let l = limits(); // 通常1000ms、GPS 3000ms
        let s = d.snapshot(2000, &l);
        assert_eq!(s.rpm, None);
        assert_eq!(s.gps, Some(fix()));
        assert_eq!(d.snapshot(3001, &l).gps, None);
    }

    #[test]
    fn gpsの範囲外は無効() {
        let mut d = VehicleData::default();
        for bad in [
            GpsFix {
                lat_e6: 91_000_000,
                ..fix()
            },
            GpsFix {
                lat_e6: -91_000_000,
                ..fix()
            },
            GpsFix {
                lon_e6: 181_000_000,
                ..fix()
            },
            GpsFix {
                speed_kmh: 999,
                ..fix()
            },
            GpsFix {
                time_s: 86_400,
                ..fix()
            },
        ] {
            d.gps.set(bad, 0);
            assert_eq!(d.snapshot(0, &limits()).gps, None, "{bad:?}");
        }
    }

    #[test]
    fn gpsの範囲の境界値は有効() {
        let mut d = VehicleData::default();
        let edge = GpsFix {
            lat_e6: -90_000_000,
            lon_e6: 180_000_000,
            speed_kmh: 0,
            time_s: 86_399,
        };
        d.gps.set(edge, 0);
        assert_eq!(d.snapshot(0, &limits()).gps, Some(edge));
    }

    #[test]
    fn 緯度経度の表示() {
        assert_eq!(format_degrees(35_681_236), "35.681236°");
        assert_eq!(format_degrees(139_767_125), "139.767125°");
        assert_eq!(format_degrees(-33_868_820), "-33.868820°");
        assert_eq!(format_degrees(5), "0.000005°");
        assert_eq!(format_degrees(-5), "-0.000005°");
        assert_eq!(format_degrees(0), "0.000000°");
    }

    #[test]
    fn 時刻の表示() {
        assert_eq!(format_time(0), "00:00:00");
        assert_eq!(format_time(12 * 3600 + 34 * 60 + 56), "12:34:56");
        assert_eq!(format_time(86_399), "23:59:59");
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

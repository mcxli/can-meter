//! 仮データ生成（段階3：実車接続前に、表示・無効値・更新停止を確認するため）。
//!
//! 時刻だけから値を決める（乱数を使わない）ため、同じ時刻なら同じ値になり、テストできる。
//! 数値は表示確認用のもので、実車の値ではない。

use crate::vehicle::VehicleData;

/// 仮データの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoMode {
    /// 一定値
    Constant,
    /// 連続変化
    Sweep,
    /// 未取得（値を一度も受信しない）
    Missing,
    /// 異常値（範囲外の値を受信し続ける）
    Anomaly,
    /// 更新停止（最初だけ受信し、その後は更新しない）
    Stale,
}

/// 仮データの生成器。
#[derive(Debug, Clone, Copy)]
pub struct DemoGenerator {
    mode: DemoMode,
}

/// `Stale` で、最初に値を受信する期間 [ms]。
pub const STALE_ACTIVE_MS: u64 = 2000;

impl DemoGenerator {
    pub const fn new(mode: DemoMode) -> Self {
        Self { mode }
    }

    pub const fn mode(&self) -> DemoMode {
        self.mode
    }

    /// 時刻 `now_ms` の仮データを `data` に反映する（受信したことにする）。
    pub fn tick(&self, now_ms: u64, data: &mut VehicleData) {
        match self.mode {
            DemoMode::Constant => set_all(data, 3200, 60, 88, 96, 24, 3, now_ms),
            DemoMode::Sweep => {
                let rpm = triangle(now_ms, 20_000, 800, 7500) as u16;
                let speed = triangle(now_ms, 30_000, 0, 160) as u16;
                let water = triangle(now_ms, 60_000, 70, 105) as i16;
                let oil = triangle(now_ms, 60_000, 80, 120) as i16;
                let accel = triangle(now_ms, 10_000, 0, 100) as u8;
                let gear = 1 + ((now_ms / 4000) % 6) as u8;
                set_all(data, rpm, speed, water, oil, accel, gear, now_ms);
            }
            DemoMode::Missing => {}
            DemoMode::Anomaly => set_all(data, u16::MAX, 999, -100, 500, 200, 9, now_ms),
            DemoMode::Stale => {
                if now_ms < STALE_ACTIVE_MS {
                    set_all(data, 3200, 60, 88, 96, 24, 3, now_ms);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn set_all(
    d: &mut VehicleData,
    rpm: u16,
    speed: u16,
    water: i16,
    oil: i16,
    accel: u8,
    gear: u8,
    now_ms: u64,
) {
    d.rpm.set(rpm, now_ms);
    d.speed_kmh.set(speed, now_ms);
    d.water_temp_c.set(water, now_ms);
    d.oil_temp_c.set(oil, now_ms);
    d.accel_pct.set(accel, now_ms);
    d.gear.set(gear, now_ms);
}

/// 周期 `period_ms` で `min`〜`max` を往復する三角波（`now_ms=0` で `min`、半周期で `max`）。
fn triangle(now_ms: u64, period_ms: u64, min: i32, max: i32) -> i32 {
    let phase = now_ms % period_ms;
    let half = period_ms / 2;
    let up = if phase < half {
        phase
    } else {
        period_ms - phase
    };
    min + ((max - min) as i64 * up as i64 / half as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle::Limits;

    fn run(mode: DemoMode, now_ms: u64) -> crate::vehicle::VehicleSnapshot {
        let mut d = VehicleData::default();
        let g = DemoGenerator::new(mode);
        // 100ms周期で時刻 now_ms まで受信を繰り返す
        let mut t = 0;
        while t <= now_ms {
            g.tick(t, &mut d);
            t += 100;
        }
        d.snapshot(now_ms, &Limits::provisional())
    }

    #[test]
    fn 一定値はサンプル画像の値になる() {
        let s = run(DemoMode::Constant, 500);
        assert_eq!(s.rpm, Some(3200));
        assert_eq!(s.speed_kmh, Some(60));
        assert_eq!(s.water_temp_c, Some(88));
        assert_eq!(s.oil_temp_c, Some(96));
        assert_eq!(s.accel_pct, Some(24));
        assert_eq!(s.gear, Some(3));
    }

    #[test]
    fn 連続変化は常に範囲内で有効() {
        let l = Limits::provisional();
        let mut d = VehicleData::default();
        let g = DemoGenerator::new(DemoMode::Sweep);
        for t in (0..120_000).step_by(100) {
            g.tick(t, &mut d);
            let s = d.snapshot(t, &l);
            assert!(s.rpm.is_some(), "rpm無効 t={t}");
            assert!(s.speed_kmh.is_some(), "speed無効 t={t}");
            assert!(s.water_temp_c.is_some(), "water無効 t={t}");
            assert!(s.oil_temp_c.is_some(), "oil無効 t={t}");
            assert!(s.accel_pct.is_some(), "accel無効 t={t}");
            assert!(s.gear.is_some(), "gear無効 t={t}");
        }
    }

    #[test]
    fn 連続変化は実際に値が動く() {
        let a = run(DemoMode::Sweep, 0);
        let b = run(DemoMode::Sweep, 10_000); // 回転数の半周期（頂点）
        assert_eq!(a.rpm, Some(800));
        assert_eq!(b.rpm, Some(7500));
    }

    #[test]
    fn 連続変化のギアは1から6を巡回する() {
        assert_eq!(run(DemoMode::Sweep, 0).gear, Some(1));
        assert_eq!(run(DemoMode::Sweep, 4000).gear, Some(2));
        assert_eq!(run(DemoMode::Sweep, 20_000).gear, Some(6));
        assert_eq!(run(DemoMode::Sweep, 24_000).gear, Some(1));
    }

    #[test]
    fn 未取得は全項目が無効() {
        let s = run(DemoMode::Missing, 5000);
        assert_eq!(s.rpm, None);
        assert_eq!(s.speed_kmh, None);
        assert_eq!(s.water_temp_c, None);
        assert_eq!(s.oil_temp_c, None);
        assert_eq!(s.accel_pct, None);
        assert_eq!(s.gear, None);
    }

    #[test]
    fn 異常値は範囲外として無効になる() {
        let s = run(DemoMode::Anomaly, 500);
        assert_eq!(s.rpm, None);
        assert_eq!(s.speed_kmh, None);
        assert_eq!(s.water_temp_c, None);
        assert_eq!(s.oil_temp_c, None);
        assert_eq!(s.accel_pct, None);
        assert_eq!(s.gear, None);
    }

    #[test]
    fn 更新停止は最初は有効でタイムアウト後に無効になる() {
        let early = run(DemoMode::Stale, 1500);
        assert_eq!(early.rpm, Some(3200));
        // 最後の受信は1900ms。タイムアウト1000msを超える3000msで無効
        let late = run(DemoMode::Stale, 3000);
        assert_eq!(late.rpm, None);
        assert_eq!(late.gear, None);
    }

    #[test]
    fn 同じ時刻なら同じ値になる() {
        assert_eq!(run(DemoMode::Sweep, 12_300), run(DemoMode::Sweep, 12_300));
    }

    #[test]
    fn 三角波の端点() {
        assert_eq!(triangle(0, 1000, 10, 110), 10);
        assert_eq!(triangle(500, 1000, 10, 110), 110);
        assert_eq!(triangle(1000, 1000, 10, 110), 10);
        assert_eq!(triangle(250, 1000, 0, 100), 50);
    }
}

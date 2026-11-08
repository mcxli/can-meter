//! 回転数通知（ギア数字の色変更・点滅）の判定。
//!
//! 仕様は `docs/開発手順/03_メーター画面.md` の「回転数通知」による。
//! 発動・解除の回転数と点滅周期は設定値として持つ。実車の限界回転数やエンジン保護を保証するものではない。

/// 設定値。解除値は発動値より低くする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShiftNoticeConfig {
    activate_rpm: u16,
    release_rpm: u16,
    blink_period_ms: u32,
}

/// 設定値が不正な場合の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// 解除値が発動値以上（ヒステリシスが成り立たない）
    ReleaseNotBelowActivate,
    /// 点滅周期が0、または明暗の半周期が0になる
    InvalidBlinkPeriod,
}

impl ShiftNoticeConfig {
    /// 不正な設定のまま通知を有効にしないため、検証して作る。
    pub fn new(
        activate_rpm: u16,
        release_rpm: u16,
        blink_period_ms: u32,
    ) -> Result<Self, ConfigError> {
        if release_rpm >= activate_rpm {
            return Err(ConfigError::ReleaseNotBelowActivate);
        }
        if blink_period_ms < 2 {
            return Err(ConfigError::InvalidBlinkPeriod);
        }
        Ok(Self {
            activate_rpm,
            release_rpm,
            blink_period_ms,
        })
    }
}

/// ギア表示の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearDisplay {
    /// 白で点灯
    Normal,
    /// 通知中の明るい赤
    NoticeBright,
    /// 通知中の暗い赤（数字は消さない）
    NoticeDim,
    /// ギア判定不能・無効。「--」をグレーで表示し、点滅しない
    Invalid,
}

/// 通知の判定器。発動したかどうかの状態だけを持つ。点滅の位相は時刻から求めるため、
/// 画面を切り替えても（判定器を共有する限り）リセットされない。
#[derive(Debug, Clone, Copy)]
pub struct ShiftNotice {
    config: ShiftNoticeConfig,
    active: bool,
}

impl ShiftNotice {
    pub const fn new(config: ShiftNoticeConfig) -> Self {
        Self {
            config,
            active: false,
        }
    }

    /// 現在の表示状態を求める。
    ///
    /// - `rpm`：有効な回転数。無効・更新停止なら `None`（通知を解除し、過去の値で通知し続けない）。
    /// - `gear_valid`：ギアが有効か。無効なら「--」を優先し、点滅しない。
    /// - `now_ms`：単調増加する時刻。
    pub fn update(&mut self, rpm: Option<u16>, gear_valid: bool, now_ms: u64) -> GearDisplay {
        match rpm {
            None => self.active = false, // 回転数が無効なら通知を解除し、再到着時に判定し直す
            Some(rpm) => {
                if rpm >= self.config.activate_rpm {
                    self.active = true;
                } else if rpm <= self.config.release_rpm {
                    self.active = false;
                }
                // 解除値より大きく発動値未満では、直前の状態を維持する
            }
        }

        if !gear_valid {
            return GearDisplay::Invalid;
        }
        if !self.active {
            return GearDisplay::Normal;
        }
        let half = u64::from(self.config.blink_period_ms / 2);
        if (now_ms / half) % 2 == 0 {
            GearDisplay::NoticeBright
        } else {
            GearDisplay::NoticeDim
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 試験用の値（実車の設定値ではない）
    fn notice() -> ShiftNotice {
        ShiftNotice::new(ShiftNoticeConfig::new(6500, 6000, 400).unwrap())
    }

    #[test]
    fn 解除値が発動値以上の設定は拒否する() {
        assert_eq!(
            ShiftNoticeConfig::new(6000, 6000, 400),
            Err(ConfigError::ReleaseNotBelowActivate)
        );
        assert_eq!(
            ShiftNoticeConfig::new(6000, 6500, 400),
            Err(ConfigError::ReleaseNotBelowActivate)
        );
    }

    #[test]
    fn 点滅周期が小さすぎる設定は拒否する() {
        assert_eq!(
            ShiftNoticeConfig::new(6500, 6000, 0),
            Err(ConfigError::InvalidBlinkPeriod)
        );
        assert_eq!(
            ShiftNoticeConfig::new(6500, 6000, 1),
            Err(ConfigError::InvalidBlinkPeriod)
        );
    }

    #[test]
    fn 通常は白で点灯する() {
        let mut n = notice();
        assert_eq!(n.update(Some(3200), true, 0), GearDisplay::Normal);
    }

    #[test]
    fn 発動値以上で明暗を交互に表示する() {
        let mut n = notice();
        assert_eq!(n.update(Some(6500), true, 0), GearDisplay::NoticeBright);
        assert_eq!(n.update(Some(6500), true, 199), GearDisplay::NoticeBright);
        assert_eq!(n.update(Some(6500), true, 200), GearDisplay::NoticeDim);
        assert_eq!(n.update(Some(6500), true, 399), GearDisplay::NoticeDim);
        assert_eq!(n.update(Some(6500), true, 400), GearDisplay::NoticeBright);
    }

    #[test]
    fn 発動値未満では発動しない() {
        let mut n = notice();
        assert_eq!(n.update(Some(6499), true, 0), GearDisplay::Normal);
    }

    #[test]
    fn 解除値より大きく発動値未満では直前の状態を維持する() {
        let mut n = notice();
        n.update(Some(6600), true, 0);
        assert_ne!(n.update(Some(6200), true, 0), GearDisplay::Normal); // 通知中は維持
        n.update(Some(5900), true, 0); // 解除
        assert_eq!(n.update(Some(6200), true, 0), GearDisplay::Normal); // 解除後は発動しない
    }

    #[test]
    fn 解除値以下で白に戻る() {
        let mut n = notice();
        n.update(Some(6600), true, 0);
        assert_eq!(n.update(Some(6000), true, 0), GearDisplay::Normal);
    }

    #[test]
    fn ギア無効は通知中でもグレーの二本線で点滅しない() {
        let mut n = notice();
        n.update(Some(6600), true, 0);
        assert_eq!(n.update(Some(6600), false, 0), GearDisplay::Invalid);
        assert_eq!(n.update(Some(6600), false, 200), GearDisplay::Invalid);
    }

    #[test]
    fn 回転数が無効になると通知を解除する() {
        let mut n = notice();
        n.update(Some(6600), true, 0);
        assert_eq!(n.update(None, true, 0), GearDisplay::Normal);
    }

    #[test]
    fn 回転数が再到着したら発動値以上かを判定し直す() {
        let mut n = notice();
        n.update(Some(6600), true, 0);
        n.update(None, true, 0);
        // 6200は解除値より大きく発動値未満。過去の通知状態を引き継がず、通常に戻ったまま
        assert_eq!(n.update(Some(6200), true, 0), GearDisplay::Normal);
        assert_ne!(n.update(Some(6600), true, 0), GearDisplay::Normal);
    }

    #[test]
    fn 点滅の位相は時刻で決まり画面切り替えでリセットされない() {
        let mut n = notice();
        // 同じ判定器・同じ時刻なら、何度呼んでも同じ位相
        let a = n.update(Some(6600), true, 250);
        let b = n.update(Some(6600), true, 250);
        assert_eq!(a, b);
        assert_eq!(a, GearDisplay::NoticeDim);
    }

    #[test]
    fn 回転数無効かつギア無効はギア無効を優先する() {
        let mut n = notice();
        assert_eq!(n.update(None, false, 0), GearDisplay::Invalid);
    }
}

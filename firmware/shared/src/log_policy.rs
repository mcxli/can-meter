//! ログの保存先・ファイル分割・同期の方針。仕様は `docs/実装仕様/ログ形式.md` による。

/// ファイルを分割する間隔 [µs]（10分。要件 LOG-09）。
pub const ROTATE_INTERVAL_US: u64 = 10 * 60 * 1_000_000;
/// ファイルシステムへ同期する間隔 [µs]（1秒。要件 LOG-13）。
pub const SYNC_INTERVAL_US: u64 = 1_000_000;

/// ログの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    Can,
    Gps,
    Events,
}

impl LogKind {
    const fn prefix(self) -> &'static str {
        match self {
            LogKind::Can => "can",
            LogKind::Gps => "gps",
            LogKind::Events => "events",
        }
    }
}

/// ファイル名。`index` は0始まりで、名前は1始まりの3桁（`can_001.csv`）。
pub fn file_name(kind: LogKind, index: u32) -> String {
    format!("{}_{:03}.csv", kind.prefix(), index + 1)
}

/// 起動ごとの保存先の名前。`LOG_0001`、仮データのときは `LOG_0001_DEMO`。
pub fn session_dir_name(number: u32, demo: bool) -> String {
    if demo {
        format!("LOG_{number:04}_DEMO")
    } else {
        format!("LOG_{number:04}")
    }
}

/// 保存先の名前から、番号を取り出す（`LOG_0003`・`LOG_0003_DEMO`）。それ以外の名前は `None`。
fn session_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("LOG_")?;
    let (digits, suffix) = rest.split_at(rest.len().min(4));
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !(suffix.is_empty() || suffix == "_DEMO") {
        return None;
    }
    digits.parse().ok()
}

/// 既存の保存先の名前から、次の保存先の名前を決める。既存の最大番号+1で、既存ログと衝突しない。
/// 番号が使い切られている（9999の次）場合は `None`。GPSの測位や時刻に依存しない。
pub fn next_session_dir(existing: &[&str], demo: bool) -> Option<String> {
    let max = existing
        .iter()
        .filter_map(|n| session_number(n))
        .max()
        .unwrap_or(0);
    let next = max.checked_add(1)?;
    (next <= 9999).then(|| session_dir_name(next, demo))
}

/// 記録開始からの経過で、ファイルの番号（0始まり）を決める。分割の境界の時刻は、新しいファイルに入る。
/// 記録開始より前の時刻は `None`。
pub fn file_index(start_us: u64, t_us: u64) -> Option<u32> {
    let elapsed = t_us.checked_sub(start_us)?;
    u32::try_from(elapsed / ROTATE_INTERVAL_US).ok()
}

/// ファイル分割の状態。時刻が前後して届いても、一度進んだファイルへは戻らない（二重記録・取り違えを防ぐ）。
#[derive(Debug, Clone, Copy)]
pub struct Rotator {
    start_us: u64,
    current: u32,
}

/// 1件を書くファイルと、ファイルを切り替えたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// 書き込み先のファイルの番号（0始まり）
    pub index: u32,
    /// このとき、新しいファイルへ切り替えた（前のファイルを閉じて、新しいファイルを開く）
    pub rotated: bool,
}

impl Rotator {
    pub const fn new(start_us: u64) -> Self {
        Self {
            start_us,
            current: 0,
        }
    }

    /// 時刻 `t_us` のデータを、どのファイルに書くかを決める。
    pub fn place(&mut self, t_us: u64) -> Placement {
        let wanted = file_index(self.start_us, t_us).unwrap_or(self.current);
        if wanted > self.current {
            self.current = wanted;
            Placement {
                index: wanted,
                rotated: true,
            }
        } else {
            Placement {
                index: self.current,
                rotated: false,
            }
        }
    }
}

/// 同期（`fsync` 相当）のタイミング。
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncPolicy {
    last_us: Option<u64>,
}

impl SyncPolicy {
    pub const fn new() -> Self {
        Self { last_us: None }
    }

    /// いま同期するべきか。するべき場合は、同期した時刻として記録する。
    /// 最初の呼び出しは、基準を記録するだけで `false`（記録開始の直後に同期しない）。
    pub fn due(&mut self, now_us: u64) -> bool {
        match self.last_us {
            None => {
                self.last_us = Some(now_us);
                false
            }
            Some(last) if now_us.saturating_sub(last) >= SYNC_INTERVAL_US => {
                self.last_us = Some(now_us);
                true
            }
            Some(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ファイル名() {
        assert_eq!(file_name(LogKind::Can, 0), "can_001.csv");
        assert_eq!(file_name(LogKind::Gps, 11), "gps_012.csv");
        assert_eq!(file_name(LogKind::Events, 998), "events_999.csv");
    }

    #[test]
    fn 保存先の名前() {
        assert_eq!(session_dir_name(1, false), "LOG_0001");
        assert_eq!(session_dir_name(42, true), "LOG_0042_DEMO");
    }

    #[test]
    fn 既存がなければ1番() {
        assert_eq!(next_session_dir(&[], false), Some("LOG_0001".into()));
    }

    #[test]
    fn 既存の最大番号の次になる() {
        let existing = ["LOG_0001", "LOG_0007", "LOG_0003_DEMO"];
        assert_eq!(next_session_dir(&existing, false), Some("LOG_0008".into()));
        assert_eq!(
            next_session_dir(&existing, true),
            Some("LOG_0008_DEMO".into())
        );
    }

    #[test]
    fn 仮データと実データで番号が重ならない() {
        let existing = ["LOG_0005_DEMO"];
        // 仮データの番号も数えるので、実データは6番になる
        assert_eq!(next_session_dir(&existing, false), Some("LOG_0006".into()));
    }

    #[test]
    fn 関係のない名前は無視する() {
        let existing = [
            "LOG_12",
            "log_0003",
            "LOG_0004X",
            "LOG_0004_demo",
            "LOG_00A4",
            "README.txt",
            "System Volume Information",
            "LOG_0002",
        ];
        assert_eq!(next_session_dir(&existing, false), Some("LOG_0003".into()));
    }

    #[test]
    fn 番号を使い切ったら作らない() {
        assert_eq!(next_session_dir(&["LOG_9999"], false), None);
        assert_eq!(
            next_session_dir(&["LOG_9998"], false),
            Some("LOG_9999".into())
        );
    }

    #[test]
    fn ファイルの番号は10分ごとに進む() {
        let start = 5_000_000;
        assert_eq!(file_index(start, start), Some(0));
        assert_eq!(file_index(start, start + ROTATE_INTERVAL_US - 1), Some(0));
        assert_eq!(file_index(start, start + ROTATE_INTERVAL_US), Some(1));
        assert_eq!(file_index(start, start + 25 * 60 * 1_000_000), Some(2));
    }

    #[test]
    fn 記録開始より前の時刻は対象外() {
        assert_eq!(file_index(1000, 999), None);
    }

    #[test]
    fn 境界の1件は新しいファイルだけに入り_二重にならない() {
        let mut r = Rotator::new(0);
        let before = r.place(ROTATE_INTERVAL_US - 1);
        let at = r.place(ROTATE_INTERVAL_US);
        let after = r.place(ROTATE_INTERVAL_US + 1);
        assert_eq!(
            before,
            Placement {
                index: 0,
                rotated: false
            }
        );
        assert_eq!(
            at,
            Placement {
                index: 1,
                rotated: true
            }
        );
        assert_eq!(
            after,
            Placement {
                index: 1,
                rotated: false
            }
        );
    }

    #[test]
    fn 時刻が前後して届いても戻らない() {
        let mut r = Rotator::new(0);
        r.place(ROTATE_INTERVAL_US + 10); // 1番へ進む
        let late = r.place(ROTATE_INTERVAL_US - 5); // 古い時刻のデータが遅れて届いた
        assert_eq!(
            late,
            Placement {
                index: 1,
                rotated: false
            }
        );
    }

    #[test]
    fn 一度に複数のファイルを飛ばしても切り替えは1回() {
        let mut r = Rotator::new(0);
        let p = r.place(3 * ROTATE_INTERVAL_US + 1);
        assert_eq!(
            p,
            Placement {
                index: 3,
                rotated: true
            }
        );
        assert!(!r.place(3 * ROTATE_INTERVAL_US + 2).rotated);
    }

    #[test]
    fn 同期は1秒ごと() {
        let mut s = SyncPolicy::new();
        assert!(!s.due(0)); // 最初は基準を記録するだけ
        assert!(!s.due(999_999));
        assert!(s.due(1_000_000));
        assert!(!s.due(1_500_000));
        assert!(s.due(2_000_000));
    }

    #[test]
    fn 同期は間隔が空いても1回だけ() {
        let mut s = SyncPolicy::new();
        s.due(0);
        assert!(s.due(10_000_000));
        assert!(!s.due(10_000_001));
    }

    #[test]
    fn 時刻が巻き戻っても同期の判定は壊れない() {
        let mut s = SyncPolicy::new();
        s.due(5_000_000);
        assert!(!s.due(1_000_000));
    }
}

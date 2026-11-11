//! ログ（CSV）の行の書式。仕様は `docs/実装仕様/ログ形式.md` による。
//!
//! 文字コードはUTF-8、改行は `\n`、区切りは `,`。1ファイルの先頭にヘッダー行を置く。
//! 欠損値は空欄で表し、`0` や `-1` で代用しない。時刻 `t_us` は、メイン機の単調増加時刻（起動からの経過時間、マイクロ秒）。

use crate::can::{CanFrame, FrameKind, IdFormat};

/// CANログのヘッダー行。
pub const CAN_HEADER: &str = "t_us,id,format,kind,dlc,data\n";
/// GPSログのヘッダー行。
pub const GPS_HEADER: &str =
    "t_us,utc,lat_deg,lon_deg,speed_kmh,fix_valid,time_valid,seq,session\n";
/// 補助ログ（イベント）のヘッダー行。
pub const EVENTS_HEADER: &str = "t_us,event,utc,dropped,detail\n";

/// UTCの日時。GPSが供給する測位時刻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utc {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

impl Utc {
    /// 値が日時として成り立っているか（月・日・時・分・秒・ミリ秒の範囲）。日ごとの日数は検証しない。
    pub fn is_plausible(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second <= 60 // うるう秒
            && self.millis < 1000
    }

    /// `2026-11-09T12:34:56.000Z` の形にする。
    pub fn to_iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millis
        )
    }
}

/// CANフレーム1件の行。IDは大文字の16進（標準は3桁、拡張は8桁）、データは大文字の16進を区切りなしで並べる。
///
/// 範囲外のID・DLCでも、受信したフレームは省略せず記録する（全受信フレームを記録する：要件 LOG-04）。
pub fn can_row(t_us: u64, frame: &CanFrame) -> String {
    let (id, format) = match frame.format {
        IdFormat::Standard => (format!("{:03X}", frame.id), "STD"),
        IdFormat::Extended => (format!("{:08X}", frame.id), "EXT"),
    };
    let kind = match frame.kind {
        FrameKind::Data => "DATA",
        FrameKind::Remote => "REMOTE",
        FrameKind::Error => "ERROR",
    };
    let mut data = String::with_capacity(16);
    for b in frame.payload() {
        data.push_str(&format!("{b:02X}"));
    }
    format!("{t_us},{id},{format},{kind},{},{data}\n", frame.dlc)
}

/// GPSの記録1件。無効・未取得の項目は `None`（空欄になる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsRecord {
    /// メイン機がGPSデータを受信した時刻 [µs]
    pub recv_us: u64,
    /// 測位UTC。時刻が確定していなければ `None`
    pub utc: Option<Utc>,
    /// 緯度 [1e-6度]
    pub lat_e6: Option<i32>,
    /// 経度 [1e-6度]
    pub lon_e6: Option<i32>,
    /// 速度 [0.1km/h]
    pub speed_kmh_x10: Option<u32>,
    /// 測位が有効か
    pub fix_valid: bool,
    /// 時刻が有効か
    pub time_valid: bool,
    /// サブ機が付ける連番（欠落の検出に使う）
    pub seq: u32,
    /// サブ機の起動ごとのセッション番号（サブ機の再起動を区別する）
    pub session: u32,
}

/// 緯度・経度（1e-6度）を、符号つきの小数（6桁）にする。例：`35.681236`、`-0.000005`。
pub fn format_e6(e6: i32) -> String {
    let sign = if e6 < 0 { "-" } else { "" };
    let abs = e6.unsigned_abs();
    format!("{sign}{}.{:06}", abs / 1_000_000, abs % 1_000_000)
}

fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> String) -> String {
    v.map(f).unwrap_or_default()
}

/// GPS1件の行。有効性は `1`（有効）・`0`（無効）。
pub fn gps_row(r: &GpsRecord) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{}\n",
        r.recv_us,
        opt(r.utc, |u| u.to_iso8601()),
        opt(r.lat_e6, format_e6),
        opt(r.lon_e6, format_e6),
        opt(r.speed_kmh_x10, |v| format!("{}.{}", v / 10, v % 10)),
        u8::from(r.fix_valid),
        u8::from(r.time_valid),
        r.seq,
        r.session,
    )
}

/// 補助ログのイベントの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// 記録の開始
    LogStart,
    /// 記録の正常な停止
    LogStop,
    /// ファイルの分割（次のファイルへ移った）
    FileRotate,
    /// 受信キューのあふれ（`dropped` に失った件数）
    QueueOverflow,
    /// SDへの書き込みの失敗
    WriteError,
    /// 測位UTCとメイン機の時刻の対応（`utc` に、`t_us` に対応するUTC）
    TimeSync,
    /// GPSの測位が有効になった
    GpsFixAcquired,
    /// GPSの測位が無効になった
    GpsFixLost,
    /// サブ機とのWi-Fi接続が切れた
    WifiDisconnected,
    /// サブ機とのWi-Fi接続が回復した
    WifiConnected,
    /// 検出できたCANの通信異常（エラーフレーム等）
    CanError,
}

impl EventKind {
    /// ログに書く名前（固定。変更するとログの互換性がなくなる）。
    pub const fn name(self) -> &'static str {
        match self {
            EventKind::LogStart => "LOG_START",
            EventKind::LogStop => "LOG_STOP",
            EventKind::FileRotate => "FILE_ROTATE",
            EventKind::QueueOverflow => "QUEUE_OVERFLOW",
            EventKind::WriteError => "WRITE_ERROR",
            EventKind::TimeSync => "TIME_SYNC",
            EventKind::GpsFixAcquired => "GPS_FIX_ACQUIRED",
            EventKind::GpsFixLost => "GPS_FIX_LOST",
            EventKind::WifiDisconnected => "WIFI_DISCONNECTED",
            EventKind::WifiConnected => "WIFI_CONNECTED",
            EventKind::CanError => "CAN_ERROR",
        }
    }
}

/// 補助ログの1件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event<'a> {
    pub t_us: u64,
    pub kind: EventKind,
    /// 時刻対応（`TimeSync`）などで、`t_us` に対応するUTC
    pub utc: Option<Utc>,
    /// 失った件数（`QueueOverflow` など）
    pub dropped: Option<u32>,
    /// 補足（自由な文字列。`source=DEMO` など）
    pub detail: &'a str,
}

/// CSVの欄にする。`,`・`"`・改行を含む場合は、`"` で囲み、中の `"` は `""` にする。
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 補助ログ1件の行。
pub fn event_row(e: &Event<'_>) -> String {
    format!(
        "{},{},{},{},{}\n",
        e.t_us,
        e.kind.name(),
        opt(e.utc, |u| u.to_iso8601()),
        opt(e.dropped, |d| d.to_string()),
        csv_field(e.detail),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc() -> Utc {
        Utc {
            year: 2026,
            month: 11,
            day: 9,
            hour: 12,
            minute: 34,
            second: 56,
            millis: 7,
        }
    }

    #[test]
    fn ヘッダーの列数は行の列数と一致する() {
        let cols = |h: &str| h.trim_end().split(',').count();
        let frame = CanFrame {
            id: 0x7E0,
            format: IdFormat::Standard,
            kind: FrameKind::Data,
            dlc: 2,
            data: [0xAB, 0xCD, 0, 0, 0, 0, 0, 0],
        };
        assert_eq!(
            cols(CAN_HEADER),
            can_row(1, &frame).trim_end().split(',').count()
        );
        let g = GpsRecord {
            recv_us: 1,
            utc: Some(utc()),
            lat_e6: Some(1),
            lon_e6: Some(2),
            speed_kmh_x10: Some(3),
            fix_valid: true,
            time_valid: true,
            seq: 4,
            session: 5,
        };
        assert_eq!(cols(GPS_HEADER), gps_row(&g).trim_end().split(',').count());
        let e = Event {
            t_us: 1,
            kind: EventKind::LogStart,
            utc: None,
            dropped: None,
            detail: "",
        };
        assert_eq!(
            cols(EVENTS_HEADER),
            event_row(&e).trim_end().split(',').count()
        );
    }

    #[test]
    fn 標準フレームの行() {
        let f = CanFrame {
            id: 0x7E,
            format: IdFormat::Standard,
            kind: FrameKind::Data,
            dlc: 3,
            data: [0x01, 0xAB, 0xFF, 9, 9, 9, 9, 9],
        };
        assert_eq!(can_row(123_456, &f), "123456,07E,STD,DATA,3,01ABFF\n");
    }

    #[test]
    fn 拡張フレームの行() {
        let f = CanFrame {
            id: 0x1FFF_FFFF,
            format: IdFormat::Extended,
            kind: FrameKind::Data,
            dlc: 8,
            data: [0, 1, 2, 3, 4, 5, 6, 7],
        };
        assert_eq!(can_row(0, &f), "0,1FFFFFFF,EXT,DATA,8,0001020304050607\n");
    }

    #[test]
    fn データなしのフレームはデータ欄が空() {
        let mut f = CanFrame {
            id: 0x100,
            format: IdFormat::Standard,
            kind: FrameKind::Data,
            dlc: 0,
            data: [0; 8],
        };
        assert_eq!(can_row(5, &f), "5,100,STD,DATA,0,\n");
        f.kind = FrameKind::Remote;
        f.dlc = 4;
        assert_eq!(can_row(5, &f), "5,100,STD,REMOTE,4,\n");
        f.kind = FrameKind::Error;
        assert_eq!(can_row(5, &f), "5,100,STD,ERROR,4,\n");
    }

    #[test]
    fn 範囲外のフレームも省略せず記録する() {
        let f = CanFrame {
            id: 0x800, // 標準フレームとしては範囲外
            format: IdFormat::Standard,
            kind: FrameKind::Data,
            dlc: 9, // DLCも範囲外
            data: [0xAA; 8],
        };
        assert_eq!(can_row(1, &f), "1,800,STD,DATA,9,AAAAAAAAAAAAAAAA\n");
    }

    #[test]
    fn utcの表示() {
        assert_eq!(utc().to_iso8601(), "2026-11-09T12:34:56.007Z");
    }

    #[test]
    fn utcの妥当性() {
        assert!(utc().is_plausible());
        assert!(!Utc { month: 13, ..utc() }.is_plausible());
        assert!(!Utc { month: 0, ..utc() }.is_plausible());
        assert!(!Utc { hour: 24, ..utc() }.is_plausible());
        assert!(!Utc {
            millis: 1000,
            ..utc()
        }
        .is_plausible());
        assert!(Utc {
            second: 60,
            ..utc()
        }
        .is_plausible()); // うるう秒
    }

    #[test]
    fn 緯度経度の小数表示() {
        assert_eq!(format_e6(35_681_236), "35.681236");
        assert_eq!(format_e6(-33_868_820), "-33.868820");
        assert_eq!(format_e6(5), "0.000005");
        assert_eq!(format_e6(-5), "-0.000005");
        assert_eq!(format_e6(0), "0.000000");
    }

    #[test]
    fn gpsの行() {
        let g = GpsRecord {
            recv_us: 9_000_000,
            utc: Some(utc()),
            lat_e6: Some(35_681_236),
            lon_e6: Some(139_767_125),
            speed_kmh_x10: Some(593),
            fix_valid: true,
            time_valid: true,
            seq: 42,
            session: 3,
        };
        assert_eq!(
            gps_row(&g),
            "9000000,2026-11-09T12:34:56.007Z,35.681236,139.767125,59.3,1,1,42,3\n"
        );
    }

    #[test]
    fn gpsの未測位は欠損を空欄で表す() {
        let g = GpsRecord {
            recv_us: 1,
            utc: None,
            lat_e6: None,
            lon_e6: None,
            speed_kmh_x10: None,
            fix_valid: false,
            time_valid: false,
            seq: 0,
            session: 0,
        };
        assert_eq!(gps_row(&g), "1,,,,,0,0,0,0\n");
    }

    #[test]
    fn 速度0は空欄ではなく0として記録する() {
        let g = GpsRecord {
            recv_us: 1,
            utc: None,
            lat_e6: None,
            lon_e6: None,
            speed_kmh_x10: Some(0),
            fix_valid: true,
            time_valid: false,
            seq: 1,
            session: 1,
        };
        assert_eq!(gps_row(&g), "1,,,,0.0,1,0,1,1\n");
    }

    #[test]
    fn csvの欄の引用() {
        assert_eq!(csv_field("source=DEMO"), "source=DEMO");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn イベントの行() {
        let e = Event {
            t_us: 100,
            kind: EventKind::QueueOverflow,
            utc: None,
            dropped: Some(12),
            detail: "can queue, full",
        };
        assert_eq!(
            event_row(&e),
            "100,QUEUE_OVERFLOW,,12,\"can queue, full\"\n"
        );
    }

    #[test]
    fn 時刻対応のイベントはutcを持つ() {
        let e = Event {
            t_us: 5_000_000,
            kind: EventKind::TimeSync,
            utc: Some(utc()),
            dropped: None,
            detail: "",
        };
        assert_eq!(
            event_row(&e),
            "5000000,TIME_SYNC,2026-11-09T12:34:56.007Z,,\n"
        );
    }

    #[test]
    fn イベント名は重複しない() {
        use EventKind::*;
        let all = [
            LogStart,
            LogStop,
            FileRotate,
            QueueOverflow,
            WriteError,
            TimeSync,
            GpsFixAcquired,
            GpsFixLost,
            WifiDisconnected,
            WifiConnected,
            CanError,
        ];
        let mut names: Vec<_> = all.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}

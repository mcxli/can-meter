//! ログ（`docs/実装仕様/ログ形式.md`）の確認。CSVを読み、形式・時刻・周期・欠落を集計する。
//!
//! 外部のクレートを使わず、`shared` の書式と復号の候補（`decode`）を使う。

use shared::can::{CanFrame, FrameKind, IdFormat};
use shared::decode::{decode, Decoded};
use shared::log_format::{CAN_HEADER, EVENTS_HEADER, GPS_HEADER};
use std::collections::BTreeMap;

/// 1行の読み取りの誤り。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// CANログの1行を読む（`t_us,id,format,kind,dlc,data`）。
pub fn parse_can_line(line: &str) -> Result<(u64, CanFrame), String> {
    let c: Vec<&str> = line.split(',').collect();
    if c.len() != 6 {
        return Err(format!("列数が6でない（{}列）", c.len()));
    }
    let t_us = c[0]
        .parse::<u64>()
        .map_err(|_| "t_us が整数でない".to_string())?;
    let id = u32::from_str_radix(c[1], 16).map_err(|_| "id が16進でない".to_string())?;
    let format = match c[2] {
        "STD" => IdFormat::Standard,
        "EXT" => IdFormat::Extended,
        _ => return Err(format!("format が不明（{}）", c[2])),
    };
    let kind = match c[3] {
        "DATA" => FrameKind::Data,
        "REMOTE" => FrameKind::Remote,
        "ERROR" => FrameKind::Error,
        _ => return Err(format!("kind が不明（{}）", c[3])),
    };
    let dlc = c[4]
        .parse::<u8>()
        .map_err(|_| "dlc が整数でない".to_string())?;
    let hex = c[5];
    if hex.len() % 2 != 0 || hex.len() > 16 {
        return Err("data の長さが不正".to_string());
    }
    let mut data = [0u8; 8];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| "data が16進でない".to_string())?;
        data[i] = u8::from_str_radix(s, 16).map_err(|_| "data が16進でない".to_string())?;
    }
    if kind == FrameKind::Data && usize::from(dlc.min(8)) != hex.len() / 2 {
        return Err(format!(
            "dlc（{dlc}）と data の長さ（{}）が合わない",
            hex.len() / 2
        ));
    }
    Ok((
        t_us,
        CanFrame {
            id,
            format,
            kind,
            dlc,
            data,
        },
    ))
}

/// IDごとの集計。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdStats {
    pub count: u64,
    pub first_us: u64,
    pub last_us: u64,
    pub max_gap_us: u64,
}

impl IdStats {
    /// 平均の受信周期 [Hz]。2件未満なら `None`。
    pub fn rate_hz(&self) -> Option<f64> {
        (self.count >= 2 && self.last_us > self.first_us)
            .then(|| (self.count - 1) as f64 / ((self.last_us - self.first_us) as f64 / 1e6))
    }
}

/// 復号の候補値の範囲（最小・最大）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub n: u64,
}

impl Range {
    fn add(&mut self, v: f64) {
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.n += 1;
    }
}

/// ログ全体の確認結果。
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub can_rows: u64,
    pub ids: BTreeMap<(bool, u32), IdStats>,
    pub first_us: Option<u64>,
    pub last_us: Option<u64>,
    /// 時刻が前の行より小さかった件数（ファイルをまたぐ場合も含む）
    pub time_reversals: u64,
    pub gps_rows: u64,
    pub gps_seq_missing: u64,
    pub events: BTreeMap<String, u64>,
    pub dropped_total: u64,
    pub decoded: BTreeMap<&'static str, Range>,
    pub errors: Vec<LineError>,
}

impl Report {
    fn add_decoded(&mut self, d: &Decoded) {
        let mut put = |k: &'static str, v: Option<f64>| {
            if let Some(v) = v {
                self.decoded.entry(k).or_default().add(v);
            }
        };
        put("回転数 [rpm]（0x140）", d.rpm.map(f64::from));
        put("アクセル開度 [%]（0x140）", d.accel_pct.map(f64::from));
        put("油温 [℃?]（0x360）", d.oil_temp_c.map(f64::from));
        put("水温 [℃?]（0x360）", d.water_temp_c.map(f64::from));
        put("車速 [km/h]（0xD1）", d.speed_kmh.map(f64::from));
        put(
            "舵角 [度]（0xD0）",
            d.steering_deg_x10.map(|v| f64::from(v) / 10.0),
        );
        put("ギアの生の値（0x141）", d.gear_raw.map(f64::from));
    }

    /// CANログの本文（ヘッダーを除く行）を取り込む。
    pub fn add_can(&mut self, file: &str, text: &str) {
        let mut lines = text.split('\n');
        if lines.next().map(|h| format!("{h}\n")) != Some(CAN_HEADER.to_string()) {
            self.errors.push(LineError {
                file: file.into(),
                line: 1,
                message: "ヘッダーが仕様と違う".into(),
            });
        }
        if !text.ends_with('\n') {
            self.errors.push(LineError {
                file: file.into(),
                line: 0,
                message: "末尾が改行で終わっていない（最後の行が途中で切れている可能性）".into(),
            });
        }
        for (i, line) in lines.enumerate().filter(|(_, l)| !l.is_empty()) {
            match parse_can_line(line) {
                Ok((t, f)) => {
                    self.can_rows += 1;
                    if let Some(prev) = self.last_us {
                        if t < prev {
                            self.time_reversals += 1;
                        }
                    }
                    self.first_us.get_or_insert(t);
                    self.last_us = Some(t);
                    let s = self
                        .ids
                        .entry((f.format == IdFormat::Extended, f.id))
                        .or_default();
                    if s.count == 0 {
                        s.first_us = t;
                    } else {
                        s.max_gap_us = s.max_gap_us.max(t.saturating_sub(s.last_us));
                    }
                    s.count += 1;
                    s.last_us = t;
                    self.add_decoded(&decode(&f));
                }
                Err(m) => self.errors.push(LineError {
                    file: file.into(),
                    line: i + 2,
                    message: m,
                }),
            }
        }
    }

    /// GPSログを取り込む（連番の欠番を数える）。
    pub fn add_gps(&mut self, file: &str, text: &str, last_seq: &mut Option<u32>) {
        let mut lines = text.split('\n');
        if lines.next().map(|h| format!("{h}\n")) != Some(GPS_HEADER.to_string()) {
            self.errors.push(LineError {
                file: file.into(),
                line: 1,
                message: "ヘッダーが仕様と違う".into(),
            });
        }
        for (i, line) in lines.enumerate().filter(|(_, l)| !l.is_empty()) {
            let c: Vec<&str> = line.split(',').collect();
            let seq = (c.len() == 9).then(|| c[7].parse::<u32>().ok()).flatten();
            match seq {
                Some(seq) => {
                    self.gps_rows += 1;
                    if let Some(prev) = *last_seq {
                        self.gps_seq_missing +=
                            u64::from(seq.saturating_sub(prev).saturating_sub(1));
                    }
                    *last_seq = Some(seq);
                }
                None => self.errors.push(LineError {
                    file: file.into(),
                    line: i + 2,
                    message: "列数が9でない、または seq が不正".into(),
                }),
            }
        }
    }

    /// 補助ログを取り込む（イベントの件数、あふれの件数の合計）。
    pub fn add_events(&mut self, file: &str, text: &str) {
        let mut lines = text.split('\n');
        if lines.next().map(|h| format!("{h}\n")) != Some(EVENTS_HEADER.to_string()) {
            self.errors.push(LineError {
                file: file.into(),
                line: 1,
                message: "ヘッダーが仕様と違う".into(),
            });
        }
        for line in lines.filter(|l| !l.is_empty()) {
            let c: Vec<&str> = line.splitn(5, ',').collect();
            if c.len() < 4 {
                continue;
            }
            *self.events.entry(c[1].to_string()).or_default() += 1;
            if c[1] == "QUEUE_OVERFLOW" {
                self.dropped_total += c[3].parse::<u64>().unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can行の読み取り() {
        let (t, f) = parse_can_line("123,140,STD,DATA,8,0000881300000000").unwrap();
        assert_eq!(t, 123);
        assert_eq!(f.id, 0x140);
        assert_eq!(decode(&f).rpm, Some(5000));
        let (_, f) = parse_can_line("5,1FFFFFFF,EXT,REMOTE,4,").unwrap();
        assert_eq!(f.format, IdFormat::Extended);
        assert_eq!(f.kind, FrameKind::Remote);
    }

    #[test]
    fn 不正な行を検出する() {
        assert!(parse_can_line("1,140,STD,DATA,8").is_err());
        assert!(parse_can_line("x,140,STD,DATA,0,").is_err());
        assert!(parse_can_line("1,14G,STD,DATA,0,").is_err());
        assert!(parse_can_line("1,140,XXX,DATA,0,").is_err());
        assert!(parse_can_line("1,140,STD,DATA,2,00").is_err()); // dlcと長さが合わない
        assert!(parse_can_line("1,140,STD,DATA,1,0").is_err()); // 奇数桁
    }

    #[test]
    fn 集計() {
        let can = "t_us,id,format,kind,dlc,data\n\
                   1000,140,STD,DATA,8,3D00881300000000\n\
                   11000,140,STD,DATA,8,3D00881300000000\n\
                   21000,140,STD,DATA,8,3D00881300000000\n\
                   21500,360,STD,DATA,8,0000888000000000\n";
        let mut r = Report::default();
        r.add_can("can_001.csv", can);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.can_rows, 4);
        let s = &r.ids[&(false, 0x140)];
        assert_eq!(s.count, 3);
        assert_eq!(s.max_gap_us, 10_000);
        assert!((s.rate_hz().unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(r.decoded["回転数 [rpm]（0x140）"].max, 5000.0);
        assert_eq!(r.decoded["水温 [℃?]（0x360）"].min, 88.0);
        assert_eq!(r.time_reversals, 0);
    }

    #[test]
    fn 時刻の逆転と末尾の切れを検出する() {
        let mut r = Report::default();
        r.add_can(
            "a",
            "t_us,id,format,kind,dlc,data\n20,100,STD,DATA,0,\n10,100,STD,DATA,0,",
        );
        assert_eq!(r.time_reversals, 1);
        assert!(r.errors.iter().any(|e| e.message.contains("末尾")));
    }

    #[test]
    fn gpsの欠番とあふれの合計() {
        let mut r = Report::default();
        let mut last = None;
        r.add_gps("g", "t_us,utc,lat_deg,lon_deg,speed_kmh,fix_valid,time_valid,seq,session\n1,,,,,0,0,0,1\n2,,,,,0,0,1,1\n3,,,,,0,0,4,1\n", &mut last);
        assert_eq!(r.gps_rows, 3);
        assert_eq!(r.gps_seq_missing, 2);
        r.add_events("e", "t_us,event,utc,dropped,detail\n1,LOG_START,,,source=DEMO\n2,QUEUE_OVERFLOW,,52,\n3,QUEUE_OVERFLOW,,8,\"a,b\"\n4,LOG_STOP,,,\n");
        assert_eq!(r.dropped_total, 60);
        assert_eq!(r.events["LOG_STOP"], 1);
    }
}

//! SDへのログ保存（段階5）。受信・描画から分離した書き込みスレッドと、有限長のキューで構成する。
//!
//! 書式・保存先・分割・同期のタイミングは `shared`（`log_format`・`log_policy`）が決める。ここは、ファイルを書く部分。
//! SDの遅延で送る側を長時間待たせないよう、キューが満杯のときは、待たずに捨てて件数を数える（`QUEUE_OVERFLOW`）。

use esp_idf_svc::sys::esp_timer_get_time;
use shared::can::{CanFrame, FrameKind, IdFormat};
use shared::log_format::{
    can_row, event_row, gps_row, Event, EventKind, GpsRecord, Utc, CAN_HEADER, EVENTS_HEADER, GPS_HEADER,
};
use shared::log_policy::{file_name, next_session_dir, LogKind, Rotator, SyncPolicy};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

/// 受信キューの長さ（件数）。
const QUEUE_LEN: usize = 512;

/// 記録の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogState {
    /// 記録を開始していない（SDの準備前）
    Idle = 0,
    /// 記録中
    Recording = 1,
    /// 停止の処理中（残りのデータを書き出している）
    Stopping = 2,
    /// 正常に停止した（残りを書き出し、同期し、ファイルを閉じた）。電源断してよい
    Stopped = 3,
    /// SDの異常で記録できない（未挿入・容量不足・書き込み失敗など）
    Error = 4,
}

static STATE: AtomicU8 = AtomicU8::new(LogState::Idle as u8);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
/// キューあふれで捨てた件数（累計）。
static DROPPED: AtomicU32 = AtomicU32::new(0);
/// 負荷試験：仮データの送り手に、短時間に大量のフレームを送らせる要求（キューあふれの確認用）。
static BURST_REQUESTED: AtomicBool = AtomicBool::new(false);
/// 負荷試験で、一度に送るフレーム数（キュー長 512 より十分に多い）。
const BURST_FRAMES: u32 = 3000;

pub fn state() -> LogState {
    match STATE.load(Ordering::Relaxed) {
        1 => LogState::Recording,
        2 => LogState::Stopping,
        3 => LogState::Stopped,
        4 => LogState::Error,
        _ => LogState::Idle,
    }
}

fn set_state(s: LogState) {
    STATE.store(s as u8, Ordering::Relaxed);
}

/// 記録を停止する（「記録停止」ボタン）。残りのデータを書き出して、ファイルを閉じる。
pub fn request_stop() {
    if state() == LogState::Recording {
        STOP_REQUESTED.store(true, Ordering::Relaxed);
    }
}

/// 負荷試験：仮データの送り手に、短時間に大量のフレームを送らせる（仮データのときだけ使う）。
/// 書き込みが追いつかず、キューがあふれて、`QUEUE_OVERFLOW` が記録されることを確認するため。
pub fn request_demo_burst() {
    BURST_REQUESTED.store(true, Ordering::Relaxed);
}

/// SDの準備に失敗したなど、記録を開始できないことを知らせる。
pub fn set_error() {
    set_state(LogState::Error);
}

/// メイン機の単調増加時刻（起動からの経過時間、マイクロ秒）。
pub fn now_us() -> u64 {
    // SAFETY: esp_timer_get_time は、起動後いつでも呼べる
    unsafe { esp_timer_get_time() as u64 }
}

/// 書き込みスレッドへ送るメッセージ。
pub enum Msg {
    Can(u64, CanFrame),
    Gps(GpsRecord),
    Event {
        t_us: u64,
        kind: EventKind,
        utc: Option<Utc>,
        dropped: Option<u32>,
        detail: String,
    },
}

/// 受信側から、待たずに送る。満杯のときは捨てて件数を数える。送れなければ `false`（書き込み側が終了している）。
pub fn send(tx: &SyncSender<Msg>, msg: Msg) -> bool {
    match tx.try_send(msg) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// ログ1種類のファイル。分割の境界で、前のファイルを閉じて（同期して）次のファイルを開く。
struct KindWriter {
    kind: LogKind,
    header: &'static str,
    dir: PathBuf,
    rotator: Rotator,
    file: Option<BufWriter<File>>,
    index: Option<u32>,
}

impl KindWriter {
    fn new(kind: LogKind, header: &'static str, dir: PathBuf, start_us: u64) -> Self {
        Self {
            kind,
            header,
            dir,
            rotator: Rotator::new(start_us),
            file: None,
            index: None,
        }
    }

    /// 1行を書く。新しいファイルを開いたときは、そのファイルの番号（0始まり）を返す。
    fn write(&mut self, t_us: u64, row: &str) -> io::Result<Option<u32>> {
        let place = self.rotator.place(t_us);
        let mut opened = None;
        if self.file.is_none() || self.index != Some(place.index) {
            self.close()?;
            // 既存のファイルを上書きしない（create_new）
            let path = self.dir.join(file_name(self.kind, place.index));
            let f = OpenOptions::new().write(true).create_new(true).open(path)?;
            let mut w = BufWriter::new(f);
            w.write_all(self.header.as_bytes())?;
            self.file = Some(w);
            self.index = Some(place.index);
            opened = Some(place.index);
        }
        if let Some(w) = self.file.as_mut() {
            w.write_all(row.as_bytes())?;
        }
        Ok(opened)
    }

    /// バッファを書き出し、ファイルシステムへ同期する。
    fn sync(&mut self) -> io::Result<()> {
        if let Some(w) = self.file.as_mut() {
            w.flush()?;
            w.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// 同期して、ファイルを閉じる。
    fn close(&mut self) -> io::Result<()> {
        if let Some(mut w) = self.file.take() {
            w.flush()?;
            w.get_ref().sync_all()?;
        }
        Ok(())
    }
}

struct Writers {
    can: KindWriter,
    gps: KindWriter,
    events: KindWriter,
}

impl Writers {
    fn sync_all(&mut self) -> io::Result<()> {
        self.can.sync()?;
        self.gps.sync()?;
        self.events.sync()
    }

    fn close_all(&mut self) -> io::Result<()> {
        let a = self.can.close();
        let b = self.gps.close();
        let c = self.events.close();
        a.and(b).and(c)
    }

    fn write_event(&mut self, e: &Event<'_>) -> io::Result<()> {
        self.events.write(e.t_us, &event_row(e)).map(|_| ())
    }

    fn handle(&mut self, msg: Msg) -> io::Result<()> {
        match msg {
            Msg::Can(t_us, frame) => {
                if let Some(index) = self.can.write(t_us, &can_row(t_us, &frame))? {
                    if index > 0 {
                        self.write_event(&Event {
                            t_us,
                            kind: EventKind::FileRotate,
                            utc: None,
                            dropped: None,
                            detail: &format!("can_index={}", index + 1),
                        })?;
                    }
                }
            }
            Msg::Gps(r) => {
                self.gps.write(r.recv_us, &gps_row(&r))?;
            }
            Msg::Event { t_us, kind, utc, dropped, detail } => {
                self.write_event(&Event { t_us, kind, utc, dropped, detail: &detail })?;
            }
        }
        Ok(())
    }
}

/// 記録を開始する。SDの `mount` の下に、起動ごとの保存先を作り、書き込みスレッドを起動する。
///
/// 成功したら、送信側を返す。`demo` が `true` のときは、保存先の名前と、`LOG_START` の `detail` に、
/// 仮データであることを残す（実車のログと区別する）。
pub fn start(mount: &str, demo: bool) -> Result<SyncSender<Msg>, String> {
    let existing: Vec<String> = fs::read_dir(mount)
        .map_err(|e| format!("保存先を読めない: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
    let name = next_session_dir(&refs, demo).ok_or("保存先の番号を使い切った")?;
    let dir = PathBuf::from(mount).join(&name);
    fs::create_dir(&dir).map_err(|e| format!("保存先を作れない: {e}"))?;
    log::info!("ログの保存先: {}", dir.display());

    let (tx, rx) = sync_channel(QUEUE_LEN);
    let start_us = now_us();
    set_state(LogState::Recording);
    std::thread::Builder::new()
        .name("log_writer".into())
        .stack_size(12 * 1024)
        .spawn(move || writer_loop(rx, dir, start_us, demo))
        .map_err(|e| format!("書き込みスレッドを起動できない: {e}"))?;
    Ok(tx)
}

fn writer_loop(rx: Receiver<Msg>, dir: PathBuf, start_us: u64, demo: bool) {
    let mut w = Writers {
        can: KindWriter::new(LogKind::Can, CAN_HEADER, dir.clone(), start_us),
        gps: KindWriter::new(LogKind::Gps, GPS_HEADER, dir.clone(), start_us),
        events: KindWriter::new(LogKind::Events, EVENTS_HEADER, dir, start_us),
    };
    let mut sync = SyncPolicy::new();
    let mut reported_dropped = 0u32;
    let mut failed = false;

    // 失敗したら、記録不可を通知し、以降の書き込みは行わない（受信側は、送り続けて捨てられる）
    let fail = |w: &mut Writers, what: &str, e: &io::Error| {
        log::error!("{what}: {e}");
        set_state(LogState::Error);
        // 補助ログへの記録は、できる範囲で試みる（失敗しても無視）
        let _ = w.write_event(&Event {
            t_us: now_us(),
            kind: EventKind::WriteError,
            utc: None,
            dropped: None,
            detail: &format!("{what}: {e}"),
        });
    };

    let source = if demo { "source=DEMO" } else { "source=VEHICLE" };
    let start = Event {
        t_us: now_us(),
        kind: EventKind::LogStart,
        utc: None,
        dropped: None,
        detail: source,
    };
    if let Err(e) = w.write_event(&start) {
        fail(&mut w, "LOG_START", &e);
        failed = true;
    }

    loop {
        let stop = STOP_REQUESTED.load(Ordering::Relaxed);
        if stop {
            set_state(LogState::Stopping);
            // 残りのデータを書き出す
            while let Ok(msg) = rx.try_recv() {
                if !failed {
                    if let Err(e) = w.handle(msg) {
                        fail(&mut w, "書き込み", &e);
                        failed = true;
                    }
                }
            }
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => {
                if !failed {
                    if let Err(e) = w.handle(msg) {
                        fail(&mut w, "書き込み", &e);
                        failed = true;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if failed {
            continue;
        }
        // キューあふれの検出件数を、補助ログに残す
        let dropped = DROPPED.load(Ordering::Relaxed);
        if dropped != reported_dropped {
            let lost = dropped - reported_dropped;
            reported_dropped = dropped;
            let e = Event {
                t_us: now_us(),
                kind: EventKind::QueueOverflow,
                utc: None,
                dropped: Some(lost),
                detail: "",
            };
            if let Err(err) = w.write_event(&e) {
                fail(&mut w, "QUEUE_OVERFLOW", &err);
                failed = true;
                continue;
            }
        }
        // 1秒ごとに、ファイルシステムへ同期する
        if sync.due(now_us()) {
            if let Err(e) = w.sync_all() {
                fail(&mut w, "同期", &e);
                failed = true;
            }
        }
    }

    // 正常な停止：LOG_STOPを書き、同期して、ファイルを閉じる。失敗したら、停止成功の扱いにしない
    if !failed {
        let stop_event = Event {
            t_us: now_us(),
            kind: EventKind::LogStop,
            utc: None,
            dropped: None,
            detail: "",
        };
        let result = w.write_event(&stop_event).and_then(|()| w.close_all());
        match result {
            Ok(()) => {
                log::info!("ログを正常に停止した");
                set_state(LogState::Stopped);
            }
            Err(e) => fail(&mut w, "停止処理", &e),
        }
    } else {
        let _ = w.close_all();
    }
}

/// 仮データのCAN・GPSを、一定の周期でキューへ送る（段階5の動作確認用。実車のCANは段階6）。
///
/// 仮のフレームは、ID `0x100`〜`0x107` を順に使い、データは連番から作る。記録が停止したら終了する。
pub fn spawn_demo_producer(tx: SyncSender<Msg>) {
    let _ = std::thread::Builder::new()
        .name("log_demo".into())
        .stack_size(8 * 1024)
        .spawn(move || {
            let mut n: u32 = 0;
            let mut gps_seq: u32 = 0;
            let mut first_gps = true;
            while state() == LogState::Recording {
                if BURST_REQUESTED.swap(false, Ordering::Relaxed) {
                    // 待たずに大量に送る。連番（データ）は、捨てた分も進むため、記録の欠番の数が、捨てた件数と一致する
                    for _ in 0..BURST_FRAMES {
                        let burst = demo_frame(n);
                        if !send(&tx, Msg::Can(now_us(), burst)) {
                            return;
                        }
                        n = n.wrapping_add(1);
                    }
                }
                let t_us = now_us();
                let frame = demo_frame(n);
                if !send(&tx, Msg::Can(t_us, frame)) {
                    break;
                }
                n = n.wrapping_add(1);
                // 約1秒（100フレーム）ごとに、GPSの仮データを送る
                if n % 100 == 0 {
                    let utc = demo_utc(t_us);
                    if first_gps {
                        first_gps = false;
                        send(
                            &tx,
                            Msg::Event {
                                t_us,
                                kind: EventKind::TimeSync,
                                utc: Some(utc),
                                dropped: None,
                                detail: "source=DEMO".into(),
                            },
                        );
                    }
                    let r = GpsRecord {
                        recv_us: t_us,
                        utc: Some(utc),
                        lat_e6: Some(35_681_236),
                        lon_e6: Some(139_767_125),
                        speed_kmh_x10: Some(593),
                        fix_valid: true,
                        time_valid: true,
                        seq: gps_seq,
                        session: 1,
                    };
                    gps_seq += 1;
                    if !send(&tx, Msg::Gps(r)) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
}

/// 仮のCANフレーム。ID `0x100`〜`0x107` を順に使い、データは連番から作る。
fn demo_frame(n: u32) -> CanFrame {
    CanFrame {
        id: 0x100 + (n % 8),
        format: IdFormat::Standard,
        kind: FrameKind::Data,
        dlc: 8,
        data: (u64::from(n) << 8).to_be_bytes(),
    }
}

/// 仮のUTC。起動からの経過に合わせて進める（2026-11-09 12:34:56 を起点にした仮の値）。
fn demo_utc(t_us: u64) -> Utc {
    let total = 12 * 3600 + 34 * 60 + 56 + (t_us / 1_000_000) % 40_000;
    Utc {
        year: 2026,
        month: 11,
        day: 9,
        hour: (total / 3600 % 24) as u8,
        minute: (total / 60 % 60) as u8,
        second: (total % 60) as u8,
        millis: ((t_us / 1000) % 1000) as u16,
    }
}

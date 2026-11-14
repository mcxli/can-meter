//! 使い方：`cargo run --release -- <保存先のディレクトリ>`（例：`/Volumes/SDCARD/LOG_0010_DEMO`）
//!
//! `can_*.csv`・`gps_*.csv`・`events_*.csv` を番号順に読み、確認結果を表示する。

use logcheck::Report;
use std::path::Path;
use std::process::ExitCode;

fn files(dir: &Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".csv"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("使い方: logcheck <保存先のディレクトリ>");
        return ExitCode::from(2);
    };
    let dir = Path::new(&dir);
    let mut r = Report::default();
    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    for p in files(dir, "can_") {
        r.add_can(&p.display().to_string(), &read(&p));
    }
    let mut last_seq = None;
    for p in files(dir, "gps_") {
        r.add_gps(&p.display().to_string(), &read(&p), &mut last_seq);
    }
    for p in files(dir, "events_") {
        r.add_events(&p.display().to_string(), &read(&p));
    }

    println!("== {}", dir.display());
    let dur = match (r.first_us, r.last_us) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 / 1e6,
        _ => 0.0,
    };
    println!(
        "CAN: {}行、{:.1}秒、{:.1}フレーム/秒、時刻の逆転 {}件",
        r.can_rows,
        dur,
        if dur > 0.0 {
            r.can_rows as f64 / dur
        } else {
            0.0
        },
        r.time_reversals
    );
    println!("GPS: {}行、連番の欠番 {}件", r.gps_rows, r.gps_seq_missing);
    println!("イベント: {:?}", r.events);
    println!(
        "キューあふれで捨てた件数（QUEUE_OVERFLOW の合計）: {}",
        r.dropped_total
    );
    println!("\nIDごと（件数・平均周期・最大間隔）");
    for ((ext, id), s) in &r.ids {
        let id = if *ext {
            format!("{id:08X}")
        } else {
            format!("{id:03X}")
        };
        let rate = s
            .rate_hz()
            .map(|h| format!("{h:7.1}Hz"))
            .unwrap_or_else(|| "      -".into());
        println!(
            "  {id:>8}  {:>8}件  {rate}  最大間隔 {:.1}ms",
            s.count,
            s.max_gap_us as f64 / 1000.0
        );
    }
    println!("\n復号の候補値（公開情報による候補。実車で照合するまで確定ではない）");
    if r.decoded.is_empty() {
        println!("  （対象のIDなし）");
    }
    for (k, v) in &r.decoded {
        println!("  {k}: 最小 {} 〜 最大 {}（{}件）", v.min, v.max, v.n);
    }
    if r.errors.is_empty() {
        println!("\n形式の誤り: なし");
        ExitCode::SUCCESS
    } else {
        println!("\n形式の誤り: {}件（先頭10件）", r.errors.len());
        for e in r.errors.iter().take(10) {
            println!("  {}:{}: {}", e.file, e.line, e.message);
        }
        ExitCode::FAILURE
    }
}

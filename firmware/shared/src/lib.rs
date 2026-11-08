//! CANメーターの共通ロジック。
//!
//! ESP-IDF・LVGLなどのハードウェア依存を持たない。`firmware/main`・`firmware/gps` から使い、
//! Macのホストで `cargo test` により検証する。

pub mod demo;
pub mod reading;
pub mod shift_notice;
pub mod vehicle;

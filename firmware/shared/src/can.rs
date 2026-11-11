//! CANフレーム（受信専用）。
//!
//! CAN通信はListen-onlyの受信専用で、フレームを送信しない（要件 CAN-04）。ここは受信したフレームの表現だけを持つ。

/// フレーム形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdFormat {
    /// 標準フレーム（11ビットID）
    Standard,
    /// 拡張フレーム（29ビットID）
    Extended,
}

/// フレームの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// データフレーム
    Data,
    /// リモートフレーム（データなし）
    Remote,
    /// エラーフレーム（検出できたもの）
    Error,
}

/// 受信したCANフレーム。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub format: IdFormat,
    pub kind: FrameKind,
    /// データ長コード（0〜8）
    pub dlc: u8,
    /// データ。先頭から `dlc` バイトが有効（データフレームのみ）
    pub data: [u8; 8],
}

impl CanFrame {
    /// IDの最大値（標準 0x7FF、拡張 0x1FFFFFFF）。
    pub const fn max_id(format: IdFormat) -> u32 {
        match format {
            IdFormat::Standard => 0x7FF,
            IdFormat::Extended => 0x1FFF_FFFF,
        }
    }

    /// フレームとして成り立っているか（IDの範囲、DLCの範囲）。
    pub fn is_well_formed(&self) -> bool {
        self.id <= Self::max_id(self.format) && self.dlc <= 8
    }

    /// 有効なデータ部分。データフレーム以外は空。
    pub fn payload(&self) -> &[u8] {
        match self.kind {
            FrameKind::Data => &self.data[..usize::from(self.dlc.min(8))],
            FrameKind::Remote | FrameKind::Error => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: FrameKind, dlc: u8) -> CanFrame {
        CanFrame {
            id: 0x7E0,
            format: IdFormat::Standard,
            kind,
            dlc,
            data: [1, 2, 3, 4, 5, 6, 7, 8],
        }
    }

    #[test]
    fn データフレームはdlc分だけが有効() {
        assert_eq!(frame(FrameKind::Data, 3).payload(), &[1, 2, 3]);
        assert_eq!(frame(FrameKind::Data, 0).payload(), &[] as &[u8]);
        assert_eq!(frame(FrameKind::Data, 8).payload().len(), 8);
    }

    #[test]
    fn リモートとエラーはデータなし() {
        assert!(frame(FrameKind::Remote, 4).payload().is_empty());
        assert!(frame(FrameKind::Error, 0).payload().is_empty());
    }

    #[test]
    fn idとdlcの範囲を検証する() {
        assert!(frame(FrameKind::Data, 8).is_well_formed());
        assert!(!frame(FrameKind::Data, 9).is_well_formed());
        let mut f = frame(FrameKind::Data, 1);
        f.id = 0x800;
        assert!(!f.is_well_formed());
        f.format = IdFormat::Extended;
        assert!(f.is_well_formed());
        f.id = 0x2000_0000;
        assert!(!f.is_well_formed());
    }

    #[test]
    fn 範囲外のdlcでもpayloadは境界を越えない() {
        // 不正なフレームでも、payloadがパニックしない
        assert_eq!(frame(FrameKind::Data, 200).payload().len(), 8);
    }
}

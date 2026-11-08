//! 車両情報の1項目（値・有効性・最終受信時刻）。
//!
//! 未受信・更新停止は無効値として扱い、取得済みの0と区別する（要件 CAN-08）。

/// 値と最終受信時刻を持つ項目。時刻はミリ秒（起動からの経過時間など、単調増加する値）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading<T> {
    value: Option<T>,
    updated_ms: u64,
}

impl<T: Copy> Reading<T> {
    /// 未受信の状態。
    pub const fn new() -> Self {
        Self {
            value: None,
            updated_ms: 0,
        }
    }

    /// 値を受信した。
    pub fn set(&mut self, value: T, now_ms: u64) {
        self.value = Some(value);
        self.updated_ms = now_ms;
    }

    /// 有効な値を返す。未受信、または最終受信から `timeout_ms` を超えて更新がない場合は `None`。
    pub fn get(&self, now_ms: u64, timeout_ms: u64) -> Option<T> {
        let value = self.value?;
        // 時刻が巻き戻った場合は、古い値を現在値として扱わない
        let age = now_ms.checked_sub(self.updated_ms)?;
        (age <= timeout_ms).then_some(value)
    }

    /// 受信済みの値を捨てる（異常値の検出時など）。
    pub fn invalidate(&mut self) {
        self.value = None;
    }
}

impl<T: Copy> Default for Reading<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 未受信は無効() {
        let r: Reading<u16> = Reading::new();
        assert_eq!(r.get(0, 1000), None);
    }

    #[test]
    fn 取得済みの0は有効で未受信と区別される() {
        let mut r = Reading::new();
        r.set(0u16, 100);
        assert_eq!(r.get(100, 1000), Some(0));
    }

    #[test]
    fn タイムアウト以内は有効() {
        let mut r = Reading::new();
        r.set(3200u16, 1000);
        assert_eq!(r.get(2000, 1000), Some(3200));
    }

    #[test]
    fn タイムアウトを超えると無効() {
        let mut r = Reading::new();
        r.set(3200u16, 1000);
        assert_eq!(r.get(2001, 1000), None);
    }

    #[test]
    fn 再受信すると有効に戻る() {
        let mut r = Reading::new();
        r.set(3200u16, 1000);
        assert_eq!(r.get(5000, 1000), None);
        r.set(3300, 5000);
        assert_eq!(r.get(5001, 1000), Some(3300));
    }

    #[test]
    fn 時刻が巻き戻った場合は無効() {
        let mut r = Reading::new();
        r.set(3200u16, 5000);
        assert_eq!(r.get(1000, 1000), None);
    }

    #[test]
    fn invalidateで無効になる() {
        let mut r = Reading::new();
        r.set(1u8, 0);
        r.invalidate();
        assert_eq!(r.get(0, 1000), None);
    }
}

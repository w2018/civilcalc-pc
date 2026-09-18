//! 云端备份的状态文案：设置页折叠栏里只显示「上次备份时间 + 累计次数」。
//!
//! 源：`civilcalc-android-v2/core/backup/BackupStats.kt`（39 行）
//!
//! ## 为什么「时间或次数任一为空」都视为「还没备份过」
//!
//! 这样「从没备份过」和「次数被清掉」两种情况给出同一句提示，
//! **不会出现「累计 0 次」这种别扭说法**。
//!
//! 纯函数（`now_ms` 可注入）便于单测。

use chrono::{Datelike, Local, Timelike};

/// 从未备份过的文案（源 `NEVER`）。
pub const NEVER: &str = "还没有备份过";

/// 状态文案：`上次备份：今天 14:30 · 累计 3 次`。
///
/// `last_backup_at_ms` 或 `count` 任一无效（`None` / `<= 0`）→ [`NEVER`]。
#[must_use]
pub fn describe(last_backup_at_ms: Option<i64>, count: i32, now_ms: i64) -> String {
    let Some(at) = last_backup_at_ms.filter(|ms| *ms > 0) else {
        return NEVER.to_string();
    };
    if count <= 0 {
        return NEVER.to_string();
    }
    format!(
        "上次备份：{} {} · 累计 {count} 次",
        day_label(at, now_ms),
        time_label(at)
    )
}

/// 今天 / 昨天 / `M月d日` / `yyyy年M月d日`（按本地时区，跨年直接给绝对日期）。
fn day_label(at_ms: i64, now_ms: i64) -> String {
    let (Some(at), Some(now)) = (to_local(at_ms), to_local(now_ms)) else {
        return String::new();
    };
    if at.year() == now.year() {
        let diff = now.ordinal() as i64 - at.ordinal() as i64;
        if diff == 0 {
            return "今天".to_string();
        }
        if diff == 1 {
            return "昨天".to_string();
        }
        return format!("{}月{}日", at.month(), at.day());
    }
    format!("{}年{}月{}日", at.year(), at.month(), at.day())
}

fn time_label(at_ms: i64) -> String {
    match to_local(at_ms) {
        Some(dt) => format!("{:02}:{:02}", dt.hour(), dt.minute()),
        None => String::new(),
    }
}

fn to_local(ms: i64) -> Option<chrono::DateTime<Local>> {
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 构造本地时间戳（测试不依赖机器时区）
    fn local_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("合法本地时间")
            .timestamp_millis()
    }

    // ---------------------------------------------------------------------
    // 「还没备份过」的三种入口
    // ---------------------------------------------------------------------

    #[test]
    fn never_when_no_timestamp() {
        assert_eq!(describe(None, 5, local_ms(2026, 9, 15, 12, 0)), NEVER);
    }

    #[test]
    fn never_when_timestamp_not_positive() {
        let now = local_ms(2026, 9, 15, 12, 0);
        assert_eq!(describe(Some(0), 5, now), NEVER);
        assert_eq!(describe(Some(-1), 5, now), NEVER);
    }

    /// 🔴 次数为 0 → 同一句「还没有备份过」（不出现「累计 0 次」）
    #[test]
    fn never_when_count_not_positive() {
        let now = local_ms(2026, 9, 15, 12, 0);
        let at = local_ms(2026, 9, 14, 10, 0);
        assert_eq!(describe(Some(at), 0, now), NEVER);
        assert_eq!(describe(Some(at), -3, now), NEVER);
    }

    // ---------------------------------------------------------------------
    // 日期标签
    // ---------------------------------------------------------------------

    #[test]
    fn label_today() {
        let now = local_ms(2026, 9, 15, 20, 0);
        let at = local_ms(2026, 9, 15, 14, 30);
        assert_eq!(describe(Some(at), 3, now), "上次备份：今天 14:30 · 累计 3 次");
    }

    #[test]
    fn label_yesterday() {
        let now = local_ms(2026, 9, 15, 9, 0);
        let at = local_ms(2026, 9, 14, 23, 5);
        assert_eq!(describe(Some(at), 1, now), "上次备份：昨天 23:05 · 累计 1 次");
    }

    /// 同年更早 → `M月d日`（**不补零**，与 Kotlin `M月d日` 一致）
    #[test]
    fn label_same_year_earlier_month_no_padding() {
        let now = local_ms(2026, 9, 15, 12, 0);
        let at = local_ms(2026, 1, 5, 8, 7);
        assert_eq!(describe(Some(at), 2, now), "上次备份：1月5日 08:07 · 累计 2 次");
    }

    /// 跨年 → 绝对日期（含年）
    #[test]
    fn label_previous_year() {
        let now = local_ms(2026, 9, 15, 12, 0);
        let at = local_ms(2025, 12, 31, 23, 59);
        assert_eq!(
            describe(Some(at), 9, now),
            "上次备份：2025年12月31日 23:59 · 累计 9 次"
        );
    }

    /// 同年但「未来时间」（时钟回拨）→ 差值为负，走绝对日期（不误判成"今天"）
    #[test]
    fn future_timestamp_same_year_uses_absolute_date() {
        let now = local_ms(2026, 9, 15, 12, 0);
        let at = local_ms(2026, 10, 1, 12, 0);
        assert_eq!(describe(Some(at), 1, now), "上次备份：10月1日 12:00 · 累计 1 次");
    }

    /// 跨年时**先比年**，不依赖 day-of-year 差值（1月1日 vs 去年12月31日）
    #[test]
    fn year_boundary_is_not_today() {
        let now = local_ms(2026, 1, 1, 0, 30);
        let at = local_ms(2025, 12, 31, 23, 30);
        let s = describe(Some(at), 1, now);
        assert!(s.contains("2025年12月31日"), "跨年必须给绝对日期：{s}");
        assert!(!s.contains("今天") && !s.contains("昨天"), "{s}");
    }

    /// 12 小时内跨日（昨天 23:00 → 今天 01:00）仍算「昨天」
    #[test]
    fn late_night_yesterday() {
        let now = local_ms(2026, 9, 15, 1, 0);
        let at = local_ms(2026, 9, 14, 23, 0);
        assert_eq!(describe(Some(at), 1, now), "上次备份：昨天 23:00 · 累计 1 次");
    }

    #[test]
    fn never_constant_text() {
        assert_eq!(NEVER, "还没有备份过");
    }
}

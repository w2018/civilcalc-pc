//! 备份包文件名：`civilcalc_backup_yyyyMMdd_HHmmss.tar.gz`
//!
//! 源：`civilcalc-android-v2/core/backup/BackupFileName.kt`（63 行）
//!
//! ## 为什么用**本地时间**命名
//!
//! 用户看文件名就知道是什么时候备的。列表排序以时间戳为准，
//! 服务端返回的修改时间不可靠时用文件名兜底。
//!
//! ⚠️ 本地时区意味着同一时刻在不同时区生成的名字不同 —— 这是**有意**的
//! （用户视角优先）。但**解析**也必须用本地时区，否则往返会对不上。

use chrono::{Datelike, Local, NaiveDateTime, Timelike};

/// 备份包文件名前缀。
pub const PREFIX: &str = "civilcalc_backup_";
/// 备份包文件名后缀。
pub const SUFFIX: &str = ".tar.gz";

/// 生成备份包文件名（本地时间）。
#[must_use]
pub fn create(timestamp_ms: i64) -> String {
    let Some(dt) = to_local(timestamp_ms) else {
        return format!("{PREFIX}{timestamp_ms}{SUFFIX}");
    };
    format!(
        "{PREFIX}{:04}{:02}{:02}_{:02}{:02}{:02}{SUFFIX}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// 是否是本应用产生的备份包。
///
/// 远端目录里可能有别的文件，**不认识的不要列进来** —— 所以长度也必须大于
/// 「前缀 + 后缀」，挡住 `civilcalc_backup_.tar.gz` 这种空壳。
#[must_use]
pub fn is_backup(name: &str) -> bool {
    name.starts_with(PREFIX)
        && name.ends_with(SUFFIX)
        && name.len() > PREFIX.len() + SUFFIX.len()
}

/// 从文件名解析时间戳；不是备份包或格式不对返回 `None`。
///
/// ⚠️ 解析必须**严格**（源用 `isLenient = false`）：
/// `20261332_990000`（13 月 32 日）这种要判为非法，而不是被"顺延"成别的日期。
#[must_use]
pub fn timestamp_of(name: &str) -> Option<i64> {
    if !is_backup(name) {
        return None;
    }
    let stamp = name
        .strip_prefix(PREFIX)?
        .strip_suffix(SUFFIX)?;
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S").ok()?;
    naive
        .and_local_timezone(Local)
        .single()
        .map(|dt| dt.timestamp_millis())
}

/// 展示用：`2026-09-15 14:30`（本地时间）。
#[must_use]
pub fn display_time(timestamp_ms: i64) -> String {
    match to_local(timestamp_ms) {
        Some(dt) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute()
        ),
        None => String::new(),
    }
}

/// 目标位置已有同名文件时追加 `(2)`、`(3)`…（最多 500 次后退化为时间戳后缀）。
///
/// `.tar.gz` **当成整体扩展名**处理，得到 `xxx(2).tar.gz` 而不是 `xxx.tar(2).gz`。
///
/// 纯函数（`existing` 由调用方查出来）便于单测。
#[must_use]
pub fn unique_among(file_name: &str, existing: &[String], suffix_fallback: i64) -> String {
    if !existing.iter().any(|e| e.as_str() == file_name) {
        return file_name.to_string();
    }
    // 扩展名判定：优先认整体 `.tar.gz`，否则取最后一个点号之后
    let ext: String = if file_name.ends_with(SUFFIX) {
        SUFFIX.to_string()
    } else {
        match file_name.rsplit_once('.') {
            Some((_, e)) if !e.is_empty() => format!(".{e}"),
            _ => String::new(),
        }
    };
    let base: &str = if ext.is_empty() {
        file_name
    } else {
        &file_name[..file_name.len() - ext.len()]
    };

    let mut index = 2;
    while index <= 500 {
        let candidate = format!("{base}({index}){ext}");
        if !existing.iter().any(|e| e.as_str() == candidate) {
            return candidate;
        }
        index += 1;
    }
    format!("{base}({suffix_fallback}){ext}")
}

/// 毫秒时间戳 → 本地时间。
fn to_local(ms: i64) -> Option<chrono::DateTime<Local>> {
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.with_timezone(&Local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 构造一个本地时间的时间戳（测试不依赖机器时区）
    fn local_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, s)
            .single()
            .expect("合法本地时间")
            .timestamp_millis()
    }

    // ---------------------------------------------------------------------
    // create / timestamp_of
    // ---------------------------------------------------------------------

    /// 文件名形状：`civilcalc_backup_yyyyMMdd_HHmmss.tar.gz`
    #[test]
    fn create_shape() {
        let ms = local_ms(2026, 9, 15, 14, 30, 5);
        assert_eq!(create(ms), "civilcalc_backup_20260915_143005.tar.gz");
    }

    /// 个位数月/日/时/分/秒都要补零
    #[test]
    fn create_zero_pads() {
        let ms = local_ms(2026, 1, 2, 3, 4, 5);
        assert_eq!(create(ms), "civilcalc_backup_20260102_030405.tar.gz");
    }

    /// 🔴 往返一致（本地时区两端一致，所以与机器时区无关）
    #[test]
    fn create_then_parse_roundtrips() {
        let ms = local_ms(2026, 9, 15, 14, 30, 5);
        let name = create(ms);
        assert_eq!(timestamp_of(&name), Some(ms));
    }

    /// 闰日也能往返
    #[test]
    fn leap_day_roundtrips() {
        let ms = local_ms(2028, 2, 29, 23, 59, 59);
        assert_eq!(timestamp_of(&create(ms)), Some(ms));
    }

    #[test]
    fn parse_rejects_non_backup_names() {
        assert_eq!(timestamp_of("random.txt"), None);
        assert_eq!(timestamp_of("civilcalc_backup_.tar.gz"), None, "空壳");
        assert_eq!(timestamp_of("civilcalc_backup_20260915.tar.gz"), None, "缺时分秒");
        assert_eq!(timestamp_of("backup_20260915_143005.tar.gz"), None, "前缀不符");
        assert_eq!(timestamp_of("civilcalc_backup_20260915_143005.zip"), None, "后缀不符");
    }

    /// 🔴 严格解析：非法日期/时间不能"顺延"
    #[test]
    fn parse_is_strict() {
        assert_eq!(timestamp_of("civilcalc_backup_20261332_990000.tar.gz"), None, "13月32日99时");
        assert_eq!(timestamp_of("civilcalc_backup_20260915_250000.tar.gz"), None, "25 时");
        assert_eq!(timestamp_of("civilcalc_backup_20260230_120000.tar.gz"), None, "2月30日");
        assert_eq!(timestamp_of("civilcalc_backup_20260915_1430.tar.gz"), None, "少两位");
    }

    // ---------------------------------------------------------------------
    // is_backup
    // ---------------------------------------------------------------------

    #[test]
    fn is_backup_accepts_valid() {
        assert!(is_backup("civilcalc_backup_20260915_143005.tar.gz"));
    }

    #[test]
    fn is_backup_rejects_others() {
        assert!(!is_backup("civilcalc_backup_.tar.gz"), "前后缀之间必须有内容");
        assert!(!is_backup("civilcalc_backup_20260915_143005.tar"));
        assert!(!is_backup("notes.txt"));
        assert!(!is_backup(""));
        assert!(!is_backup(".tar.gz"));
    }

    // ---------------------------------------------------------------------
    // display_time
    // ---------------------------------------------------------------------

    #[test]
    fn display_time_shape() {
        let ms = local_ms(2026, 9, 15, 14, 30, 0);
        assert_eq!(display_time(ms), "2026-09-15 14:30");
    }

    #[test]
    fn display_time_zero_pads() {
        let ms = local_ms(2026, 1, 2, 3, 4, 0);
        assert_eq!(display_time(ms), "2026-01-02 03:04");
    }

    // ---------------------------------------------------------------------
    // unique_among
    // ---------------------------------------------------------------------

    #[test]
    fn unique_returns_same_when_absent() {
        let existing = vec!["a.tar.gz".to_string()];
        assert_eq!(unique_among("b.tar.gz", &existing, 1), "b.tar.gz");
    }

    /// 🔴 `.tar.gz` 当整体扩展名：`xxx(2).tar.gz`，不是 `xxx.tar(2).gz`
    #[test]
    fn unique_appends_before_tar_gz() {
        let existing = vec!["civilcalc_backup_20260915_143005.tar.gz".to_string()];
        assert_eq!(
            unique_among("civilcalc_backup_20260915_143005.tar.gz", &existing, 1),
            "civilcalc_backup_20260915_143005(2).tar.gz"
        );
    }

    #[test]
    fn unique_increments_past_existing() {
        let existing = vec![
            "a.tar.gz".to_string(),
            "a(2).tar.gz".to_string(),
            "a(3).tar.gz".to_string(),
        ];
        assert_eq!(unique_among("a.tar.gz", &existing, 1), "a(4).tar.gz");
    }

    #[test]
    fn unique_single_extension() {
        let existing = vec!["photo.jpg".to_string()];
        assert_eq!(unique_among("photo.jpg", &existing, 1), "photo(2).jpg");
    }

    /// 没有扩展名时后缀直接接在末尾
    #[test]
    fn unique_without_extension() {
        let existing = vec!["readme".to_string()];
        assert_eq!(unique_among("readme", &existing, 1), "readme(2)");
    }

    /// 隐藏文件（以点开头）被当作「无扩展名」——与源 `substringAfterLast('.')` 的行为一致
    #[test]
    fn unique_dotfile_has_no_extension() {
        let existing = vec![".gitignore".to_string()];
        // `substringAfterLast('.')` = "gitignore" → ext = ".gitignore" → base = ""
        assert_eq!(unique_among(".gitignore", &existing, 1), "(2).gitignore");
    }

    /// 500 次都撞车 → 退化为时间戳后缀
    #[test]
    fn unique_falls_back_after_500() {
        let mut existing = vec!["a.tar.gz".to_string()];
        for i in 2..=500 {
            existing.push(format!("a({i}).tar.gz"));
        }
        assert_eq!(unique_among("a.tar.gz", &existing, 12345), "a(12345).tar.gz");
    }

    /// 候选位置被占但未满 500 → 仍走递增（不提前退化）
    #[test]
    fn unique_uses_timestamp_only_when_exhausted() {
        let existing = vec!["a.tar.gz".to_string(), "a(2).tar.gz".to_string()];
        assert_eq!(unique_among("a.tar.gz", &existing, 999), "a(3).tar.gz");
    }
}

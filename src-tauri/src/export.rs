//! 导出落盘的安全与命名（PC 端**安全关键路径**）。
//!
//! 源：`app/src/main/kotlin/com/w2018/civilcalc/update/DocxExporter.kt`
//! （`sanitizeFileName` / `uniqueFileNameIn` / `saveToDownloads`）
//!
//! ## 与源项目的差异（PC 端必须补的）
//!
//! Android 侧的文件名清理只做了两件事：
//!
//! ```kotlin
//! name.replace(Regex("[\\\\/:*?\"<>|]"), "_").replace("..", "").trim()
//! ```
//!
//! 因为 Android 的下载目录对文件名宽容得多。**Windows 不行**，必须补三类：
//!
//! | 补什么 | 为什么 |
//! |---|---|
//! | **保留设备名** `CON`/`PRN`/`AUX`/`NUL`/`COM1-9`/`LPT1-9`（含 `CONIN$`/`CONOUT$`） | `CON.docx` 在 Windows 上**根本创建不了**，且报错信息晦涩 |
//! | **尾部点号与空格** | Windows 会**静默剥离**：写 `报告.docx.` 实际得到 `报告.docx`，用户看到的与预期不符 |
//! | **长度上限** | 单个文件名组件超过 255 字符会失败 |
//!
//! 另加一道**路径越界闸门** [`ensure_within`]：即便文件名被清理过，
//! 也绝不允许最终路径逃出指定目录（防 `..`、防符号链接、防 NTFS 数据流）。
//!
//! ## ⚠️ 本模块是安全边界，改动前先看测试
//!
//! 这里的每个分支都有对应单测；`cargo test -p civilcalc-pc` 会拦住回退。

use civilcalc_core::CoreError;
use std::path::{Path, PathBuf};

/// Windows 保留设备名（**不含扩展名判断**，只看第一个点号之前的部分）。
///
/// 来源：Microsoft 文档 "Naming Files, Paths, and Namespaces"。
/// 注意 `COM0`/`LPT0` **不在**保留列表里（只有 1~9）。
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Windows 文件名非法字符（与源项目正则 `[\\/:*?"<>|]` 一致）
const ILLEGAL_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// 单个文件名组件的长度上限（留出余量，Windows 组件上限 255）
const MAX_NAME_LEN: usize = 200;

/// 清理失败且原名为空时的兜底名
const FALLBACK_NAME: &str = "未命名";

/// 清理文件名（PC 版，源项目规则 + 三类 Windows 修正）。
///
/// 处理顺序（顺序有讲究）：
///
/// 1. 非法字符与 ASCII 控制字符 → `_`
/// 2. 删掉所有 `..`（对齐源项目 `.replace("..", "")`）
/// 3. 去首尾空白
/// 4. **去尾部点号与空格**（Windows 会静默剥离，不如自己先去掉）
/// 5. 第一个点号之前是**保留设备名** → 前缀 `_`
/// 6. 长度超限 → 截断（保留扩展名）
/// 7. 结果为空 → [`FALLBACK_NAME`]
///
/// ```
/// # use civilcalc_pc_lib::export::sanitize_file_name;
/// assert_eq!(sanitize_file_name(r#"a/b:c*d"#), "a_b_c_d");
/// assert_eq!(sanitize_file_name("a..b"), "ab");
/// assert_eq!(sanitize_file_name("报告.docx."), "报告.docx");
/// assert_eq!(sanitize_file_name("CON.docx"), "_CON.docx");
/// // 分隔符先变 `_`、再删 `..`，故残留前导下划线
/// assert_eq!(sanitize_file_name("../../etc/passwd"), "__etc_passwd");
/// ```
pub fn sanitize_file_name(raw: &str) -> String {
    // 1. 非法字符 / 控制字符 → _
    let mut s: String = raw
        .chars()
        .map(|c| {
            if ILLEGAL_CHARS.contains(&c) || (c as u32) < 0x20 || c == '\u{7f}' {
                '_'
            } else {
                c
            }
        })
        .collect();

    // 2. 删掉所有 ".."（循环直到不再出现，防 "...." → ".."）
    while s.contains("..") {
        s = s.replace("..", "");
    }

    // 3. 去首尾空白
    s = s.trim().to_string();

    // 4. 去尾部点号与空格（Windows 会静默剥离，不如自己先去掉）
    s = s.trim_end_matches(['.', ' ', '\t']).to_string();

    // 5. 保留设备名：只看第一个点号之前的部分（Windows 的判定方式）
    let head = s.split('.').next().unwrap_or("");
    if WINDOWS_RESERVED_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(head))
    {
        s = format!("_{s}");
    }

    // 6. 长度上限：截断主体、保留扩展名
    if s.chars().count() > MAX_NAME_LEN {
        s = truncate_keeping_extension(&s, MAX_NAME_LEN);
    }

    // 7. 兜底
    if s.is_empty() {
        return FALLBACK_NAME.to_string();
    }
    s
}

/// 截断到 `max` 个字符，尽量保留扩展名
fn truncate_keeping_extension(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    // 扩展名（最后一个点号之后，且长度合理）
    let ext: String = match s.rfind('.') {
        Some(idx) if idx + 1 < s.len() => s[idx..].chars().take(16).collect(),
        _ => String::new(),
    };
    let keep = max.saturating_sub(ext.chars().count());
    let mut out: String = chars.into_iter().take(keep).collect();
    out = out.trim_end_matches(['.', ' ']).to_string();
    out.push_str(&ext);
    out
}

/// 目录内取一个不冲突的文件名：已存在则追加 `(2)`、`(3)`…，最多到 `(500)`。
///
/// 1:1 对齐源项目 `uniqueFileNameIn`：`(500)` 之后用毫秒时间戳兜底
/// （几乎不可能走到，但源项目有这个分支，保留以行为一致）。
pub fn unique_file_name_in(dir: &Path, file_name: &str) -> String {
    if !dir.join(file_name).exists() {
        return file_name.to_string();
    }

    let (base, ext) = split_name(file_name);
    for i in 2..=500u32 {
        let candidate = if ext.is_empty() {
            format!("{base}({i})")
        } else {
            format!("{base}({i}).{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    let stamp = civilcalc_core::now_ms();
    if ext.is_empty() {
        format!("{base}({stamp})")
    } else {
        format!("{base}({stamp}).{ext}")
    }
}

/// 拆成 (主体, 扩展名)。对齐源项目 `substringBeforeLast('.')` / `substringAfterLast('.')`。
///
/// - `"a.docx"` → `("a", "docx")`
/// - `"a.b.docx"` → `("a.b", "docx")`
/// - `"a"` → `("a", "")`
/// - `"a."` → `("a", "")`（源项目 `substringAfterLast('.', "")` 在此返回空串）
fn split_name(file_name: &str) -> (String, String) {
    match file_name.rfind('.') {
        // `idx` 必为合法字符边界，故 `[..idx]` / `[idx+1..]` 都安全
        Some(idx) => (
            file_name[..idx].to_string(),
            file_name[idx + 1..].to_string(),
        ),
        None => (file_name.to_string(), String::new()),
    }
}

/// **路径越界闸门**：确保 `target` 落在 `base` 之内，否则返回
/// [`CoreError::PathTraversal`]。
///
/// ## 为什么光清理文件名不够
///
/// 清理只作用于**单个文件名组件**。若上层把用户输入拼进路径（如
/// `dir.join(user_input)`），`user_input` 里仍可能带路径分隔符或 `..`
/// —— 清理函数会把它们换成 `_`，但**调用方可能忘了调用**。
/// 这道闸门是兜底：不管路径怎么来的，最终都要过它。
///
/// ## 实现要点
///
/// - 用 `canonicalize` 解析符号链接与 `..`（纯字符串比较会被绕过）
/// - `target` 可能**尚不存在**（即将创建），所以向上找到最近的已存在祖先再判定
/// - 判定用 [`Path::starts_with`]（按路径组件比较，不是字符串前缀 ——
///   `C:\a\bc` 不会被 `C:\a\b` 误判为在内）
pub fn ensure_within(base: &Path, target: &Path) -> Result<PathBuf, CoreError> {
    let base_canon = std::fs::canonicalize(base).map_err(|e| CoreError::Storage {
        message: format!("基准目录不可用（{}）: {e}", base.display()),
    })?;

    // 向上找最近的已存在祖先
    let mut probe = target.to_path_buf();
    loop {
        if probe.exists() {
            break;
        }
        match probe.parent() {
            Some(p) if !p.as_os_str().is_empty() => probe = p.to_path_buf(),
            _ => break,
        }
    }

    let probe_canon = std::fs::canonicalize(&probe).map_err(|e| CoreError::Storage {
        message: format!("目标路径不可用（{}）: {e}", probe.display()),
    })?;

    if !probe_canon.starts_with(&base_canon) {
        return Err(CoreError::PathTraversal {
            message: format!(
                "拒绝越界写入：{} 不在 {} 之内",
                target.display(),
                base.display()
            ),
        });
    }

    Ok(target.to_path_buf())
}

/// 在 `dir` 内生成一个**安全且不冲突**的落盘路径。
///
/// = [`sanitize_file_name`] → [`unique_file_name_in`] → [`ensure_within`]。
///
/// 这是命令层应当直接调用的入口：**不要自己拼路径**。
pub fn safe_output_path(dir: &Path, raw_name: &str) -> Result<PathBuf, CoreError> {
    let clean = sanitize_file_name(raw_name);
    let unique = unique_file_name_in(dir, &clean);
    ensure_within(dir, &dir.join(&unique))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("civilcalc-export-test-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    // ---------------- 非法字符（对齐源项目正则） ----------------

    #[test]
    fn illegal_chars_become_underscore() {
        // 源项目正则 [\\/:*?"<>|] 的 9 个字符
        assert_eq!(sanitize_file_name(r#"a\b"#), "a_b");
        assert_eq!(sanitize_file_name("a/b"), "a_b");
        assert_eq!(sanitize_file_name("a:b"), "a_b");
        assert_eq!(sanitize_file_name("a*b"), "a_b");
        assert_eq!(sanitize_file_name("a?b"), "a_b");
        assert_eq!(sanitize_file_name(r#"a"b"#), "a_b");
        assert_eq!(sanitize_file_name("a<b"), "a_b");
        assert_eq!(sanitize_file_name("a>b"), "a_b");
        assert_eq!(sanitize_file_name("a|b"), "a_b");
    }

    #[test]
    fn control_chars_become_underscore() {
        assert_eq!(sanitize_file_name("a\nb"), "a_b");
        assert_eq!(sanitize_file_name("a\tb"), "a_b");
        assert_eq!(sanitize_file_name("a\u{0}b"), "a_b");
    }

    #[test]
    fn ntfs_data_stream_colon_is_neutralized() {
        // "file.txt:evil" 是 NTFS 数据流写法
        assert_eq!(sanitize_file_name("file.txt:evil"), "file.txt_evil");
    }

    // ---------------- `..` 与路径穿越 ----------------

    #[test]
    fn dotdot_is_removed() {
        assert_eq!(sanitize_file_name("a..b"), "ab");
        // 连续 4 个点 → 一轮替换掉两处 ".."，结果为空 → 兜底
        assert_eq!(sanitize_file_name("...."), FALLBACK_NAME);
        assert_eq!(sanitize_file_name("....docx"), "docx");
        // 3 个点：替换后剩一个点，又被"去尾部点号"清掉
        assert_eq!(sanitize_file_name("..."), FALLBACK_NAME);
    }

    #[test]
    fn path_traversal_attempts_are_defanged() {
        // ⚠️ 注意实际结果：分隔符先变 `_`，再删 `..`，于是残留前导下划线
        assert_eq!(sanitize_file_name("../../etc/passwd"), "__etc_passwd");
        assert_eq!(
            sanitize_file_name(r"..\..\windows\system32"),
            "__windows_system32"
        );
        // 关键性质：不再含 `..`，也不含路径分隔符
        for evil in ["../../x", r"..\..\y", "a/../../b"] {
            let out = sanitize_file_name(evil);
            assert!(!out.contains(".."), "{evil} → {out} 仍含 ..");
            assert!(!out.contains('/'), "{evil} → {out} 仍含 /");
            assert!(!out.contains('\\'), "{evil} → {out} 仍含 \\");
        }
    }

    // ---------------- 尾部点号 / 空格（PC 新增） ----------------

    #[test]
    fn trailing_dots_and_spaces_are_stripped() {
        assert_eq!(sanitize_file_name("报告.docx."), "报告.docx");
        assert_eq!(sanitize_file_name("报告.docx  "), "报告.docx");
        assert_eq!(sanitize_file_name("报告.docx . "), "报告.docx");
        assert_eq!(sanitize_file_name(" 报告.docx"), "报告.docx");
    }

    // ---------------- 保留设备名（PC 新增） ----------------

    #[test]
    fn reserved_device_names_get_prefixed() {
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("CON.docx"), "_CON.docx");
        assert_eq!(sanitize_file_name("PRN.txt"), "_PRN.txt");
        assert_eq!(sanitize_file_name("AUX"), "_AUX");
        assert_eq!(sanitize_file_name("NUL"), "_NUL");
        assert_eq!(sanitize_file_name("COM1"), "_COM1");
        assert_eq!(sanitize_file_name("LPT9.docx"), "_LPT9.docx");
    }

    #[test]
    fn reserved_check_is_case_insensitive() {
        assert_eq!(sanitize_file_name("con.docx"), "_con.docx");
        assert_eq!(sanitize_file_name("Con.DOCX"), "_Con.DOCX");
    }

    /// Windows 的判定是"第一个点号之前"：`CON.a.b` 也是保留名
    #[test]
    fn reserved_check_uses_first_dot_segment() {
        assert_eq!(sanitize_file_name("CON.a.b"), "_CON.a.b");
        assert_eq!(sanitize_file_name("NUL.x.txt"), "_NUL.x.txt");
    }

    #[test]
    fn non_reserved_similar_names_are_untouched() {
        // COM0 / LPT0 不在保留列表；CONSOLE 也不是
        assert_eq!(sanitize_file_name("COM0.docx"), "COM0.docx");
        assert_eq!(sanitize_file_name("LPT0.docx"), "LPT0.docx");
        assert_eq!(sanitize_file_name("CONSOLE.docx"), "CONSOLE.docx");
        assert_eq!(sanitize_file_name("MYCON.docx"), "MYCON.docx");
    }

    // ---------------- 长度与兜底 ----------------

    #[test]
    fn overlong_name_is_truncated_keeping_extension() {
        let long = "甲".repeat(400);
        let out = sanitize_file_name(&format!("{long}.docx"));
        assert!(out.chars().count() <= MAX_NAME_LEN, "应被截断");
        assert!(out.ends_with(".docx"), "应保留扩展名: {out}");
    }

    #[test]
    fn empty_result_falls_back() {
        assert_eq!(sanitize_file_name(""), FALLBACK_NAME);
        assert_eq!(sanitize_file_name("   "), FALLBACK_NAME);
        assert_eq!(sanitize_file_name(".."), FALLBACK_NAME);
        assert_eq!(sanitize_file_name("..."), FALLBACK_NAME);
    }

    #[test]
    fn normal_names_pass_through() {
        for n in [
            "梁正截面受弯计算_20260917_120000_000.docx",
            "report.xlsx",
            "a b c.txt",
            "计算结果(2).docx",
        ] {
            assert_eq!(sanitize_file_name(n), n, "{n} 不应被改动");
        }
    }

    // ---------------- 同名加序号（对齐源项目 uniqueFileNameIn） ----------------

    #[test]
    fn unique_name_when_no_conflict() {
        let d = tmp_dir("unique-none");
        assert_eq!(unique_file_name_in(&d, "a.docx"), "a.docx");
    }

    #[test]
    fn unique_name_appends_counter() {
        let d = tmp_dir("unique-counter");
        fs::write(d.join("a.docx"), b"x").unwrap();
        assert_eq!(unique_file_name_in(&d, "a.docx"), "a(2).docx");

        fs::write(d.join("a(2).docx"), b"x").unwrap();
        assert_eq!(unique_file_name_in(&d, "a.docx"), "a(3).docx");
    }

    #[test]
    fn unique_name_handles_no_extension() {
        let d = tmp_dir("unique-noext");
        fs::write(d.join("README"), b"x").unwrap();
        assert_eq!(unique_file_name_in(&d, "README"), "README(2)");
    }

    #[test]
    fn unique_name_handles_multi_dot() {
        let d = tmp_dir("unique-multidot");
        fs::write(d.join("a.b.docx"), b"x").unwrap();
        // 主体按最后一个点号切分
        assert_eq!(unique_file_name_in(&d, "a.b.docx"), "a.b(2).docx");
    }

    // ---------------- 路径越界闸门 ----------------

    #[test]
    fn ensure_within_accepts_child() {
        let d = tmp_dir("within-ok");
        let target = d.join("sub").join("f.docx");
        assert!(ensure_within(&d, &target).is_ok());
    }

    #[test]
    fn ensure_within_accepts_immediate_child_that_exists() {
        let d = tmp_dir("within-exist");
        let f = d.join("f.docx");
        fs::write(&f, b"x").unwrap();
        assert!(ensure_within(&d, &f).is_ok());
    }

    #[test]
    fn ensure_within_rejects_parent_escape() {
        let d = tmp_dir("within-escape");
        let target = d.join("..").join("evil.docx");
        let err = ensure_within(&d, &target).unwrap_err();
        assert_eq!(err.code(), "PATH_TRAVERSAL");
    }

    #[test]
    fn ensure_within_rejects_absolute_escape() {
        let d = tmp_dir("within-abs");
        let outside = std::env::temp_dir().join("civilcalc-export-escape-probe.docx");
        let err = ensure_within(&d, &outside).unwrap_err();
        assert_eq!(err.code(), "PATH_TRAVERSAL");
    }

    /// `starts_with` 按**路径组件**比较：`/a/bc` 不在 `/a/b` 之内
    #[test]
    fn ensure_within_is_component_wise_not_string_prefix() {
        let d = tmp_dir("within-prefix");
        let sibling = d
            .parent()
            .unwrap()
            .join(format!("{}-sibling", d.file_name().unwrap().to_string_lossy()));
        fs::create_dir_all(&sibling).unwrap();

        let err = ensure_within(&d, &sibling.join("f.docx")).unwrap_err();
        assert_eq!(err.code(), "PATH_TRAVERSAL", "同前缀的兄弟目录不算在内");
    }

    // ---------------- 组合入口 ----------------

    #[test]
    fn safe_output_path_end_to_end() {
        let d = tmp_dir("safe-e2e");
        // 恶意输入：穿越 + 保留名 + 非法字符 + 尾部点号
        let p = safe_output_path(&d, "../../CON:evil.docx.").unwrap();

        assert_eq!(p.parent().unwrap(), d, "必须落在目标目录内");
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with('_'), "保留名应被加前缀: {name}");
        assert!(!name.contains('/') && !name.contains('\\'));
        assert!(!name.contains(':'));
        assert!(!name.ends_with('.'), "尾部点号应被去掉: {name}");
    }

    #[test]
    fn safe_output_path_avoids_overwrite() {
        let d = tmp_dir("safe-overwrite");
        let first = safe_output_path(&d, "报告.docx").unwrap();
        fs::write(&first, b"x").unwrap();

        let second = safe_output_path(&d, "报告.docx").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "报告(2).docx"
        );
    }

    #[test]
    fn safe_output_path_rejects_blank_name_via_fallback() {
        let d = tmp_dir("safe-blank");
        let p = safe_output_path(&d, "   ").unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), FALLBACK_NAME);
    }
}

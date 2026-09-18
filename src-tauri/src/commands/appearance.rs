//! 组 14a：外观与背景图（5 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `appearance_get` | — | [`Appearance`] |
//! | `appearance_save` | `appearance` | `void` |
//! | `background_get` | — | [`BackgroundInfo`] |
//! | `background_save` | `path`, `transparency` | `void` |
//! | `background_clear` | — | `void` |
//!
//! ## 背景图存哪
//!
//! 图片本体**复制**到 `app_data_dir/background.jpg`（固定文件名），
//! 偏好里只存标记 `background_image = "1"` 与透明度。
//!
//! 为什么不存原始路径：用户从"下载"或"桌面"选图后，原文件随时可能被移走/删除；
//! 复制进应用目录才能保证下次启动还在。
//!
//! ## `background_clear` 会**一并重置**文字色与透明度
//!
//! 源项目行为：清除背景后文字色/透明度失去意义，留着会让下次设背景时
//! 突然"变样"。见 [`background_clear`] 的实现。
//!
//! ## ⚠️ 本模块**不删** `app_data_dir` 下的任何图片以外的东西
//!
//! 清除背景只删 `background.jpg` 这一个文件（继承源项目红线：
//! 绝不递归删数据目录）。

use crate::config::{TEXT_COLOR_AUTO, DEFAULT_BG_TRANSPARENCY};
use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 主题模式（与 `config::ThemeMode` 的线上表示一致）
pub type ThemeModeName = String;

/// 外观聚合视图（主题 + 背景 + 文字色 + 页面底色）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    /// `SYSTEM` / `LIGHT` / `DARK`
    pub theme_mode: ThemeModeName,
    pub has_background: bool,
    /// 0~100
    pub background_transparency: i64,
    /// `AUTO` 或具体色值（如 `#FF0000`）
    pub text_color: String,
    /// 自定义页面底色（`#RRGGBB`）；`None` = 用主题默认底色
    ///
    /// `#[serde(default)]`：这个字段是后加的，老前端 / 老备份包里没有它，
    /// 缺省必须是「未设置」而不是反序列化失败。
    #[serde(default)]
    pub background_color: Option<String>,
}

/// 背景信息
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundInfo {
    pub has_background: bool,
    /// 0~100
    pub background_transparency: i64,
    /// 存储的文字色（可能是 `AUTO`）。**实际生效色由主题决定，前端自行解析**
    pub text_color: String,
    /// 背景图文件绝对路径（未设置时为 `null`）
    pub image_path: Option<String>,
}

/// 读外观聚合视图
#[tauri::command]
pub fn appearance_get(state: State<'_, AppState>) -> CmdResult<Appearance> {
    state.with_config(|c| Appearance {
        theme_mode: c.theme_mode().as_str().to_string(),
        has_background: c.has_background_image(),
        background_transparency: c.background_transparency(),
        text_color: c.text_color().to_string(),
        background_color: c.background_color().map(str::to_string),
    })
}

/// 保存外观（主题 + 背景开关 + 透明度 + 文字色 + 页面底色）。
///
/// 只写传进来的字段 —— 但因为是**整体覆盖**，前端应当先 `appearance_get`。
/// `background_transparency` 会被夹到 `0..=100`。
#[tauri::command]
pub fn appearance_save(state: State<'_, AppState>, appearance: Appearance) -> CmdResult<()> {
    state.update_config(move |c| {
        c.set_str(crate::config::KEY_THEME, &appearance.theme_mode);
        c.set_has_background_image(appearance.has_background);
        c.set_background_transparency(appearance.background_transparency);
        c.set_text_color(appearance.text_color);
        c.set_background_color(appearance.background_color.as_deref());
    })
}

/// 读背景信息
#[tauri::command]
pub fn background_get(state: State<'_, AppState>) -> CmdResult<BackgroundInfo> {
    let path = state.paths.background_path();
    let (has, transparency, text_color) = state.with_config(|c| {
        (
            c.has_background_image(),
            c.background_transparency(),
            c.text_color().to_string(),
        )
    })?;
    let exists = path.exists();

    Ok(BackgroundInfo {
        // 标记为真但文件丢了 → 报 false（避免前端去加载一个不存在的图）
        has_background: has && exists,
        background_transparency: transparency,
        text_color,
        image_path: if has && exists {
            Some(path.to_string_lossy().to_string())
        } else {
            None
        },
    })
}

/// 设置背景图：把 `path` 指向的图片**复制**到应用数据目录，并记录透明度。
///
/// - 源文件不存在 / 不是文件 → [`CommandError::NotFound`]
/// - 复制失败 → [`CommandError::Storage`]
/// - `transparency` 夹到 `0..=100`
#[tauri::command]
pub fn background_save(
    state: State<'_, AppState>,
    path: String,
    transparency: i64,
) -> CmdResult<()> {
    let src = std::path::Path::new(&path);
    if !src.is_file() {
        return Err(CommandError::NotFound {
            message: format!("背景图不存在或不是文件: {path}"),
        });
    }

    let dest = state.paths.background_path();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CommandError::Storage {
            message: format!("创建数据目录失败: {e}"),
        })?;
    }
    std::fs::copy(src, &dest).map_err(|e| CommandError::Storage {
        message: format!("复制背景图失败: {e}"),
    })?;

    state.update_config(move |c| {
        c.set_has_background_image(true);
        c.set_background_transparency(transparency);
    })?;

    civilcalc_core::log::i(
        "Commands",
        &format!("已设置背景图 → {}", dest.display()),
    );
    Ok(())
}

/// 清除背景图，并**一并重置**文字色与透明度。
///
/// ## 为什么连文字色一起重置
///
/// 源项目行为。理由：文字色/透明度是**为背景图服务**的 ——
/// 背景没了它们就失去意义，留着会让用户下次设背景时"莫名其妙变样"。
///
/// ⚠️ **只删 `background.jpg` 这一个文件**，不做任何递归删除。
#[tauri::command]
pub fn background_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let path = state.paths.background_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| CommandError::Storage {
            message: format!("删除背景图失败（{}）: {e}", path.display()),
        })?;
    }

    state.update_config(|c| {
        c.set_has_background_image(false);
        c.remove(crate::config::KEY_BG_TRANSPARENCY);
        c.set_text_color(TEXT_COLOR_AUTO);
    })?;

    civilcalc_core::log::i("Commands", "已清除背景图（含文字色与透明度重置）");
    Ok(())
}

/// 默认透明度（供前端初始化用；与后端常量一致）
#[tauri::command]
pub fn background_default_transparency() -> i64 {
    DEFAULT_BG_TRANSPARENCY
}

#[cfg(test)]
mod tests {
    use crate::config::{AppConfig, KEY_BG_TRANSPARENCY, KEY_TEXT_COLOR, KEY_THEME, THEME_DARK};

    /// `appearance_get` 的读取口径
    #[test]
    fn appearance_reads_config_defaults() {
        let c = AppConfig::in_memory();
        assert_eq!(c.theme_mode().as_str(), "SYSTEM");
        assert!(!c.has_background_image());
        assert_eq!(c.background_transparency(), 22);
        assert_eq!(c.text_color(), "AUTO");
        assert_eq!(c.background_color(), None, "未设置过页面底色");
    }

    /// `appearance_save` 的写入口径
    #[test]
    fn appearance_save_writes_all_fields() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_THEME, "DARK");
        c.set_has_background_image(true);
        c.set_background_transparency(70);
        c.set_text_color("#FF0000");
        c.set_background_color(Some("#123456"));

        assert_eq!(c.theme_mode().as_str(), "DARK");
        assert!(c.has_background_image());
        assert_eq!(c.background_transparency(), 70);
        assert_eq!(c.text_color(), "#FF0000");
        assert_eq!(c.background_color(), Some("#123456"));
    }

    /// 页面底色：`None` / 空白都表示「回到主题默认」，且**删键**而不是写默认值
    #[test]
    fn background_color_none_removes_key() {
        let mut c = AppConfig::in_memory();
        c.set_background_color(Some("#abcdef"));
        assert_eq!(c.background_color(), Some("#abcdef"));

        c.set_background_color(None);
        assert_eq!(c.background_color(), None);
        assert_eq!(c.get_str(crate::config::KEY_BG_COLOR), None, "键应被删除");

        // 空白串等同于 None（前端清空输入框时会传 ""）
        c.set_background_color(Some("   "));
        assert_eq!(c.background_color(), None);
    }

    /// 🔴 重置「外观」区块必须把页面底色一起清掉（需求 9）
    #[test]
    fn reset_appearance_clears_background_color() {
        use crate::config::ConfigSection;
        let mut c = AppConfig::in_memory();
        c.set_background_color(Some("#123456"));
        c.set_text_color("#00FF00");

        c.reset_section(ConfigSection::Appearance);

        assert_eq!(c.background_color(), None, "页面底色要跟着一起重置");
        assert_eq!(c.text_color(), "AUTO");
    }

    /// `background_clear` 一并重置文字色与透明度
    #[test]
    fn clear_resets_transparency_and_text_color() {
        let mut c = AppConfig::in_memory();
        c.set_has_background_image(true);
        c.set_background_transparency(88);
        c.set_text_color("#00FF00");

        // 命令层的重置动作
        c.set_has_background_image(false);
        c.remove(KEY_BG_TRANSPARENCY);
        c.set_text_color("AUTO");

        assert!(!c.has_background_image());
        assert_eq!(c.get_int(KEY_BG_TRANSPARENCY), None, "键应被删除而非写默认值");
        assert_eq!(c.background_transparency(), 22, "读取时回落默认");
        assert_eq!(c.text_color(), "AUTO");
        assert_eq!(c.get_str(KEY_TEXT_COLOR), Some("AUTO"));
    }

    /// 清除背景**不影响主题**
    #[test]
    fn clear_keeps_theme() {
        let mut c = AppConfig::in_memory();
        c.set_str(KEY_THEME, THEME_DARK);
        c.set_has_background_image(true);

        c.set_has_background_image(false);
        c.remove(KEY_BG_TRANSPARENCY);
        c.set_text_color("AUTO");

        assert_eq!(c.theme_mode().as_str(), "DARK", "主题不该被重置");
    }

    /// 透明度越界会被夹紧（命令层依赖这个行为）
    #[test]
    fn transparency_is_clamped_on_write() {
        let mut c = AppConfig::in_memory();
        c.set_background_transparency(500);
        assert_eq!(c.background_transparency(), 100);
        c.set_background_transparency(-10);
        assert_eq!(c.background_transparency(), 0);
    }

    /// `Appearance` / `BackgroundInfo` 的 serde 形状（前端契约）
    #[test]
    fn serde_is_camel_case() {
        let a = super::Appearance {
            theme_mode: "DARK".into(),
            has_background: true,
            background_transparency: 30,
            text_color: "AUTO".into(),
            background_color: Some("#123456".into()),
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["themeMode"], serde_json::json!("DARK"));
        assert_eq!(v["hasBackground"], serde_json::json!(true));
        assert_eq!(v["backgroundTransparency"], serde_json::json!(30));
        assert_eq!(v["textColor"], serde_json::json!("AUTO"));
        assert_eq!(v["backgroundColor"], serde_json::json!("#123456"));
        assert!(v.get("theme_mode").is_none(), "不得泄漏 snake_case");

        let b = super::BackgroundInfo {
            has_background: false,
            background_transparency: 22,
            text_color: "AUTO".into(),
            image_path: None,
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["imagePath"], serde_json::Value::Null);
    }

    /// 老前端 / 老备份包不带 `backgroundColor` → 必须读成「未设置」而不是报错
    #[test]
    fn appearance_background_color_is_optional() {
        let raw = r#"{"themeMode":"SYSTEM","hasBackground":false,"backgroundTransparency":22,"textColor":"AUTO"}"#;
        let a: super::Appearance = serde_json::from_str(raw).expect("缺字段也要能读");
        assert_eq!(a.background_color, None);
    }
}

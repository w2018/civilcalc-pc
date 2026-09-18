<div align="center">

# AI全能计算器

**面向工程计算的 AI 公式工作台**

把一句自然语言描述、或一张手写算式照片，变成可计算、可校验、可导出的工程公式。

[![License](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%7C%2011-0078D4.svg)](#安装)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org)
[![Vue](https://img.shields.io/badge/Vue-3-42B883.svg)](https://vuejs.org)

</div>

Windows 桌面应用，把「翻规范、找公式、按计算器、抄进 Excel」压成一条链路：
**输入 → AI 解析成结构化公式 → 实时求值 → 导出**。

内置 **55 条工程公式**，覆盖结构、地基基础、岩土基坑、荷载、施工工程量、测量、
造价、数学与几何，每条标注规范依据（GB 50010 / GB 50017 / GB 50007 /
GB 50009 / GB 50011 等）。

## 功能

- **AI 解析** —— 自然语言或图片（最多 5 张）→ 结构化公式；思考过程流式展示，可续写微调
- **公式计算台** —— 动态参数表单、二维数学排版、分步过程、代入验算、多结果输出
- **导出** —— Excel 公式（单元格引用式 / 带数值式）、Word 计算书
- **归档** —— 历史与收藏、Token 用量统计、备份恢复（本地 / WebDAV，可选密码加密）
- **模型** —— 多档位配置、思考强度、双接入协议；内置模型测试台
- **外观** —— 背景图与透明度、文字颜色、深浅主题

## 技术栈

| 层 | 选型 |
|---|---|
| 应用框架 | Tauri 2 |
| 后端 | Rust（6 crate workspace） |
| 前端 | Vue 3 + TypeScript + Element Plus（Pinia + Vue Router） |
| 数据库 | SQLite（`rusqlite` bundled） |
| 密钥存储 | Windows 凭据管理器（`keyring`） |
| HTTP | `reqwest`（rustls，流式 SSE） |
| Word 生成 | `docx-rs` |
| 备份加密 | `pbkdf2` + `aes-gcm` + `sha2` |
| 表达式引擎 | 自研（不引入第三方） |
| 打包 | NSIS（`.exe`），per-user 安装，简体中文 |

## 架构

```
前端 src/          Vue 3 视图 · Pinia · api/（唯一 IPC 出口，不直连 Tauri）
                   ↕  110 个 Tauri 命令 · 6 个流式事件
应用壳 src-tauri/  命令分发 · 全局状态 · 路径 · 密钥 · 配置 · 导出
                   ↕
领域 crate         core    领域模型 / 表达式引擎 / 二维排版 / Excel 转换 / 验算 / 内置库
                   store   SQLite 持久化（唯一依赖 rusqlite 的 crate）/ 图片存储
                   llm     双协议 LLM 客户端 / 流式解析 / 用量归一化
                   backup  tar.gz 归档 / 分块 AES-256-GCM 加密 / WebDAV
                   report  Word 计算书生成 / 导出选项 / 文本清洗
```

## 安装

从 [Releases](https://github.com/w2018/civilcalc-pc/releases) 下载最新的
`AI-Calculator_x.y.z_x64-setup.exe`，双击安装（per-user，无需管理员权限）。

> 需要 Windows 10 / 11（x64）。安装包**未做代码签名**，首次运行会被 SmartScreen
> 拦一下 —— 点「更多信息」→「仍要运行」。

## 从源码构建

需要 Windows 10/11 + MSVC 工具链（VS Build Tools 勾选「使用 C++ 的桌面开发」）、
Node.js 22 + pnpm 12、Rust stable（版本由 `rust-toolchain.toml` 固定）。

```bash
pnpm install
pnpm tauri dev      # 开发运行
pnpm typecheck      # 类型检查
pnpm tauri build    # 出安装包
```

> **Git Bash 用户**：直接用 `cargo` 会因 `link.exe` 被 coreutils 遮蔽而失败，
> 用 `./scripts/dev-env.sh cargo test --workspace` 包一层。
>
> **改版本号**用 `./scripts/bump-version.sh`（三处必须一致，有契约测试钉住）。
> 约定：默认次版本 +1、补丁位归零 —— `v1.0.10` 的下一个版本是 `v1.1.0`。

## 数据与隐私

- 数据（公式、历史、图片、配置）**只存本机**，不经过任何自有服务器。
- API Key 存 **Windows 凭据管理器**，不落配置文件。
- 与 AI 的通信只发往你自己配置的模型服务商。无遥测、无埋点。

## 与 Android 版兼容

Android 版 `civilcalc-android-v2`（v3.3.0）的 PC 端移植。备份包
（`tar.gz` + `CCENC1` / PBKDF2 / AES-256-GCM）与 Android 版**互相可恢复**。

## 许可证

[GNU General Public License v3.0](LICENSE)。

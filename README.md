<div align="center">

# AI全能计算器

**面向工程计算的 AI 公式工作台**

把一段自然语言描述、或一张手写算式照片，变成可计算、可校验、可导出的工程公式。

[![License](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%7C%2011-0078D4.svg)](#安装)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org)
[![Vue](https://img.shields.io/badge/Vue-3-42B883.svg)](https://vuejs.org)

</div>

---

## 这是什么

一个 Windows 桌面应用，把「工程计算」这件事从「翻规范、找公式、按计算器、抄进 Excel」
压缩成一条链路：

```
自然语言 / 手写照片
      ↓  AI 解析成结构化公式（带参数表、单位、取值依据）
   公式计算台
      ↓  实时求值 + 二维数学排版 + 分步过程 + 代入验算
   结果落地
      ↓  Excel 公式 / Word 计算书 / 历史归档 / 云端备份
```

内置 **55 条工程公式**，覆盖结构、地基基础、岩土基坑、荷载、施工工程量、
测量、造价、数学与几何，每条都标注了规范依据（GB 50010、GB 50017、GB 50007、
GB 50009、GB 50011、GB 50003、GB 50005、CJJ/T 8 …）。

---

## 功能

### 输入与解析

| 能力 | 说明 |
|---|---|
| **AI 自然语言解析** | 一句话描述需求 → 结构化公式（参数、单位、取值范围、结果符号） |
| **图片混合解析** | 最多 5 张照片与文字一起发送，直接从手写算式/规范截图取公式 |
| **AI 思考过程** | 流式实时展示推理过程，可复制、可全屏、随历史持久化 |
| **AI 续写微调** | 在已有公式基础上继续改；新版本命名 `原名-vN.0`，谱系可追溯、可回退 |
| **计算公式详解** | 需求理解 + 解决方式（分点）+ 逐步拆解，三档主题色高亮 |

### 计算与呈现

| 能力 | 说明 |
|---|---|
| **公式计算台** | 动态参数表单、必填校验、结果带单位；参数框支持四则运算与运算符快插 |
| **二维数学排版** | 分式、上下标、根号、绝对值、方程组大括号 —— 按运算树还原成数学式 |
| **多结果输出** | 一条公式输出多组结果（分号分段），四列对照表，逐条生成 Excel 公式 |
| **内置公式库** | 55 条带规范依据的公式，开箱即用 |

### 导出与归档

| 能力 | 说明 |
|---|---|
| **Excel 公式导出** | 单元格引用式 / 带数值式两种模式，导出前做语法与自引用校验 |
| **Word 计算书** | 章节按需勾选，可选渲染公式详解与分步过程，同名文件不覆盖 |
| **历史与收藏** | 计算自动入历史；收藏、批量管理、思考过程回溯 |
| **Token 用量统计** | 按模型统计输入 / 输出 / 缓存命中 / 思考 token，可导出 CSV |

### 数据与维护

| 能力 | 说明 |
|---|---|
| **备份与恢复** | 整包 `tar.gz`，逐类勾选，可选密码加密；支持本地文件与 WebDAV 云端 |
| **模型测试** | 对话式压测台，多轮记忆跨进程保留，上下文超限自动压缩，可自定义容量与系统提示词 |
| **模型与密钥** | 多档位配置、思考强度、双接入协议（Chat Completions / Responses）；密钥存系统凭据管理器 |
| **重置软件** | 9 类数据可分别重置，显示真实存量，逐项二次确认 |
| **外观** | 全局背景图与透明度实时调节，文字颜色可自定义或跟随主题 |

---

## 技术栈

| 层 | 选型 |
|---|---|
| 应用框架 | **Tauri 2**（Rust 后端 + 系统 WebView 前端） |
| 后端 | **Rust**（edition 2021，6 crate workspace） |
| 前端 | **Vue 3 + TypeScript + Element Plus**（Pinia + Vue Router，自研扁平风设计令牌） |
| 数据库 | **SQLite**（`rusqlite` bundled，7 张表） |
| 密钥存储 | **Windows 凭据管理器**（`keyring`） |
| HTTP | `reqwest`（rustls，流式 SSE） |
| Word 生成 | `docx-rs` |
| 备份加密 | `pbkdf2` + `aes-gcm` + `sha2` |
| 表达式引擎 | **自研**（词法 → 语法 → 求值，不引入第三方） |
| 拼音检索 | `pinyin` |
| 打包 | NSIS（`.exe`）+ MSI，per-user 安装，简体中文 |

---

## 架构

### 分层

```
┌──────────────────────────────────────────────────────────────────┐
│  前端  src/                                                       │
│  Vue 3 视图 · Pinia store · api/（**唯一** IPC 出口，不直连 Tauri）│
└──────────────────────────────┬───────────────────────────────────┘
                               │  Tauri IPC
                               │  110 个命令 · 6 个流式事件
                               │  命令名 snake_case，字段名 camelCase
┌──────────────────────────────┴───────────────────────────────────┐
│  应用壳  src-tauri/                                               │
│  命令分发 · 全局状态 · 路径 · 密钥 · 配置 · 导出 · 启动流程          │
└──────────────────────────────┬───────────────────────────────────┘
        ┌──────────┬───────────┼───────────┬────────────┐
        ▼          ▼           ▼           ▼            ▼
  civilcalc-  civilcalc-  civilcalc-  civilcalc-  civilcalc-
     core        store        llm        backup      report
    领域层      持久化层      服务层       服务层      服务层
```

### crate 职责

| crate | 职责 |
|---|---|
| `civilcalc-core` | 领域模型、表达式引擎、二维排版、Excel 转换、验算、版本谱系、检索、内置公式库 |
| `civilcalc-store` | SQLite 持久化（**唯一**依赖 `rusqlite` 的 crate）、图片存储、迁移 |
| `civilcalc-llm` | 双协议 LLM 客户端、流式解析、思考强度映射、用量归一化 |
| `civilcalc-backup` | `tar.gz` 归档、分块 AES-256-GCM 加密、WebDAV 客户端 |
| `civilcalc-report` | Word 计算书生成、模板模型、导出选项、文本清洗 |
| `src-tauri` | Tauri 应用壳与命令层 |

### 几条贯穿全局的约定

- **前端不直接依赖 Tauri API** —— 全部经 `src/api/` 收口，便于替换与测试。
- **四层字段一致** —— Rust 结构 ↔ SQLite 表 ↔ TS 类型 ↔ UI state 命名统一。
- **AI 输出视为不可信输入** —— 结构经 schema 校验后才入库；渲染前经白名单过滤；
  来源引用不允许由模型自填。
- **不静默降级** —— 后端命令一律返回 `Result<T, CommandError>`，错误向上传到界面。
- **前后端契约显式化** —— 命令名、枚举取值、DTO 字段名、版本号在两侧逐字对齐。
  这类错位不会报错，只会静默取不到值，所以改动时必须两侧同步。

### 项目结构

```
civilcalc-pc/
├── crates/                  # 5 个领域 crate
├── src-tauri/               # Tauri 应用壳
│   ├── src/commands/        # 22 个命令模块
│   └── tests/               # 契约测试与集成测试
├── src/                     # Vue 3 前端
│   ├── views/               # 12 个页面
│   ├── components/          # 组件
│   ├── api/                 # IPC 封装（唯一出口）
│   ├── stores/              # Pinia
│   └── styles/              # 设计令牌
├── scripts/                 # 版本号管理、构建环境包装、图标生成
└── .github/workflows/       # CI
```

---

## 安装

从 [Releases](https://github.com/w2018/civilcalc-pc/releases) 下载最新的
`AI全能计算器_x.y.z_x64-setup.exe`，双击安装即可（per-user 安装，无需管理员权限）。

> **系统要求**：Windows 10 / 11（x64）。
>
> ⚠️ 安装包**未做代码签名**，首次运行会被 SmartScreen 拦一下 ——
> 点「更多信息」→「仍要运行」即可。

---

## 从源码构建

### 环境要求

- **Windows 10/11** + MSVC 工具链（VS Build Tools 勾选「使用 C++ 的桌面开发」）
- **Node.js 22** + **pnpm 12**
- **Rust stable**（版本由 `rust-toolchain.toml` 固定）

### 构建

```bash
pnpm install
pnpm tauri dev          # 开发运行

pnpm typecheck          # 类型检查
pnpm tauri build        # 出安装包（NSIS .exe + MSI）
```

### Windows + Git Bash 用户

直接用 `cargo` 可能失败：Git Bash 的 `link.exe`（GNU coreutils）会遮蔽 MSVC 的
`link.exe`，另外 `vcvars64.bat` 在受限环境里可能只设了 `PATH` 而没设 `LIB`。

仓库内提供了不依赖 `vcvars64.bat` 的环境包装脚本，直接从文件系统探测工具链路径：

```bash
./scripts/dev-env.sh cargo test --workspace
./scripts/dev-env.sh cargo clippy --workspace --all-targets -- -D warnings
./scripts/dev-env.sh pnpm tauri build
```

### 版本号

版本号在 `Cargo.toml` / `src-tauri/tauri.conf.json` / `package.json` 三处必须一致
（有契约测试钉住）。用脚本改，别手改：

```bash
./scripts/bump-version.sh          # 补丁位 +1
./scripts/bump-version.sh --minor  # 次版本 +1
./scripts/bump-version.sh --show   # 查看当前版本
```

---

## 数据与隐私

- 所有数据（公式、历史、图片、配置）**只存在本机**，不经过任何自有服务器。
- API Key 存入 **Windows 凭据管理器**，不落配置文件、不随配置导出（除非显式勾选）。
- 与 AI 的通信**只发往你自己配置的模型服务商**（OpenAI / Anthropic / 智谱等）。
- 无遥测、无埋点、无后台上报。
- 备份包默认本地导出；启用 WebDAV 时才会连你填写的网盘地址。

---

## 与 Android 版的兼容性

本应用是 Android 版 `civilcalc-android-v2`（v3.3.0）的 PC 端移植，并保持两条兼容性红线：

- **备份包二进制兼容** —— `tar.gz` 结构与加密格式（`CCENC1` 魔数、PBKDF2 200k 迭代、
  AES-256-GCM 64 KiB 分块）与 Android 版一致，两端的备份包可互相恢复。
- **Excel 转换契约不变** —— 单元格引用与公式生成的语义与 Android 版逐条对齐。

---

## 许可证

[GNU General Public License v3.0](LICENSE)。

本项目为独立重写实现，与 Android 版同源同许可。

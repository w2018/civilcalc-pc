#!/usr/bin/env python3
"""从单一母版 PNG 生成 Tauri 所需的全部图标尺寸。

用法：
    python scripts/gen-icons.py

输入：assets/app-icon.png（母版，**至少 256x256**，带 alpha）
输出：src-tauri/icons/*.png + icon.ico

## 为什么是「从母版位图缩」而不是「用代码画」

应用图标是一张**彩色插画**（紫机身 + 黄显示屏 + 青/黄/粉按键 + 深色描边），
不是能几行路径写出来的几何标识 —— 用代码重画只会画歪。
所以母版留在 `assets/app-icon.png`，这里只负责**缩放 + 打包**。

⚠️ 别把这里的逻辑改成「手写 SVG 再光栅化」：曾经试过（v1.0.8），
画出来的图形与真正的应用图标对不上。图形以母版为准。

## 为什么要「预乘 alpha」重采样

母版是 RGBA，透明区域的 RGB 是 (0,0,0)。若直接对 RGBA 做 LANCZOS 缩放，
边缘像素会把「黑色的透明像素」混进来，缩出来的图标四周会出现**黑边/灰边**。

正确做法是先把 RGB 乘以 alpha（预乘），在预乘空间重采样，再除回去。
Pillow 里 `convert("RGBa")` 就是预乘表示，`convert("RGBA")` 会还原 —— 
这一对转换是本脚本的关键。

## 依赖

需要 **Pillow**（`pip install pillow`）。界面里的品牌标识
（`src/components/common/AppLogo.vue`）是内联 SVG，与本脚本**互不影响** ——
两者图形一致，但一个是矢量一个是位图，各自服务不同场景。

## 尺寸清单

Tauri 官方 `tauri icon` 会生成下面这一整套；本脚本对齐它，
其中 Windows 打包（NSIS / MSI）**实际必需**的只有：

    icon.ico  +  32x32.png  +  128x128.png  +  128x128@2x.png

其余 `Square*Logo.png` / `StoreLogo.png` 是 MSIX（Windows 商店包）用的，
当前 `bundle.targets` 不含 appx，但一并生成以免将来加 target 时缺文件。
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "assets" / "app-icon.png"
OUT = ROOT / "src-tauri" / "icons"

# ---------------------------------------------------------------- 尺寸清单

# 常规 PNG（文件名 → 边长）
PNGS: dict[str, int] = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 256,
    # MSIX / Windows 商店图标
    "StoreLogo.png": 50,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
}

# ICO 内嵌尺寸。Windows 会在不同场景挑最合适的一档：
# 16 标题栏 / 24 小图标 / 32 任务栏 / 48 桌面 / 256 大图标视图
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def load_master() -> Image.Image:
    if not SRC.exists():
        sys.exit(f"[ERROR] 母版图标不存在: {SRC}")
    im = Image.open(SRC)
    if im.mode != "RGBA":
        im = im.convert("RGBA")
    if min(im.size) < 256:
        sys.exit(f"[ERROR] 母版至少需要 256x256，当前 {im.size}")
    return im


def resample(im: Image.Image, size: int) -> Image.Image:
    """预乘 alpha 重采样，避免透明边缘产生黑边。"""
    if im.size == (size, size):
        return im.copy()
    premul = im.convert("RGBa")  # 预乘 alpha
    scaled = premul.resize((size, size), Image.Resampling.LANCZOS)
    return scaled.convert("RGBA")  # 还原为直通 alpha


def main() -> None:
    master = load_master()
    OUT.mkdir(parents=True, exist_ok=True)
    print(f"母版: {SRC.name}  {master.size[0]}x{master.size[1]}  RGBA")
    print(f"输出: {OUT}")

    cache: dict[int, Image.Image] = {}

    def get(size: int) -> Image.Image:
        if size not in cache:
            cache[size] = resample(master, size)
        return cache[size]

    # ---- PNG ----
    for name, size in PNGS.items():
        img = get(size)
        path = OUT / name
        # optimize=True 对 32 位 PNG 收益有限，但无损
        img.save(path, format="PNG", optimize=True)
        print(f"  [png] {name:<28} {size}x{size}")

    # ---- ICO ----
    # 传入最大的那张 + sizes 列表，Pillow 会为每档各缩一份内嵌进去。
    # 直通 alpha 缩放（非预乘），但在 16~48px 上差异不可见。
    ico_path = OUT / "icon.ico"
    master_256 = get(256)
    master_256.save(ico_path, format="ICO", sizes=[(s, s) for s in ICO_SIZES])
    print(f"  [ico] {'icon.ico':<28} {ICO_SIZES}")

    # ---- 自检 ----
    with Image.open(ico_path) as check:
        got = sorted(check.info.get("sizes", []))
    print(f"\n校验 icon.ico 内嵌尺寸: {got}")
    assert len(got) >= 4, f"ICO 档位过少（{got}），Windows 缩放会糊"

    print(f"\n完成，共 {len(PNGS)} 个 PNG + 1 个 ICO")


if __name__ == "__main__":
    main()

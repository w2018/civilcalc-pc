<script setup lang="ts">
/**
 * AppLogo —— 应用标识（**内联 SVG**，与 `assets/app-icon.png` 同一套图形）。
 *
 * ## 这个图形从哪来
 *
 * 它就是本应用的**默认图标**（`assets/app-icon.png`，母版 256×256）：
 * 紫机身 + 黄显示屏（显示 `123`）+ 黄/青按键 + 粉色按键 + 深色描边。
 *
 * 这里按母版**逐块量出坐标**重画成矢量（用连通域分析取每个色块的包围盒，
 * 再除以 4 换算到 64 单位坐标系），所以缩放不会糊、也不依赖位图资源。
 *
 * ⚠️ **改图形要连母版一起改**：任务栏 / 安装包上的图标走
 * `scripts/gen-icons.py`（从 `assets/app-icon.png` 缩放），与本组件是两份
 * 不同介质。两者图形必须一致 —— 这是需求「设置/关于的图标要与其它图片风格一致」
 * 的另一半：界面里的标识和系统里的图标是同一个。
 *
 * ## 为什么内联 SVG 而不是 `<img>` 引位图
 *
 * - 任意尺寸都锐利（「关于」页 56px、侧栏 26px、设置页 40px 共用一份）
 * - 不占资源请求、不受打包路径影响
 *
 * ## 配色（从母版采样）
 *
 * | 用途 | 色值 |
 * |---|---|
 * | 描边 | `#004058` |
 * | 机身 | `#B898E0` |
 * | 显示屏 / 第一排按键 | `#F8F078` |
 * | 第二排 / 底部宽按键 | `#50E0C8` |
 * | 底部粉按键 | `#F8C8D0` |
 * | 显示屏数字 | `#F07090` |
 *
 * ⚠️ 这些颜色**不跟随主题**：它是应用图标，本来就该在任何主题下
 * 保持同一副面孔（和任务栏图标一致）。
 */
withDefaults(
  defineProps<{
    /** 渲染边长（px） */
    size?: number
    /** 无障碍名称（默认「AI全能计算器」） */
    label?: string
  }>(),
  { size: 40, label: 'AI全能计算器' },
)

/** 描边与配色 */
const NAVY = '#004058'
const LAVENDER = '#B898E0'
const YELLOW = '#F8F078'
const TEAL = '#50E0C8'
const PINK = '#F8C8D0'
const ROSE = '#F07090'

/** 按键网格：三列（x）与三行（y） */
const COLS = [17.1, 28.3, 39.4]
const ROWS = [24.9, 35.3, 45.9]
/** 单个按键尺寸 */
const BTN_W = 7.2
const BTN_H = 6.9

/** 第一排黄键、第二排青键（各三个） */
const KEYS_ROW1 = COLS.map((x) => ({ x, y: ROWS[0], w: BTN_W, h: BTN_H, fill: YELLOW }))
const KEYS_ROW2 = COLS.map((x) => ({ x, y: ROWS[1], w: BTN_W, h: BTN_H, fill: TEAL }))

/** 第三排：一个横跨两列的宽青键 + 一个粉键 */
const KEYS_ROW3 = [
  { x: COLS[0], y: ROWS[2], w: COLS[1] + BTN_W - COLS[0], h: BTN_H, fill: TEAL },
  { x: COLS[2], y: ROWS[2], w: BTN_W, h: BTN_H, fill: PINK },
]
</script>

<template>
  <svg
    class="logo"
    :width="size"
    :height="size"
    viewBox="0 0 64 64"
    role="img"
    :aria-label="label"
  >
    <!-- 机身 -->
    <rect
      x="9.6"
      y="6.6"
      width="44.8"
      height="50.8"
      rx="9.5"
      :fill="LAVENDER"
      :stroke="NAVY"
      stroke-width="2.6"
    />

    <!-- 显示屏 -->
    <rect
      x="14.8"
      y="10.4"
      width="34"
      height="10.4"
      rx="5.2"
      :fill="YELLOW"
      :stroke="NAVY"
      stroke-width="2.6"
    />

    <!--
      显示屏数字。

      用 `<text>` 而不是画路径：这三个字只占图标高度的 10%，
      换成路径会多出上百个坐标点却看不出差别。
      字体栈与全站一致，`font-weight: 700` 对齐母版里的粗体观感。
    -->
    <text
      x="17.4"
      y="18.6"
      :fill="ROSE"
      font-size="9"
      font-weight="700"
      font-family="'PingFang SC', 'Microsoft YaHei', 'Helvetica Neue', system-ui, sans-serif"
      letter-spacing="0.2"
    >
      123
    </text>

    <!-- 按键 -->
    <rect
      v-for="(k, i) in [...KEYS_ROW1, ...KEYS_ROW2, ...KEYS_ROW3]"
      :key="i"
      :x="k.x"
      :y="k.y"
      :width="k.w"
      :height="k.h"
      rx="2.6"
      :fill="k.fill"
      :stroke="NAVY"
      stroke-width="2.2"
    />
  </svg>
</template>

<style scoped>
.logo {
  display: block;
  flex-shrink: 0;
}
</style>

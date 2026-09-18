<script setup lang="ts">
/**
 * NavIcon —— 侧边导航的**内联 SVG 图标**（需求 9）。
 *
 * ## 为什么不再用 emoji
 *
 * 原来导航项用 `✨ 🧮 ⭐ 🕘 📚 🧪 📊 💾 🖼 ⚙ ℹ` 这些 emoji 当图标。
 * 问题不是「不好看」，是**不受控**：
 *
 * - 字形由**系统字体**决定（Segoe UI Emoji / Noto Color Emoji…），
 *   同一份代码在不同机器上形状与配色都不一样
 * - 全是**彩色**的，而侧栏是单色扁平风 —— 一排彩色小图会把整站的设计语言打散
 * - 尺寸 / 基线 / 描边粗细都无法统一（emoji 是字体，只能改字号）
 *
 * 内联 SVG 三个问题一起解决：路径写死在代码里、用 `currentColor` 跟着
 * 主题与选中态走、任意尺寸都锐利。
 *
 * ## 只用描边，不用填充
 *
 * 全部 `fill="none"` + `stroke="currentColor"`，粗细 1.7 ——
 * 与整站「细线 + 扁平」的语言一致，也天然支持选中变色。
 *
 * ## 不引图标库
 *
 * 引入 `@element-plus/icons-vue` 会多打一份资源，且它的风格（实心/圆角）
 * 与本项目不搭。11 个图标手写路径只有几 KB，且能精确控制。
 *
 * ## 为什么用「图元数组」而不是 `v-html`
 *
 * `v-html` 在这里虽然安全（内容是代码里写死的，不是 AI 输出），
 * 但项目的硬约定是「`v-html` 只用于已过滤的 AI 输出」——
 * 留一个例外会让后来人分不清哪处 v-html 需要过滤。
 * 所以把每个图标写成图元数组，模板里逐个渲染。
 */
type Prim =
  | { t: 'path'; d: string }
  | { t: 'circle'; cx: number; cy: number; r: number }
  | { t: 'rect'; x: number; y: number; w: number; h: number; r: number }

const props = withDefaults(
  defineProps<{
    /** 图标名（与 `router/nav.ts` 的 `icon` 字段一致） */
    name: string
    /** 渲染边长（px） */
    size?: number
  }>(),
  { size: 20 },
)

/**
 * 每个图标的图元。坐标系固定 24×24 —— 所有图标共用同一个 `viewBox`，
 * 换尺寸时只需改渲染宽高，形状比例不会变。
 */
const ICONS: Record<string, Prim[]> = {
  // 新建公式：四角星（"生成"的通用语义）+ 一颗小星
  sparkle: [
    { t: 'path', d: 'M12 3.2l1.7 5.1 5.1 1.7-5.1 1.7L12 16.8l-1.7-5.1L5.2 10l5.1-1.7z' },
    { t: 'path', d: 'M18.4 15.4l.8 2.3 2.3.8-2.3.8-.8 2.3-.8-2.3-2.3-.8 2.3-.8z' },
  ],

  // 公式计算：计算器（外框 + 显示屏 + 按键 + 底线）
  calculator: [
    { t: 'rect', x: 5, y: 2.5, w: 14, h: 19, r: 2.5 },
    { t: 'path', d: 'M8.5 6.6h7' },
    { t: 'path', d: 'M9 11.4h.01M12 11.4h.01M15 11.4h.01M9 14.9h.01M12 14.9h.01M15 14.9h.01' },
    { t: 'path', d: 'M9 18.4h6' },
  ],

  // 收藏：五角星
  star: [
    { t: 'path', d: 'M12 3.3l2.7 5.5 6.1.9-4.4 4.3 1 6.1-5.4-2.9-5.4 2.9 1-6.1L3.2 9.7l6.1-.9z' },
  ],

  // 历史：表盘 + 指针
  history: [
    { t: 'circle', cx: 12, cy: 12, r: 8.8 },
    { t: 'path', d: 'M12 7.4V12l3.2 1.9' },
  ],

  // 公式库：书
  library: [
    { t: 'path', d: 'M5 4.6A2.6 2.6 0 0 1 7.6 2H19v18H7.6A2.6 2.6 0 0 0 5 22.6z' },
    { t: 'path', d: 'M5 19.4A2.6 2.6 0 0 1 7.6 17H19' },
    { t: 'path', d: 'M9 6.5h6' },
  ],

  // 模型测试：锥形瓶 + 液面
  flask: [
    { t: 'path', d: 'M9.5 2.8h5' },
    { t: 'path', d: 'M10.5 2.8v6.4L5.6 18a2 2 0 0 0 1.7 3h9.4a2 2 0 0 0 1.7-3l-4.9-8.8V2.8' },
    { t: 'path', d: 'M7.6 14.5h8.8' },
  ],

  // 用量：柱状图
  chart: [
    { t: 'path', d: 'M3.2 20.5h17.6' },
    { t: 'path', d: 'M6.5 20.5v-5.2M11 20.5V7.2M15.5 20.5v-8.4M20 20.5v-3.4' },
  ],

  // 备份：云 + 向上箭头
  backup: [
    { t: 'path', d: 'M7.4 19h9.4a4.1 4.1 0 0 0 .6-8.15A5.6 5.6 0 0 0 6.9 9.4 4.3 4.3 0 0 0 7.4 19z' },
    { t: 'path', d: 'M12 16.2v-4.6' },
    { t: 'path', d: 'M10 13.6l2-2 2 2' },
  ],

  // 图片缓存：相框 + 太阳 + 山
  image: [
    { t: 'rect', x: 3.4, y: 4.6, w: 17.2, h: 14.8, r: 2.4 },
    { t: 'circle', cx: 8.6, cy: 9.7, r: 1.5 },
    { t: 'path', d: 'M4.2 16.6l4.6-4.4 4.2 4.2 2.6-2.4 4.2 4' },
  ],

  // 设置：三条滑杆 + 旋钮（比齿轮更容易在小尺寸下辨认）
  settings: [
    { t: 'path', d: 'M3.6 7h8.2M17.2 7h3.2M3.6 12h3.4M11 12h9.4M3.6 17h9.4M18.4 17h2' },
    { t: 'circle', cx: 15.4, cy: 7, r: 1.9 },
    { t: 'circle', cx: 9.2, cy: 12, r: 1.9 },
    { t: 'circle', cx: 16.6, cy: 17, r: 1.9 },
  ],

  // 关于：信息圈
  info: [
    { t: 'circle', cx: 12, cy: 12, r: 8.8 },
    { t: 'path', d: 'M12 11v5.4' },
    { t: 'path', d: 'M12 7.7h.01' },
  ],
}

const prims = (): Prim[] => ICONS[props.name] ?? []
</script>

<template>
  <svg
    class="nav-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <template v-for="(p, i) in prims()" :key="i">
      <path v-if="p.t === 'path'" :d="p.d" />
      <circle v-else-if="p.t === 'circle'" :cx="p.cx" :cy="p.cy" :r="p.r" />
      <rect v-else :x="p.x" :y="p.y" :width="p.w" :height="p.h" :rx="p.r" />
    </template>
  </svg>
</template>

<style scoped>
.nav-icon {
  display: block;
  flex-shrink: 0;
}
</style>

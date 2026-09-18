<script setup lang="ts">
/**
 * MarkdownText —— AI 输出渲染（三档高亮 + `{{img:N}}` 内联图）。
 *
 * 见 docs/05-项目开发方案.md §2.3.2 与 `docs/07` 的 P4-18。
 *
 * ## 三档高亮
 *
 * | 标记 | 渲染 | 用途（提示词契约） |
 * |---|---|---|
 * | `**加粗**` | 加粗 | 关键依据 / 结论 / 系数含义 |
 * | `` `代码` `` | 行内代码（等宽 + 浅底） | 变量符号 / 系数数值 / 单位 |
 * | `==高亮==` | 金色荧光底 | 每步最核心的结果或结论 |
 *
 * ## `{{img:N}}` 是**内联**的，不是堆到文末
 *
 * 正文先按 `{{img:N}}` 切段，文本段各渲染一份 Markdown，
 * 图片段插在它原本的位置。序号 N 从 1 起，对应 `resolve_images` 的返回。
 * 取不到图（已被删除）时渲染「图片已删除」占位 —— **不要**整段丢掉，
 * 否则用户会以为模型没提图。
 *
 * ## 安全
 *
 * 所有 HTML 走 `renderMarkdown()` 的白名单过滤后才 `v-html`。
 * **不要**在别处直接 `v-html` AI 输出。
 */
import { computed } from 'vue'
import { renderMarkdown } from '@/utils/markdown'
import { splitByImageRefs, type ResolvedImages } from '@/types/ai'

const props = withDefaults(
  defineProps<{
    /** 原始 Markdown 文本 */
    text: string
    /** `{{img:N}}` 的解析结果（序号 → data URL）；为 `null` 时图位显示占位 */
    images?: ResolvedImages | null
    /** 内联模式：不产生块级外边距（用于表格单元格、参数说明等） */
    inline?: boolean
    /**
     * 紧凑模式：整体字号降一档、段间距收紧。
     *
     * 用于**对话流**（模型测试）—— 那里一屏要塞下多轮问答，
     * 正文用 14px 会显得很占地方。公式工作台里的详解仍用常规字号。
     */
    compact?: boolean
  }>(),
  { images: null, inline: false, compact: false },
)

const emit = defineEmits<{
  /** 点击内联图（父组件弹全屏查看器） */
  (e: 'open-image', index: number): void
}>()

/** 切段 + 逐段渲染（图片段原样保留位置） */
const parts = computed(() =>
  splitByImageRefs(props.text).map((p) =>
    p.kind === 'text' ? { kind: 'text' as const, html: renderMarkdown(p.text) } : p,
  ),
)

/** 取第 N 张图（N 从 1 起） */
function srcOf(index: number): string | null {
  return props.images?.[String(index)] ?? null
}
</script>

<template>
  <div class="md" :class="{ 'md--inline': inline, 'md--compact': compact }">
    <template v-for="(p, i) in parts" :key="i">
      <!-- 文本段：已过滤的 HTML -->
      <component :is="inline ? 'span' : 'div'" v-if="p.kind === 'text'" class="md__block" v-html="p.html" />

      <!-- 图片段：内联在原位 -->
      <button
        v-else
        class="md__img"
        type="button"
        :aria-label="`查看第 ${p.index} 张附图`"
        @click="emit('open-image', p.index)"
      >
        <img v-if="srcOf(p.index)" :src="srcOf(p.index)!" :alt="`附图 ${p.index}`" />
        <span v-else class="md__img-missing">图片已删除（第 {{ p.index }} 张）</span>
      </button>
    </template>
  </div>
</template>

<style scoped>
.md {
  color: var(--c-text);
  font-size: var(--f-size-base);
  line-height: 1.7;
  word-break: break-word;
}

.md--inline {
  display: inline;
  line-height: inherit;
}

/**
 * 紧凑模式（对话流）：整档降一级字号 + 收紧行高与列表缩进。
 *
 * 正文 14px → 13px，标题 16px → 14px，行高 1.7 → 1.6。
 * 一屏能多放约 20% 内容，同时不影响可读性。
 */
.md--compact {
  font-size: var(--f-size-sm);
  line-height: 1.6;
}

.md--compact :deep(h1),
.md--compact :deep(h2),
.md--compact :deep(h3),
.md--compact :deep(h4),
.md--compact :deep(h5),
.md--compact :deep(h6) {
  margin: var(--sp-2) 0 var(--sp-1);
  font-size: var(--f-size-base);
}

.md--compact :deep(ul),
.md--compact :deep(ol) {
  margin: var(--sp-1) 0;
  padding-left: var(--sp-4);
}

.md--compact :deep(li) {
  margin: 1px 0;
}

.md--compact :deep(pre) {
  margin: var(--sp-1) 0;
  padding: var(--sp-2);
}

.md--compact :deep(blockquote) {
  margin: var(--sp-1) 0;
  padding: var(--sp-1) var(--sp-3);
}

.md--compact :deep(table) {
  font-size: var(--f-size-xs);
}

.md__block {
  margin: 0;
}

/**
 * 🔴 段间距必须由 `p` 自己带出来。
 *
 * `marked` 把每个自然段包成 `<p>`，而一个「文本段」（按 `{{img:N}}` 切出来的）
 * 里通常有**很多个 `<p>`**。只给 `.md__block + .md__block` 加间距的话，
 * 段与段之间会完全贴死 —— 整段输出看起来像一坨纯文本，
 * 用户会觉得「Markdown 没生效」。
 */
.md :deep(p) {
  margin: 0 0 var(--sp-2);
}

.md :deep(p:last-child) {
  margin-bottom: 0;
}

/* 紧凑模式：段落间只留 4px */
.md--compact :deep(p) {
  margin-bottom: var(--sp-1);
}

/* 段与段之间给一点呼吸（内联模式不给） */
.md:not(.md--inline) .md__block + .md__block,
.md:not(.md--inline) .md__block + .md__img,
.md:not(.md--inline) .md__img + .md__block {
  margin-top: var(--sp-2);
}

/* ---- 三档高亮（v-html 内容要走 :deep）---- */

.md :deep(strong),
.md :deep(b) {
  font-weight: 600;
  color: var(--c-text);
}

.md :deep(code) {
  padding: 1px 5px;
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  font-family: var(--f-mono);
  font-size: 0.92em;
  color: var(--c-text);
}

/* 高亮底色：金色荧光（与提示词里的「金色荧光底」一致） */
.md :deep(mark) {
  padding: 0 3px;
  border-radius: var(--r-sm);
  background: #ffe58f;
  color: #613400;
}

[data-theme='dark'] .md :deep(mark) {
  background: #6b4d00;
  color: #ffe58f;
}

.md :deep(pre) {
  margin: var(--sp-2) 0;
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
  overflow-x: auto;
}

.md :deep(pre code) {
  padding: 0;
  background: transparent;
}

.md :deep(ul),
.md :deep(ol) {
  margin: var(--sp-2) 0;
  padding-left: var(--sp-5);
}

.md :deep(li) {
  margin: 2px 0;
}

.md :deep(blockquote) {
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
  border-left: 3px solid var(--c-divider);
  background: var(--c-surface-2);
  color: var(--c-text-2);
}

.md :deep(a) {
  color: var(--c-link);
  text-decoration: none;
}

.md :deep(a:hover) {
  text-decoration: underline;
}

.md :deep(h1),
.md :deep(h2),
.md :deep(h3),
.md :deep(h4),
.md :deep(h5),
.md :deep(h6) {
  margin: var(--sp-3) 0 var(--sp-2);
  font-size: var(--f-size-lg);
  font-weight: 600;
  line-height: 1.4;
}

/* 层级靠字号递减，而不是靠更大的外边距（对话里标题不该撑开版面） */
.md :deep(h4),
.md :deep(h5),
.md :deep(h6) {
  font-size: var(--f-size-base);
}

.md :deep(table) {
  width: 100%;
  margin: var(--sp-2) 0;
  border-collapse: collapse;
  font-size: var(--f-size-sm);
}

.md :deep(th),
.md :deep(td) {
  padding: var(--sp-2);
  border-bottom: var(--hairline) solid var(--c-divider);
  text-align: left;
}

.md :deep(th) {
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-weight: 500;
}

.md :deep(hr) {
  margin: var(--sp-4) 0;
  border: none;
  border-top: var(--hairline) solid var(--c-divider);
}

/* ---- 内联图 ---- */

.md__img {
  display: block;
  margin: var(--sp-2) 0;
  padding: 0;
  border: none;
  border-radius: var(--r-card);
  background: var(--c-surface-2);
  cursor: zoom-in;
  overflow: hidden;
  max-width: 100%;
}

.md__img img {
  display: block;
  max-width: 100%;
  max-height: 320px;
  object-fit: contain;
}

.md__img-missing {
  display: block;
  padding: var(--sp-4);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}
</style>

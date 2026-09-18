<script setup lang="ts">
/**
 * 二维数学排版渲染器（P3-12）。
 *
 * 见 docs/08-IPC契约.md 组 12b 与 ADR-019。
 *
 * ## 两种用法
 *
 * 1. **传 `expression`（最常用）**：组件内部调 `mathLayout` 取布局树。
 *    这是公式头部、分步、结果行的标准入口。
 * 2. 已拿到布局树时也可直接传 `layout`（本组件也接受）。
 *
 * ## 🔴 降级约定（硬约束）
 *
 * `mathLayout` **失败不报错**：坏段不进 `lines` 但记进 `warnings`，
 * 此时 `lines` 可能为空。两类情况都**必须回落成等宽原文**：
 *
 * - `lines` 为空（排版整体失败）→ 显示 `fallbackText`（默认即 expression）
 * - 单段被剔除但其它段还在 → 仍渲染可用段，并**在底部灰字提示** warnings
 *
 * 绝不静默丢内容。
 *
 * ## 其它行为
 *
 * - 长公式横向滚动（容器 `overflow-x:auto`），不折行。
 * - 点击展开全屏（`el-dialog`），大字号看完整公式。
 * - 方程组（多行且每行带 ① 编号）在左侧加一道大括号。
 * - 文本按 `kind` 上色（见 `tokens.css` 的 `--c-math-*`）。
 */
import { computed, ref, watch } from 'vue'
import { mathLayout } from '@/api/display'
import { isUsable, type MathLayout } from '@/types/math'
import MathNodeView from './MathNodeView.vue'

const props = withDefaults(
  defineProps<{
    /** 公式表达式（含分号分段）。传 `layout` 时可省略。 */
    expression?: string
    /** schema 自定义常量（让 `alpha` 被识别为常量而非变量） */
    constants?: Record<string, number>
    /** 已算好的布局树（传了就跳过 fetch） */
    layout?: MathLayout | null
    /** 强制等宽原文（不排版） */
    raw?: boolean
    /** 字号档 */
    size?: 'sm' | 'base' | 'lg'
    /** 点击展开全屏 */
    fullscreen?: boolean
  }>(),
  {
    expression: '',
    constants: undefined,
    layout: null,
    raw: false,
    size: 'base',
    fullscreen: true,
  },
)

const fetched = ref<MathLayout | null>(null)
const loading = ref(false)
const error = ref(false)

/** 实际使用的布局：优先已传入的 `layout`，否则用 fetch 结果 */
const layout = computed<MathLayout | null>(() => props.layout ?? fetched.value)

async function render(): Promise<void> {
  if (props.layout || props.raw || !props.expression.trim()) {
    fetched.value = null
    return
  }
  loading.value = true
  error.value = false
  try {
    fetched.value = await mathLayout(props.expression, props.constants)
  } catch (e) {
    error.value = true
    fetched.value = null
    console.warn('[MathDisplay] 排版失败，回落原文', e)
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.expression, props.constants, props.raw, props.layout],
  () => void render(),
  { immediate: true },
)

const usable = computed(() => isUsable(layout.value))
const showRaw = computed(() => props.raw || error.value || !usable.value)
const hasWarnings = computed(
  () => !!layout.value && layout.value.warnings.length > 0,
)

/** 方程组：多行且每行都带编号（①②③…） */
const isSystem = computed(
  () =>
    !!layout.value &&
    layout.value.lines.length > 1 &&
    layout.value.lines.every((l) => !!l.mark),
)

const fullscreenOpen = ref(false)
function openFullscreen(): void {
  if (props.fullscreen && !showRaw.value) fullscreenOpen.value = true
}
</script>

<template>
  <div class="math" :class="`math--${size}`">
    <!-- 加载中 -->
    <span v-if="loading" class="math__loading">排版中…</span>

    <!-- 降级：等宽原文 -->
    <code v-else-if="showRaw" class="math__raw">{{ expression }}</code>

    <!-- 排版结果 -->
    <div v-else class="math__scroll" @dblclick="openFullscreen">
      <div class="math__lines" :class="{ 'math__lines--system': isSystem }">
        <div v-for="(line, li) in layout!.lines" :key="li" class="math__line">
          <span v-if="line.symbol" class="math__sym">{{ line.symbol }} =</span>
          <span class="math__nodes">
            <MathNodeView v-for="(n, ni) in line.nodes" :key="ni" :node="n" />
          </span>
          <span v-if="line.mark" class="math__mark">{{ line.mark }}</span>
        </div>
      </div>

      <p v-if="hasWarnings" class="math__warn">
        部分段落无法排版，已按原文显示
      </p>
    </div>

    <!-- 全屏 -->
    <el-dialog
      v-model="fullscreenOpen"
      title="公式详情"
      width="min(92vw, 900px)"
      append-to-body
      class="math__dialog"
    >
      <div class="math__lines math__lines--big">
        <div v-for="(line, li) in layout!.lines" :key="li" class="math__line">
          <span v-if="line.symbol" class="math__sym">{{ line.symbol }} =</span>
          <span class="math__nodes">
            <MathNodeView v-for="(n, ni) in line.nodes" :key="ni" :node="n" />
          </span>
          <span v-if="line.mark" class="math__mark">{{ line.mark }}</span>
        </div>
      </div>
      <p v-if="hasWarnings" class="math__warn">部分段落无法排版，已按原文显示</p>
    </el-dialog>
  </div>
</template>

<style scoped>
.math {
  width: 100%;
}

.math--sm {
  font-size: var(--f-size-sm);
}
.math--base {
  font-size: var(--f-size-base);
}
.math--lg {
  font-size: var(--f-size-lg);
}

.math__loading {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

/* 降级原文 */
.math__raw {
  display: block;
  font-family: var(--f-mono);
  font-size: inherit;
  line-height: 1.7;
  color: var(--c-text);
  white-space: pre-wrap;
  word-break: break-all;
}

/* 横向滚动容器 */
.math__scroll {
  overflow-x: auto;
  overflow-y: hidden;
  cursor: zoom-in;
}

.math__lines {
  display: inline-flex;
  flex-direction: column;
  gap: var(--sp-2);
  min-width: 100%;
  padding: 2px 0;
}

/* 方程组左侧大括号 */
.math__lines--system {
  position: relative;
  padding-left: var(--sp-4);
  margin-left: 4px;
}
.math__lines--system::before {
  content: '';
  position: absolute;
  left: 0;
  top: 0.3em;
  bottom: 0.3em;
  width: 8px;
  border: 2px solid var(--c-math-bracket);
  border-right: none;
  border-radius: 6px 0 0 6px;
}

.math__line {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: nowrap;
  color: var(--c-text);
  line-height: 1.8;
}

.math__sym {
  flex-shrink: 0;
  color: var(--c-math-symbol);
  font-style: italic;
}

.math__nodes {
  display: inline-flex;
  align-items: center;
  flex-wrap: nowrap;
}

.math__mark {
  flex-shrink: 0;
  margin-left: var(--sp-1);
  color: var(--c-text-3);
  font-size: 0.9em;
}

.math__warn {
  margin: var(--sp-1) 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

/* 全屏 */
.math__lines--big {
  font-size: var(--f-size-2xl);
  line-height: 2;
  padding: var(--sp-3) 0;
}
</style>

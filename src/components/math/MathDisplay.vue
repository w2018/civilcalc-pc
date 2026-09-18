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
 * - 双击展开全屏（`el-dialog`）放大预览；弹窗里**先按比例缩字号让整条公式装下**，
 *   缩到下限仍装不下才横向滚动（见 `fitFormula()` 的说明）。
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

/**
 * 弹窗里公式的缩放比（`1` = 原字号）。
 *
 * ## 🔴 为什么是「缩字号 + 横向滚动条」而不是折行
 *
 * 折行虽然也能看全，但数学式一折行**项的边界就断了**
 * （`nTop×LTop×π/4` 可能被切成两行），读起来容易看错 —— 排版组件整体上
 * 也是「长公式不折行、横向滚动」的口径。
 *
 * 所以策略是两步：
 * 1. 先量「内容宽 ÷ 可用宽」，能靠缩字号一行装下就装下（最干净）；
 * 2. 缩到 `MIN_FIT_SCALE` 还装不下就**不再缩**，交给横向滚动条。
 *
 * ⚠️ 必须在弹窗 `opened` 之后才量：开启动画期间宽度还没稳定。
 */
const fitScale = ref(1)
const fitBoxRef = ref<HTMLElement | null>(null)

/**
 * 缩放下限。
 *
 * `--f-size-2xl` 是 24px、`--f-size-base` 是 14px —— 24 × 0.6 ≈ 14.4px，
 * 也就是「缩到底也还和正文一样大」。再往下缩，「放大预览」就不如不放大，
 * 不如留着字号让用户横向拖。
 */
const MIN_FIT_SCALE = 0.6

/** 最多量两轮：间距/内边距是固定 px，缩放不是严格线性，第二轮修掉残差 */
const MAX_FIT_PASSES = 2

function fitFormula(): void {
  fitScale.value = 1
  measureAndFit(0)
}

function measureAndFit(pass: number): void {
  // 等一帧：让「缩放复位 + 新字号」的布局先落地再量，否则量到的是上一次的宽度
  requestAnimationFrame(() => {
    const box = fitBoxRef.value
    const inner = box?.firstElementChild as HTMLElement | null
    if (!box || !inner) return

    const avail = box.clientWidth
    const need = inner.getBoundingClientRect().width
    if (avail <= 0 || need <= avail) return

    // 留 1% 余量，别贴着边
    const wanted = fitScale.value * (avail / need) * 0.99
    if (wanted < MIN_FIT_SCALE) {
      // 缩到下限仍装不下 → 停在下限，剩下的交给横向滚动条
      fitScale.value = MIN_FIT_SCALE
      return
    }
    const next = Math.max(MIN_FIT_SCALE, wanted)
    if (next === fitScale.value) return
    fitScale.value = next
    if (pass + 1 < MAX_FIT_PASSES) measureAndFit(pass + 1)
  })
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

    <!-- 全屏（放大预览） -->
    <el-dialog
      v-model="fullscreenOpen"
      title="公式详情"
      width="min(94vw, 1200px)"
      append-to-body
      class="math__dialog"
      @opened="fitFormula"
    >
      <!-- ⚠️ 宽度只能在 `opened` 之后量（开启动画期间尺寸未定） -->
      <div ref="fitBoxRef" class="math__fit" :style="{ '--math-fit': String(fitScale) }">
        <div class="math__lines math__lines--big">
          <div v-for="(line, li) in layout!.lines" :key="li" class="math__line">
            <span v-if="line.symbol" class="math__sym">{{ line.symbol }} =</span>
            <span class="math__nodes">
              <MathNodeView v-for="(n, ni) in line.nodes" :key="ni" :node="n" />
            </span>
            <span v-if="line.mark" class="math__mark">{{ line.mark }}</span>
          </div>
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

/* 放大预览 */
.math__fit {
  /* 缩到下限（`MIN_FIT_SCALE`）仍装不下时横向滚动 —— 用全局那套 8px 细滚动条 */
  overflow: auto;
  /* 方程组等多行内容给个上限，别把弹窗撑出屏幕 */
  max-height: 60vh;
}

/* 字号按 `fitFormula()` 量出的 `--math-fit` 缩放；不折行（见脚本里的说明） */
.math__lines--big {
  font-size: calc(var(--f-size-2xl) * var(--math-fit, 1));
  line-height: 2;
  padding: var(--sp-3) 0;
}
</style>

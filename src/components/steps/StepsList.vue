<script setup lang="ts">
/**
 * 计算过程（分步列表）—— 见 docs/05 §2.3.4。
 *
 * ## 每步展示四行
 *
 * ```
 * 步骤 1 · 截面有效高度 h₀                    [复制]
 * h0 = h - as
 * → h0 = 500 - 35
 * = 465 mm
 * Excel: =B2-B3
 * ```
 *
 * ## 按 `group` 分组
 *
 * AI 解析时会用 `group` 把步骤分成「几何参数 / 材料参数 / 承载力验算」等段落。
 * 没分组的归到「计算过程」。分组标题可折叠（步骤多时很需要）。
 */
import { computed, ref } from 'vue'
import { copyText } from '@/utils/clipboard'
import { formatNumber, fullPrecision } from '@/utils/format'
import MathDisplay from '@/components/math/MathDisplay.vue'
import type { StepResult } from '@/types/domain'

const props = defineProps<{
  steps: StepResult[]
}>()

const emit = defineEmits<{
  /** 复制成功（父组件可用来弹 toast） */
  (e: 'copied', text: string): void
}>()

/** 未指定 `group` 的步骤归到这里 */
const DEFAULT_GROUP = '计算过程'

interface Group {
  name: string
  steps: StepResult[]
}

/** 分组（保持首次出现的顺序，不排序） */
const groups = computed<Group[]>(() => {
  const map = new Map<string, StepResult[]>()
  for (const s of props.steps) {
    const g = s.group?.trim() || DEFAULT_GROUP
    const arr = map.get(g)
    if (arr) arr.push(s)
    else map.set(g, [s])
  }
  return [...map.entries()].map(([name, steps]) => ({ name, steps }))
})

/** 折叠状态：`group → 是否展开`。默认全展开 */
const expanded = ref<Record<string, boolean>>({})

function isExpanded(name: string): boolean {
  return expanded.value[name] !== false
}

function toggleGroup(name: string): void {
  expanded.value = { ...expanded.value, [name]: !isExpanded(name) }
}

/** 单个步骤的复制文本（含代入式与 Excel 公式，便于贴进计算书） */
function stepText(s: StepResult): string {
  const lines = [`${s.label}（${s.symbol}）`, `${s.symbol} = ${s.expression}`]
  if (s.substitutedExpression) lines.push(s.substitutedExpression)
  lines.push(`= ${formatNumber(s.value)}${s.unit ? ` ${s.unit}` : ''}`)
  if (s.excelFormula) lines.push(`Excel: ${s.excelFormula}`)
  return lines.join('\n')
}

async function onCopy(s: StepResult): Promise<void> {
  const text = stepText(s)
  if (await copyText(text)) emit('copied', text)
}

/** 复制全部步骤 */
async function onCopyAll(): Promise<void> {
  const text = groups.value
    .map((g) => [g.name, ...g.steps.map(stepText)].join('\n\n'))
    .join('\n\n')
  if (await copyText(text)) emit('copied', text)
}
</script>

<template>
  <div class="steps">
    <div class="steps__toolbar">
      <span class="steps__count">共 {{ steps.length }} 步</span>
      <button class="steps__link" type="button" @click="onCopyAll()">复制全部</button>
    </div>

    <section v-for="g in groups" :key="g.name" class="steps__group">
      <button
        class="steps__group-head"
        type="button"
        :aria-expanded="isExpanded(g.name)"
        @click="toggleGroup(g.name)"
      >
        <span class="steps__caret" aria-hidden="true">{{ isExpanded(g.name) ? '▾' : '▸' }}</span>
        <span class="steps__group-name">{{ g.name }}</span>
        <span class="steps__group-count">{{ g.steps.length }}</span>
      </button>

      <ol v-show="isExpanded(g.name)" class="steps__list">
        <li v-for="(s, i) in g.steps" :key="`${s.symbol}-${i}`" class="step">
          <div class="step__head">
            <span class="step__label">{{ s.label }}</span>
            <code class="step__symbol">{{ s.symbol }}</code>
            <button class="steps__link step__copy" type="button" @click="onCopy(s)">复制</button>
          </div>

          <div class="step__lines">
            <!-- 原始公式（二维排版） -->
            <MathDisplay
              class="step__math step__math--formula"
              :expression="s.expression"
              size="sm"
            />

            <!-- 代入数值后的公式（引擎已生成，可能为空） -->
            <MathDisplay
              v-if="s.substitutedExpression"
              class="step__math step__math--sub"
              :expression="s.substitutedExpression"
              size="sm"
            />

            <!-- 结果 -->
            <div class="step__line step__line--result" :title="fullPrecision(s.value)">
              = {{ formatNumber(s.value) }}
              <span v-if="s.unit" class="step__unit">{{ s.unit }}</span>
            </div>

            <!-- Excel 单元格引用式 -->
            <div v-if="s.excelFormula" class="step__line step__line--excel">
              <span class="step__excel-key">Excel:</span>
              <code class="step__excel-val">{{ s.excelFormula }}</code>
            </div>
          </div>
        </li>
      </ol>
    </section>
  </div>
</template>

<style scoped>
.steps__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--sp-2);
}

.steps__count {
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

/* 文字按钮（规范：卡片内操作用文字按钮，不用实心按钮） */
.steps__link {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.steps__link:hover {
  text-decoration: underline;
}

.steps__group + .steps__group {
  margin-top: var(--sp-3);
}

.steps__group-head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  padding: var(--sp-2) 0;
  border: none;
  border-bottom: var(--hairline) solid var(--c-divider);
  background: transparent;
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
  text-align: left;
}

.steps__group-head:hover {
  color: var(--c-text);
}

.steps__caret {
  width: 12px;
  flex-shrink: 0;
}

.steps__group-name {
  flex: 1;
}

.steps__group-count {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.steps__list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.step {
  padding: var(--sp-3) 0;
  border-bottom: var(--hairline) solid var(--c-divider);
}

.step:last-child {
  border-bottom: none;
}

.step__head {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
  margin-bottom: var(--sp-1);
}

.step__label {
  flex: 1;
  min-width: 0;
  font-size: var(--f-size-sm);
  color: var(--c-text);
}

.step__symbol {
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.step__copy {
  flex-shrink: 0;
}

.step__lines {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.step__line {
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  line-height: 1.6;
  word-break: break-all;
}

/* 二维排版（替代等宽原文） */
.step__math {
  max-width: 100%;
}
.step__math--formula :deep(.math__sym) {
  color: var(--c-text-2);
}
.step__math--sub {
  opacity: 0.85;
}

.step__line--formula {
  color: var(--c-text-2);
}

.step__line--sub {
  color: var(--c-text-3);
}

.step__line--result {
  color: var(--c-text);
  font-weight: 600;
}

.step__unit {
  font-family: var(--f-sans);
  font-weight: 400;
  color: var(--c-text-3);
  margin-left: var(--sp-1);
}

.step__line--excel {
  display: flex;
  gap: var(--sp-1);
  color: var(--c-text-3);
}

.step__excel-key {
  flex-shrink: 0;
}

.step__excel-val {
  min-width: 0;
  word-break: break-all;
}
</style>

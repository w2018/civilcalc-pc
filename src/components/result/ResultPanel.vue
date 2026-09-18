<script setup lang="ts">
/**
 * 结果面板 —— 见 docs/05-项目开发方案.md §2.3.4。
 *
 * 组成（自上而下）：
 *
 * 1. **主结果**（大字号 + 单位 + 复制）
 * 2. **告警**（非致命提示，如"结果仅供参考"）
 * 3. **多结果四列表**（仅当 `outputs.length > 1`）
 * 4. **分支卡片**（仅当 `branches` 非空）
 *
 * 错误态（求值失败）也在这里展示 —— 结果区是用户视线的落点，
 * 把错误放在参数区上方反而容易被忽略。
 *
 * ## 单输出不显示多结果表
 *
 * `EvalResult.outputs` 对单段公式也有**一个**元素（`symbol = null`），
 * 但那时表格只有一行、与主结果重复，所以按 `length > 1` 判断。
 */
import { computed, ref, watch } from 'vue'
import { copyText } from '@/utils/clipboard'
import {
  evalErrorMessage,
  type EvalError,
  type EvalResult,
  type FormulaSchema,
} from '@/types/domain'
import { errorMessage as commandErrorMessage, type CommandError } from '@/types/error'
import { formatNumber, fullPrecision } from '@/utils/format'
import { mathLayoutResultLine } from '@/api/display'
import type { MathLayout } from '@/types/math'
import MathDisplay from '@/components/math/MathDisplay.vue'
import BranchCard from './BranchCard.vue'
import MultiResultTable from './MultiResultTable.vue'
import type { EvalStatus } from '@/composables/useEval'

const props = defineProps<{
  schema: FormulaSchema | null
  result: EvalResult | null
  status: EvalStatus
  evalError?: EvalError | null
  commandError?: CommandError | null
}>()

const emit = defineEmits<{
  (e: 'copied', text: string): void
}>()

const isLoading = computed(() => props.status === 'loading')

/** 错误文案（求值错误优先 —— 它更具体） */
const errorText = computed(() => {
  if (props.evalError) return evalErrorMessage(props.evalError)
  if (props.commandError) return commandErrorMessage(props.commandError)
  return ''
})

const hasError = computed(() => errorText.value.length > 0)

/** 主结果文案 */
const primaryText = computed(() => {
  const r = props.result
  if (!r) return ''
  return formatNumber(r.primary)
})

const primaryFull = computed(() => {
  const r = props.result
  return r ? fullPrecision(r.primary) : ''
})

const resultUnit = computed(() => props.schema?.resultUnit?.trim() ?? '')

/** 多结果：仅在真有多段时显示表格 */
const showMulti = computed(() => (props.result?.outputs.length ?? 0) > 1)

/**
 * 多结果的二维结果行 `(x, y) = (1, 2)`。
 *
 * 用 `mathLayoutResultLine` 取节点串，再包成单行 `MathLayout` 交给
 * `MathDisplay`。值由**调用方**格式化（`mathLayoutResultLine` 不管精度口径，
 * 见 `types/math.ts` 的 `ResultPair`）。
 */
const resultLineLayout = ref<MathLayout | null>(null)

const resultPairs = computed(() => {
  const r = props.result
  if (!r) return []
  return r.outputs.map((o, i) => ({
    symbol: o.symbol ?? props.schema?.resultSymbol ?? `#${i + 1}`,
    value: formatNumber(o.value),
  }))
})

watch(
  resultPairs,
  async (pairs) => {
    if (pairs.length <= 1) {
      resultLineLayout.value = null
      return
    }
    try {
      const nodes = await mathLayoutResultLine(pairs)
      resultLineLayout.value = { lines: [{ symbol: null, nodes, mark: null }], warnings: [] }
    } catch (e) {
      console.warn('[ResultPanel] 结果行排版失败，回落表格', e)
      resultLineLayout.value = null
    }
  },
  { immediate: true },
)

/** 分支的「含义/单位」查表 */
const outputMeta = computed(() => props.schema?.resultOutputs ?? [])

/**
 * 分支的单位。
 *
 * ⚠️ 只能按 `label` 猜 —— `EvalBranch` 里没有 `symbol` 字段，
 * 而 `resultOutputs` 是按 `symbol` 索引的。关联不上就返回 `null`
 * （宁可不显示单位，也不要张冠李戴）。
 */
function branchUnit(symbol: string): string | null {
  return outputMeta.value.find((o) => o.symbol === symbol)?.unit ?? null
}

/** 复制主结果（数值 + 单位） */
async function copyPrimary(): Promise<void> {
  const unit = resultUnit.value
  const text = unit ? `${primaryText.value} ${unit}` : primaryText.value
  if (await copyText(text)) emit('copied', text)
}

/** 复制全部结果（多结果时用） */
async function copyAll(): Promise<void> {
  const r = props.result
  if (!r) return
  const lines = r.outputs.map((o, i) => {
    const sym = o.symbol ?? props.schema?.resultSymbol ?? `#${i + 1}`
    const unit = outputMeta.value.find((m) => m.symbol === sym)?.unit ?? ''
    return `${sym} = ${formatNumber(o.value)}${unit ? ` ${unit}` : ''}`
  })
  const text = lines.join('\n')
  if (await copyText(text)) emit('copied', text)
}

/** 复制错误文案（用户报错时需要把原文发给开发者） */
async function copyError(): Promise<void> {
  if (await copyText(errorText.value)) emit('copied', errorText.value)
}
</script>

<template>
  <div class="result">
    <!-- 加载中：顶部细条，不遮内容（避免每次改参数都闪一下骨架） -->
    <div v-if="isLoading" class="result__loading" role="status" aria-label="计算中" />

    <!-- 错误态 -->
    <div v-if="hasError" class="card card--error" role="alert">
      <div class="card__head">
        <span class="card__title card__title--error">
          <span aria-hidden="true">⚠</span> 无法计算
        </span>
        <button class="link" type="button" @click="copyError()">复制错误</button>
      </div>
      <p class="error__text">{{ errorText }}</p>
    </div>

    <!-- 主结果 -->
    <div v-else-if="result" class="card">
      <div class="card__head">
        <span class="card__title">主结果</span>
        <button class="link" type="button" @click="copyPrimary()">复制结果</button>
      </div>
      <div class="primary" :title="primaryFull">
        <span class="primary__value">{{ primaryText }}</span>
        <span v-if="resultUnit" class="primary__unit">{{ resultUnit }}</span>
      </div>
    </div>

    <!-- 告警 -->
    <div v-if="result && result.warnings.length > 0" class="card card--warn">
      <div class="card__head">
        <span class="card__title card__title--warn">
          <span aria-hidden="true">⚠</span> {{ result.warnings.length }} 条告警
        </span>
      </div>
      <ul class="warn__list">
        <li v-for="(w, i) in result.warnings" :key="i" class="warn__item">{{ w }}</li>
      </ul>
    </div>

    <!-- 多结果四列表 -->
    <div v-if="showMulti && result" class="card">
      <div class="card__head">
        <span class="card__title">全部结果</span>
        <button class="link" type="button" @click="copyAll()">复制全部</button>
      </div>
      <MathDisplay
        v-if="resultLineLayout"
        class="result__line"
        :layout="resultLineLayout"
        size="lg"
        :fullscreen="false"
      />
      <MultiResultTable
        :outputs="result.outputs"
        :result-outputs="outputMeta"
        :result-symbol="schema?.resultSymbol"
        :result-name="schema?.resultName"
        :result-unit="schema?.resultUnit"
      />
    </div>

    <!-- 分支 -->
    <template v-if="result && result.branches.length > 0">
      <BranchCard
        v-for="(b, i) in result.branches"
        :key="`${b.label}-${i}`"
        :branch="b"
        :unit="branchUnit(b.label)"
      />
    </template>
  </div>
</template>

<style scoped>
.result {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

/* 顶部细条：不占布局高度（absolute），避免内容跳动 */
.result__loading {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: var(--c-primary);
  animation: pulse 1s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 0.3;
  }
  50% {
    opacity: 1;
  }
}

/* 卡片：白底 + 圆角 10px + **无阴影无描边**（规范） */
.card {
  padding: var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.card--error {
  background: var(--c-surface);
  border: var(--hairline) solid var(--c-danger);
}

.card--warn {
  background: var(--c-surface-2);
}

.card__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
  margin-bottom: var(--sp-2);
}

.card__title {
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.card__title--error {
  color: var(--c-danger);
}

.card__title--warn {
  color: var(--c-warning);
}

/* 文字按钮（规范：卡片内操作用文字按钮） */
.link {
  flex-shrink: 0;
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.link:hover {
  text-decoration: underline;
}

/* 主结果：24px + 600（规范） */
.primary {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
}

.primary__value {
  font-family: var(--f-mono);
  font-size: var(--f-size-2xl);
  font-weight: 600;
  /* 需求 7：结果要有字体颜色 —— 用主色与正文区分，
     一屏里数字很多，纯黑的结果值不容易一眼抓到 */
  color: var(--c-primary);
  word-break: break-all;
}

.primary__unit {
  font-size: var(--f-size-base);
  color: var(--c-text-3);
}

/* 多结果二维结果行 */
.result__line {
  margin-bottom: var(--sp-3);
  padding: var(--sp-2) 0;
  border-bottom: var(--hairline) solid var(--c-divider);
}

.error__text {
  margin: 0;
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  color: var(--c-danger);
  word-break: break-word;
  white-space: pre-wrap;
}

.warn__list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.warn__item {
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
  line-height: 1.6;
}

.warn__item::before {
  content: '⚠ ';
  color: var(--c-warning);
}
</style>

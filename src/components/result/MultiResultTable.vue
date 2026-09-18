<script setup lang="ts">
/**
 * 多结果表格（四列：符号 / 含义 / 数值 / 单位）—— 见 docs/05 §2.3.4。
 *
 * ## 「含义」与「单位」从哪来
 *
 * `EvalResult.outputs` 只带 `symbol` 与 `value`；含义与单位在
 * `schema.resultOutputs` 里（AI 解析时生成的元数据）。
 * 这里按 `symbol` 关联；关联不上时留空（**不编造**）。
 *
 * 单输出公式的 `outputs` 里那一个元素 `symbol = null`，
 * 此时用 schema 的 `resultSymbol` / `resultName` / `resultUnit` 兜底。
 *
 * ## 不用千分位、数值列右对齐
 *
 * 见 `utils/format.ts`。数值列右对齐便于纵向比较量级。
 */
import { computed } from 'vue'
import { formatNumber, fullPrecision } from '@/utils/format'
import type { EvalOutput, ResultOutput } from '@/types/domain'

const props = defineProps<{
  outputs: EvalOutput[]
  /** schema 里的输出元数据（含义 / 单位） */
  resultOutputs: ResultOutput[]
  /** 单输出公式的兜底信息 */
  resultSymbol?: string
  resultName?: string
  resultUnit?: string | null
}>()

interface Row {
  symbol: string
  name: string
  value: number
  unit: string
  /** 是否有可展示的单位（空串表示无量纲，显示 `—`） */
  hasUnit: boolean
}

const rows = computed<Row[]>(() =>
  props.outputs.map((o) => {
    const sym = o.symbol ?? null
    const meta = sym ? props.resultOutputs.find((r) => r.symbol === sym) : undefined

    const unit = (meta?.unit ?? (sym ? null : props.resultUnit) ?? '').trim()
    return {
      symbol: sym ?? props.resultSymbol ?? '',
      name: meta?.name ?? (sym ? '' : (props.resultName ?? '')),
      value: o.value,
      unit,
      hasUnit: unit.length > 0,
    }
  }),
)
</script>

<template>
  <div class="multi">
    <table class="multi__table">
      <thead>
        <tr>
          <th class="multi__th multi__th--symbol">符号</th>
          <th class="multi__th">含义</th>
          <th class="multi__th multi__th--num">数值</th>
          <th class="multi__th multi__th--unit">单位</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(r, i) in rows" :key="`${r.symbol}-${i}`" class="multi__tr">
          <td class="multi__td multi__td--symbol">{{ r.symbol }}</td>
          <td class="multi__td">{{ r.name }}</td>
          <td class="multi__td multi__td--num" :title="fullPrecision(r.value)">
            {{ formatNumber(r.value) }}
          </td>
          <td class="multi__td multi__td--unit">{{ r.hasUnit ? r.unit : '—' }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.multi {
  overflow-x: auto;
}

.multi__table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--f-size-base);
}

/* 表头：底 `--c-surface-2`、字 `--c-text-3`（规范） */
.multi__th {
  padding: var(--sp-2) var(--sp-3);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  font-weight: 500;
  text-align: left;
  white-space: nowrap;
}

.multi__th--symbol {
  width: 96px;
}

.multi__th--num {
  width: 160px;
  text-align: right;
}

.multi__th--unit {
  width: 96px;
}

/* 仅横向分割线（规范：不用全边框、无斑马纹） */
.multi__td {
  padding: var(--sp-3);
  border-bottom: var(--hairline) solid var(--c-divider);
  color: var(--c-text);
  vertical-align: middle;
}

.multi__tr:last-child .multi__td {
  border-bottom: none;
}

.multi__td--symbol {
  font-family: var(--f-mono);
  color: var(--c-text-2);
}

.multi__td--num {
  text-align: right;
  font-family: var(--f-mono);
  font-weight: 600;
}

.multi__td--unit {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}
</style>

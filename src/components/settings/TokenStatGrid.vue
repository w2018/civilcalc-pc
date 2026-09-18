<script setup lang="ts">
/**
 * TokenStatGrid —— 用量统计的对称栅格。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「TokenStatGrid」。
 *
 * ## 为什么固定 6 格（3×2）而不是自适应
 *
 * 「对称」是这一块的设计要求：输入/输出/缓存/思考四个维度
 * 必须一眼能横向比较。列数随宽度变化会让同一组数字换行到不同位置，
 * 对比就断了。窄屏改成 2×3，但**始终是矩形**，不留半行。
 *
 * ## 思考 token 的调优提示放在这里
 *
 * 思考 token 远多于正文时，用户的钱花在了「想」而不是「答」上 ——
 * 这是唯一值得主动提示的异常。阈值取 3 倍（经验值）：
 * 正常推理任务的思考量很少超过正文的 3 倍。
 */
import { computed } from 'vue'
import type { UsageTotals } from '@/types/system'

const props = withDefaults(
  defineProps<{
    totals: UsageTotals
    /** 缓存命中率的分母用「输入 token」 */
    showCacheRate?: boolean
  }>(),
  { showCacheRate: true },
)

/** 千分位（统计数字，与工程数值格式化分开处理） */
function n(v: number): string {
  return v.toLocaleString('zh-CN')
}

const cacheRate = computed(() => {
  const p = props.totals.totalPrompt
  if (p <= 0) return null
  return (props.totals.totalCached / p) * 100
})

const cells = computed(() => [
  { key: 'calls', label: '调用次数', value: n(props.totals.callCount), unit: '次' },
  { key: 'prompt', label: '输入 Token', value: n(props.totals.totalPrompt), unit: '' },
  { key: 'completion', label: '输出 Token', value: n(props.totals.totalCompletion), unit: '' },
  { key: 'total', label: '合计 Token', value: n(props.totals.totalAll), unit: '' },
  {
    key: 'cached',
    label: '缓存命中',
    value: n(props.totals.totalCached),
    unit: cacheRate.value === null ? '' : `（${cacheRate.value.toFixed(1)}%）`,
  },
  { key: 'reasoning', label: '思考 Token', value: n(props.totals.totalReasoning), unit: '' },
])

/** 思考量是否明显偏高（> 正文 3 倍且绝对值不小） */
const thinkingHeavy = computed(
  () => props.totals.totalReasoning > 1000 && props.totals.totalReasoning > props.totals.totalCompletion * 3,
)
</script>

<template>
  <div class="grid-wrap">
    <div class="grid">
      <div v-for="c in cells" :key="c.key" class="cell">
        <span class="cell__label">{{ c.label }}</span>
        <span class="cell__value">
          {{ c.value }}
          <em v-if="c.unit" class="cell__unit">{{ c.unit }}</em>
        </span>
      </div>
    </div>

    <p v-if="thinkingHeavy" class="tip">
      思考 token 明显多于正文输出（{{ n(totals.totalReasoning) }} vs
      {{ n(totals.totalCompletion) }}）—— 如果不需要推理过程，可以把模型的思考强度调低，能省不少额度。
    </p>
  </div>
</template>

<style scoped>
.grid-wrap {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--sp-2);
}

@media (max-width: 1100px) {
  .grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

.cell {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.cell__label {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.cell__value {
  color: var(--c-text);
  font-size: var(--f-size-xl);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.cell__unit {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-style: normal;
  font-weight: 400;
}

.tip {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}
</style>

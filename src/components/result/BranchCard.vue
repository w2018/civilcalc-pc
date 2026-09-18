<script setup lang="ts">
/**
 * 分支结果卡片 —— 见 docs/05 §2.3.4。
 *
 * ## 「不适用」必须附原因
 *
 * 条件不满足时显示 `不适用` **加 `原因：{condition}`**。
 * 只写「不适用」而不说条件是什么，用户会以为程序坏了 ——
 * 而实际上它是在正确工作（条件分支本就不该有值）。
 *
 * ⚠️ **绝不用 `NaN` 字符串替代**（文档硬约束）。
 */
import { computed } from 'vue'
import { formatNumber, fullPrecision, NOT_APPLICABLE_TEXT } from '@/utils/format'
import type { EvalBranch } from '@/types/domain'

const props = defineProps<{
  branch: EvalBranch
  /** 该分支对应的单位（来自 `resultOutputs`，取不到则为空） */
  unit?: string | null
}>()

const applicable = computed(() => props.branch.applicable)

/** 适用时才有数值；不适用时 `value` 通常为 `null` */
const valueText = computed(() => {
  if (!applicable.value) return NOT_APPLICABLE_TEXT
  const v = props.branch.value
  if (v === null || v === undefined || !Number.isFinite(v)) return NOT_APPLICABLE_TEXT
  return formatNumber(v)
})

const fullText = computed(() => {
  const v = props.branch.value
  return v === null || v === undefined ? '' : fullPrecision(v)
})
</script>

<template>
  <div class="branch" :class="{ 'branch--na': !applicable }">
    <div class="branch__head">
      <span class="branch__label">{{ branch.label }}</span>
      <span class="branch__tag" :class="applicable ? 'branch__tag--ok' : 'branch__tag--na'">
        {{ applicable ? '适用' : NOT_APPLICABLE_TEXT }}
      </span>
    </div>

    <div v-if="applicable" class="branch__value" :title="fullText">
      {{ valueText }}
      <span v-if="unit" class="branch__unit">{{ unit }}</span>
    </div>
    <div v-else class="branch__value branch__value--na">{{ NOT_APPLICABLE_TEXT }}</div>

    <!-- 不适用时必须给出原因；适用时条件也有参考价值，一并展示 -->
    <div v-if="branch.condition" class="branch__reason">
      <span class="branch__reason-key">{{ applicable ? '条件' : '原因' }}：</span>
      <span class="branch__reason-val">{{ branch.condition }}</span>
    </div>
  </div>
</template>

<style scoped>
.branch {
  padding: var(--sp-3) var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface);
  border: var(--hairline) solid var(--c-divider);
}

/* 不适用：灰底 + 灰字（规范） */
.branch--na {
  background: var(--c-surface-2);
  border-color: transparent;
}

.branch__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.branch__label {
  flex: 1;
  min-width: 0;
  font-size: var(--f-size-base);
  font-weight: 600;
  color: var(--c-text);
}

.branch--na .branch__label {
  color: var(--c-text-3);
  font-weight: 400;
}

.branch__tag {
  flex-shrink: 0;
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  font-size: var(--f-size-xs);
}

.branch__tag--ok {
  background: var(--c-primary-light);
  color: var(--c-primary);
}

.branch__tag--na {
  background: var(--c-surface-hover);
  color: var(--c-text-3);
}

.branch__value {
  margin-top: var(--sp-1);
  font-family: var(--f-mono);
  font-size: var(--f-size-xl);
  font-weight: 600;
  color: var(--c-text);
}

.branch__value--na {
  font-size: var(--f-size-base);
  font-weight: 400;
  color: var(--c-text-3);
}

.branch__unit {
  margin-left: var(--sp-1);
  font-family: var(--f-sans);
  font-size: var(--f-size-sm);
  font-weight: 400;
  color: var(--c-text-3);
}

.branch__reason {
  margin-top: var(--sp-1);
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  word-break: break-word;
}

.branch__reason-key {
  flex-shrink: 0;
}

.branch__reason-val {
  font-family: var(--f-mono);
}
</style>

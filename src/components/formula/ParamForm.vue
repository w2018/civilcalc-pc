<script setup lang="ts">
/**
 * 参数表单 —— 按 `variables` 动态渲染 N 个 [`ParamField`]。
 *
 * 见 docs/05-项目开发方案.md §2.3.4 / §2.3.5。
 *
 * ## 列数自适应
 *
 * 用 `grid-template-columns: repeat(auto-fill, minmax(360px, 1fr))`：
 *
 * | 可用宽度 | 列数 |
 * |---|---|
 * | < 760px | 1 列 |
 * | 760 ~ 1119px | 2 列 |
 * | ≥ 1120px | 3 列 |
 *
 * 比按断点写死更省事，也不会出现「参数框被压到 200px、运算符按钮换行」。
 * `360px` 的下限是量出来的：`输入框 + 8 个运算符按钮` 需要约 340px。
 *
 * ## 为什么不用 `el-form`
 *
 * Element 的 `el-form-item` 自带 label 位置、必填星号、校验消息等一整套
 * 布局，与「底线输入」冲突（它的错误消息位置、label 宽度都是固定的）。
 * 覆盖成本高于自己写 —— 且这里只有「文本输入」一种控件。
 */
import { ref, watch } from 'vue'
import ParamField from './ParamField.vue'
import type { FormulaVar } from '@/types/domain'

const props = defineProps<{
  variables: FormulaVar[]
  /** `symbol → 原始输入` */
  values: Record<string, string>
  /** `symbol → 失焦求值显示值` */
  evaluated: Record<string, string>
  /** `symbol → 错误文案` */
  errors: Record<string, string>
  /** 需要聚焦的字段（后端 `Domain` 错误时非空） */
  focusKey?: string | null
}>()

const emit = defineEmits<{
  (e: 'change', symbol: string, raw: string): void
  (e: 'blur', symbol: string): void
  (e: 'operator', symbol: string, text: string): void
}>()

/** 每个字段的组件实例（用于聚焦） */
const fieldRefs = ref<Record<string, { focus: () => void } | null>>({})

function setFieldRef(symbol: string, el: unknown): void {
  fieldRefs.value[symbol] = (el as { focus: () => void } | null) ?? null
}

/** 聚焦指定字段（`focusKey` 变化时自动触发） */
function focusField(symbol: string): void {
  fieldRefs.value[symbol]?.focus()
}

watch(
  () => props.focusKey,
  (key) => {
    if (key) focusField(key)
  },
  { flush: 'post' },
)

defineExpose({ focusField })
</script>

<template>
  <div class="form">
    <p v-if="variables.length === 0" class="form__empty">该公式没有参数，可直接查看结果。</p>

    <div v-else class="form__grid">
      <ParamField
        v-for="v in variables"
        :key="v.symbol"
        :ref="(el) => setFieldRef(v.symbol, el)"
        :variable="v"
        :model-value="values[v.symbol] ?? ''"
        :evaluated="evaluated[v.symbol] ?? null"
        :error="errors[v.symbol] ?? null"
        @update:model-value="(raw: string) => emit('change', v.symbol, raw)"
        @blur="emit('blur', v.symbol)"
        @operator="(text: string) => emit('operator', v.symbol, text)"
      />
    </div>
  </div>
</template>

<style scoped>
.form__empty {
  margin: 0;
  padding: var(--sp-4) 0;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.form__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
  gap: var(--sp-5) var(--sp-5);
}
</style>

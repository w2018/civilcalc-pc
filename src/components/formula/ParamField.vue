<script setup lang="ts">
/**
 * 参数输入框（微信风底线输入）—— 见 docs/05-项目开发方案.md §2.3.4。
 *
 * ```
 * ┌─────────────────────────────────────────────────────┐
 * │  混凝土轴心抗压强度设计值  fc              N/mm²     │  ← desc + symbol + 单位
 * │                                                      │
 * │  14.3                                    [+ - * /]   │  ← 值区 + 运算符快插
 * │  ────────────────────────────────────────────────    │  ← 底线 1px
 * │  范围：≥ 0                                           │  ← 辅助行
 * └─────────────────────────────────────────────────────┘
 * ```
 *
 * ## 「显示原文」与「显示求值结果」的切换
 *
 * 参数框支持四则运算。失焦时把算式求值（`12*3` → `36`）并显示结果；
 * **重新聚焦时恢复原文**（`12*3`），否则用户想改成 `12*4` 得从头敲。
 *
 * 这个切换由父组件通过 `evaluated` prop 提供，本组件只管
 * 「聚焦时用 `modelValue`，未聚焦时用 `evaluated ?? modelValue`」。
 *
 * ## 自动聚焦
 *
 * 父组件通过模板 ref 调 `focus()` —— 后端返回 `Domain` 错误时
 * 需要把对应字段标红并聚焦。见 `FormulaView` 里的 `watch(focusedErrorKey)`。
 */
import { computed, ref, useTemplateRef } from 'vue'
import OperatorQuickInsert from './OperatorQuickInsert.vue'
import type { FormulaVar } from '@/types/domain'

const props = withDefaults(
  defineProps<{
    variable: FormulaVar
    /** 用户原始输入（可为算式） */
    modelValue: string
    /** 失焦后的求值显示值；无则显示原文 */
    evaluated?: string | null
    /** 字段级错误（非空即错误态） */
    error?: string | null
  }>(),
  { evaluated: null, error: null },
)

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'blur'): void
  (e: 'operator', text: string): void
}>()

const inputRef = useTemplateRef<HTMLInputElement>('input')

/** 是否正在聚焦 */
const focused = ref(false)

/**
 * 输入框实际显示的文本。
 *
 * ⚠️ 聚焦时**必须**用原文 —— 否则用户点进 `36` 想改成 `36.5`，
 * 看到的是 `12*3`，改完存进去的是 `12*3.5`（表达式被破坏）。
 */
const display = computed(() => {
  if (focused.value) return props.modelValue
  return props.evaluated ?? props.modelValue
})

/**
 * 当前显示的是**求值结果**而不是用户原文（如输入 `12*3`、显示 `36`）。
 *
 * 需求 7：这种值要用另一种颜色标出来 ——
 * 否则用户看不出「我打的算式被算成了什么」，
 * 也分不清 `36` 是自己填的还是算出来的。
 */
const showingComputed = computed(() => !focused.value && Boolean(props.evaluated))

/** 辅助行文案：优先显示范围，其次默认值 */
const hint = computed(() => {
  const { min, max, default: def } = props.variable
  const hasMin = min !== null && min !== undefined
  const hasMax = max !== null && max !== undefined

  if (hasMin && hasMax) return `范围：${min} ~ ${max}`
  if (hasMin) return `范围：≥ ${min}`
  if (hasMax) return `范围：≤ ${max}`
  if (def !== null && def !== undefined) return `默认：${def}`
  return ''
})

function onInput(e: Event): void {
  emit('update:modelValue', (e.target as HTMLInputElement).value)
}

function onFocus(): void {
  focused.value = true
}

function onBlur(): void {
  focused.value = false
  emit('blur')
}

/** 运算符快插：追加到**原文**末尾（不是求值显示值） */
function onInsert(text: string): void {
  emit('operator', text)
  // 快插后应保持聚焦，让用户能连续点
  inputRef.value?.focus()
}

/**
 * 聚焦并滚动到可视区（父组件在 `Domain` 错误时调用）。
 *
 * `scrollIntoView` 用 `block: 'center'` —— 参数多时出错字段可能在
 * 屏幕外，滚到顶部容易让用户看不到上下文。
 */
function focus(): void {
  inputRef.value?.focus()
  inputRef.value?.scrollIntoView({ block: 'center', behavior: 'smooth' })
}

defineExpose({ focus })
</script>

<template>
  <div class="field" :class="{ 'field--error': !!error }">
    <div class="field__head">
      <span class="field__desc">{{ variable.desc }}</span>
      <code class="field__symbol">{{ variable.symbol }}</code>
      <span v-if="variable.unit" class="field__unit">{{ variable.unit }}</span>
      <span v-if="variable.required" class="field__required" aria-label="必填">*</span>
    </div>

    <div class="field__row">
      <input
        ref="input"
        class="field__input"
        :class="{ 'field__input--computed': showingComputed }"
        type="text"
        inputmode="decimal"
        autocomplete="off"
        spellcheck="false"
        :value="display"
        :aria-invalid="!!error"
        :aria-describedby="error ? `err-${variable.symbol}` : undefined"
        @input="onInput"
        @focus="onFocus"
        @blur="onBlur"
      />
      <OperatorQuickInsert @insert="onInsert" />
    </div>

    <div v-if="error" :id="`err-${variable.symbol}`" class="field__error" role="alert">
      <span class="field__error-icon" aria-hidden="true">⚠</span>
      <span class="field__error-text">{{ error }}</span>
    </div>
    <div v-else-if="hint" class="field__hint">{{ hint }}</div>
  </div>
</template>

<style scoped>
.field {
  display: flex;
  flex-direction: column;
}

.field__head {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
  margin-bottom: var(--sp-1);
}

.field__desc {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--f-size-sm);
  color: var(--c-text);
}

.field__symbol {
  flex-shrink: 0;
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.field__unit {
  flex-shrink: 0;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

.field__required {
  flex-shrink: 0;
  color: var(--c-danger);
  font-size: var(--f-size-sm);
}

/* 输入框框体（需求 9：原来是「只用底线」的极简风，框体不明显、不好辨识）。
   现在给整行一个可见的圆角框，快捷符号按钮留在框内右侧。
   三态靠**边框颜色 + 主色光圈**表达，而不是原来那条会变粗的底线。 */
.field__row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  height: 40px;
  padding: 0 var(--sp-1) 0 var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-btn);
  background: var(--c-surface);
  transition:
    border-color 0.12s ease,
    box-shadow 0.12s ease;
}

.field__row:hover {
  border-color: var(--c-text-3);
}

.field:focus-within .field__row {
  border-color: var(--c-primary);
  box-shadow: 0 0 0 2px var(--c-primary-light);
}

.field--error .field__row {
  border-color: var(--c-danger);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--c-danger) 18%, transparent);
}

.field__input {
  flex: 1;
  min-width: 0;
  height: 36px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-text);
  font-family: var(--f-mono);
  font-size: var(--f-size-lg);
  outline: none;
}

.field__input::placeholder {
  color: var(--c-text-3);
}

/* 显示的是「算式求值后的结果」而非用户原文 —— 用主色区分（需求 7） */
.field__input--computed {
  color: var(--c-primary);
}

.field__hint,
.field__error {
  margin-top: var(--sp-1);
  font-size: var(--f-size-xs);
  line-height: 1.5;
}

.field__hint {
  color: var(--c-text-3);
}

.field__error {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-1);
  color: var(--c-danger);
}

.field__error-icon {
  flex-shrink: 0;
}

.field__error-text {
  min-width: 0;
  word-break: break-word;
}
</style>

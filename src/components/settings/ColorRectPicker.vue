<script setup lang="ts">
/**
 * ColorRectPicker —— 长方形选色面板。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「ColorRectPicker」（对齐源项目组件名）。
 *
 * ## 为什么是「长方形」而不是圆形色块
 *
 * 圆形色块在 12px 字号下辨识度差、点击热区也小。
 * 长方形色条排列更紧凑，一屏能放完预设色 + 当前值预览。
 *
 * ## 取值约定
 *
 * 值是 `#RRGGBB` 或 `AUTO`（`AUTO` = 交给主题/背景对比度决定）。
 * 非法输入**不写回** —— 只在本地标红，避免把 `#12` 这种值存进偏好。
 */
import { computed, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    /** `#RRGGBB` 或 `AUTO` */
    modelValue: string
    /** 是否允许 `AUTO`（文字色允许；页面底色不允许） */
    allowAuto?: boolean
    /** `AUTO` 的显示文案 */
    autoLabel?: string
    /** 预设色（默认给一套中性 + 微信风色） */
    presets?: string[]
  }>(),
  {
    allowAuto: false,
    autoLabel: '自动',
    presets: () => [
      '#191919',
      '#576b95',
      '#07c160',
      '#fa5151',
      '#fa9d3b',
      '#a66be8',
      '#8c8c8c',
      '#ffffff',
      '#f2f2f2',
      '#e8f8ef',
      '#10301f',
      '#111111',
    ],
  },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: string): void
}>()

/** 本地输入框（允许用户中途输入一半，不立刻写回） */
const draft = ref(props.modelValue)

watch(
  () => props.modelValue,
  (v) => {
    draft.value = v
  },
)

const isValid = computed(() => /^#[0-9a-fA-F]{6}$/.test(draft.value.trim()))

const isAuto = computed(() => props.allowAuto && props.modelValue === 'AUTO')

function pick(hex: string): void {
  emit('update:modelValue', hex)
}

function commitDraft(): void {
  const v = draft.value.trim()
  if (v === 'AUTO' && props.allowAuto) {
    emit('update:modelValue', 'AUTO')
    return
  }
  if (!isValid.value) {
    // 非法值不写回，把输入框拉回当前生效值
    draft.value = props.modelValue
    return
  }
  emit('update:modelValue', v.toUpperCase())
}

/** 原生取色器：给「想要的颜色预设里没有」的场景 */
function onNative(e: Event): void {
  const v = (e.target as HTMLInputElement).value
  draft.value = v
  emit('update:modelValue', v.toUpperCase())
}
</script>

<template>
  <div class="picker">
    <div class="picker__row">
      <button
        v-if="allowAuto"
        class="picker__auto"
        :class="{ 'picker__auto--on': isAuto }"
        type="button"
        @click="pick('AUTO')"
      >
        {{ autoLabel }}
      </button>

      <button
        v-for="c in presets"
        :key="c"
        class="picker__swatch"
        :class="{ 'picker__swatch--on': modelValue.toUpperCase() === c.toUpperCase() }"
        type="button"
        :style="{ background: c }"
        :title="c"
        :aria-label="`选择颜色 ${c}`"
        @click="pick(c)"
      />

      <label class="picker__native" title="自定义颜色">
        <input type="color" :value="isValid ? draft : '#000000'" @input="onNative" />
        <span aria-hidden="true">＋</span>
      </label>
    </div>

    <div class="picker__input">
      <el-input
        v-model="draft"
        size="small"
        placeholder="#RRGGBB"
        @blur="commitDraft()"
        @keyup.enter="commitDraft()"
      />
      <span class="picker__cur" :style="{ background: isValid ? draft : 'transparent' }" />
      <span v-if="!isValid" class="picker__bad">格式应为 #RRGGBB</span>
    </div>
  </div>
</template>

<style scoped>
.picker {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.picker__row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  align-items: center;
}

.picker__auto {
  height: 24px;
  padding: 0 var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  background: var(--c-surface);
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-xs);
  cursor: pointer;
}

.picker__auto--on {
  border-color: var(--c-primary);
  background: var(--c-primary-light);
  color: var(--c-primary);
}

/* 长方形色块（不是圆形） */
.picker__swatch {
  width: 32px;
  height: 24px;
  padding: 0;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  cursor: pointer;
}

.picker__swatch--on {
  outline: 2px solid var(--c-primary);
  outline-offset: 1px;
}

.picker__native {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 24px;
  border: var(--hairline) dashed var(--c-divider);
  border-radius: var(--r-sm);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  cursor: pointer;
  overflow: hidden;
}

.picker__native input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: pointer;
}

.picker__input {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.picker__cur {
  width: 24px;
  height: 24px;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
}

.picker__bad {
  color: var(--c-danger);
  font-size: var(--f-size-xs);
}
</style>

<script setup lang="ts">
/**
 * ExportOptionsForm —— 计算书导出选项（7 个章节开关 + 免责声明）。
 *
 * 见 ADR-023（「导出选项」取代「模板」）与 docs/08 §3 组 13。
 *
 * ## 默认全开
 *
 * 用户不勾也能导出一份完整计算书 —— 所以这里不做「至少选一项」的强校验，
 * 只在一个章节都没勾时给**提示**（导出会得到只有封面的空文档）。
 *
 * ## 免责声明空串 = 用内置默认
 *
 * 不要在前端填一份默认文案进去 —— 那样「用户没改」与
 * 「用户手打了和默认一样的内容」就分不清了，后端也无法判断该不该用内置。
 */
import { computed } from 'vue'
import { isEmptySelection, type ExportOptions } from '@/types/report'

const props = defineProps<{
  options: ExportOptions
}>()

const emit = defineEmits<{
  (e: 'update', options: ExportOptions): void
}>()

interface Item {
  key: keyof Omit<ExportOptions, 'disclaimer'>
  label: string
  hint: string
}

const ITEMS: readonly Item[] = [
  { key: 'showCover', label: '封面', hint: '主标题 + 计算器名' },
  { key: 'showFormulaInfo', label: '公式信息', hint: '名称 / 来源 / 领域 / 表达式' },
  { key: 'showParamsTable', label: '参数表', hint: '变量、单位、取值' },
  { key: 'showResultBlock', label: '结果区', hint: '主结果 / 多结果 / 分支' },
  { key: 'showCalculationSteps', label: '分步计算', hint: '逐步代入式' },
  { key: 'showExplanation', label: '公式详解', hint: '理解需求 / 解决方式 / 分步依据' },
  { key: 'showNotes', label: '附注', hint: '设计说明与提醒词' },
] as const

function toggle(key: Item['key'], on: boolean): void {
  emit('update', { ...props.options, [key]: on })
}

function setDisclaimer(v: string): void {
  emit('update', { ...props.options, disclaimer: v })
}

const empty = computed(() => isEmptySelection(props.options))

const allOn = computed(() => ITEMS.every((i) => props.options[i.key]))
</script>

<template>
  <div class="exp">
    <div class="exp__grid">
      <div v-for="i in ITEMS" :key="i.key" class="exp__item">
        <el-checkbox
          :model-value="options[i.key]"
          @update:model-value="(v: boolean | string | number) => toggle(i.key, !!v)"
        >
          {{ i.label }}
        </el-checkbox>
        <span class="exp__hint">{{ i.hint }}</span>
      </div>
    </div>

    <div class="exp__ops">
      <button class="link" type="button" :disabled="allOn" @click="emit('update', { ...options, showCover: true, showFormulaInfo: true, showParamsTable: true, showResultBlock: true, showCalculationSteps: true, showExplanation: true, showNotes: true })">
        全选
      </button>
    </div>

    <p v-if="empty" class="warn">
      一个章节都没勾 —— 导出的文档只有封面。若这是有意的，忽略即可。
    </p>

    <label class="field">
      <span class="field__label">免责声明</span>
      <el-input
        :model-value="options.disclaimer"
        placeholder="留空即使用内置默认文案"
        @update:model-value="setDisclaimer"
      />
      <span class="field__hint">显示在计算书末尾。留空时用内置默认（不在此处硬编码）。</span>
    </label>
  </div>
</template>

<style scoped>
.exp {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.exp__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--sp-2) var(--sp-4);
}

.exp__item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.exp__hint {
  padding-left: var(--sp-5);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.exp__ops {
  display: flex;
  gap: var(--sp-3);
}

.warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.field__label {
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.field__hint {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

.link {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.link:hover:not(:disabled) {
  text-decoration: underline;
}

.link:disabled {
  color: var(--c-text-3);
  cursor: default;
}
</style>

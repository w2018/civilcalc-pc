<script setup lang="ts">
/**
 * BackupSelectionForm —— 备份范围勾选（本地导出与云端上传**共用**）。
 *
 * 见 docs/05-项目开发方案.md「流程 5：备份与恢复」与 `types/backup.ts`。
 *
 * ## 为什么抽成组件而不是各写一份
 *
 * 「本地导出」与「上传云端」的勾选项**完全一样**（后端同一个
 * `BackupSelection`）。各写一份的话，将来加一个类别（比如「公式草稿」）
 * 必然漏掉一处 —— 用户会发现「本地导出的包比云端上传的包少东西」。
 *
 * ## 🔴 图片三档的默认值不是随便定的
 *
 * | 档 | 默认 | 理由 |
 * |---|---|---|
 * | 被公式引用的 | ✅ 勾 | 不勾则公式里的内联图恢复后是「已删除」占位 |
 * | 被对话/历史引用的 | ❌ 不勾 | 体积大、价值低，按需选择 |
 * | 未被引用的 | ❌ 不勾 | 纯占体积 |
 *
 * ## 密钥那一项要单独警告
 *
 * `includeApiKeys` 勾上后**明文**写进备份包（跨端契约如此）。
 * 这里给显式提示，而不是等用户把包传上云才发现。
 */
import { computed } from 'vue'
import type { BackupSelection } from '@/types/backup'

const props = defineProps<{
  selection: BackupSelection
  /** 紧凑模式（塞在弹窗里时用） */
  compact?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:selection', v: BackupSelection): void
}>()

interface Item {
  key: keyof BackupSelection
  label: string
  hint?: string
  /** 危险项（要额外提示） */
  warn?: boolean
  /** 属于图片三档（缩进显示） */
  imageGroup?: boolean
}

const MAIN: readonly Item[] = [
  { key: 'formulas', label: '公式与收藏', hint: '含版本链' },
  { key: 'history', label: '计算历史' },
  { key: 'preferences', label: '偏好设置', hint: '主题 / 提示词 / 提醒词 / 导出选项' },
  { key: 'usageStats', label: 'Token 用量统计' },
  { key: 'llmConfig', label: 'AI 模型配置', hint: '不含密钥' },
  {
    key: 'includeApiKeys',
    label: '模型密钥（明文）',
    hint: '密钥会以明文写进备份包 —— 包放到哪儿，密钥就在哪儿',
    warn: true,
  },
] as const

const IMAGE_ITEMS: readonly Item[] = [
  { key: 'includeFormulaImages', label: '公式引用的图片', hint: '不勾则恢复后内联图变占位' },
  { key: 'includeHistoryImages', label: '历史记录引用的图片' },
  { key: 'includeUnreferencedImages', label: '未被引用的图片' },
] as const

function toggle(key: keyof BackupSelection, on: boolean): void {
  emit('update:selection', { ...props.selection, [key]: on })
}

/** 一个类别都没勾（导出的包是空壳） */
const nothingSelected = computed(
  () =>
    !props.selection.formulas &&
    !props.selection.history &&
    !props.selection.preferences &&
    !props.selection.usageStats &&
    !props.selection.llmConfig &&
    !props.selection.includeFormulaImages &&
    !props.selection.includeHistoryImages &&
    !props.selection.includeUnreferencedImages,
)

/** 勾了密钥 → 给一条更醒目的提示 */
const keyWarning = computed(() => props.selection.includeApiKeys)
</script>

<template>
  <div class="sel" :class="{ 'sel--compact': compact }">
    <div class="sel__grid">
      <label v-for="i in MAIN" :key="i.key" class="item" :class="{ 'item--warn': i.warn }">
        <el-checkbox
          :model-value="selection[i.key]"
          @update:model-value="(v: boolean | string | number) => toggle(i.key, !!v)"
        />
        <span class="item__main">
          <span class="item__label">{{ i.label }}</span>
          <span v-if="i.hint" class="item__hint">{{ i.hint }}</span>
        </span>
      </label>
    </div>

    <div class="sel__group">
      <span class="sel__group-title">图片</span>
      <div class="sel__grid">
        <label v-for="i in IMAGE_ITEMS" :key="i.key" class="item item--sub">
          <el-checkbox
            :model-value="selection[i.key]"
            @update:model-value="(v: boolean | string | number) => toggle(i.key, !!v)"
          />
          <span class="item__main">
            <span class="item__label">{{ i.label }}</span>
            <span v-if="i.hint" class="item__hint">{{ i.hint }}</span>
          </span>
        </label>
      </div>
    </div>

    <p v-if="keyWarning" class="warn">
      已勾选「模型密钥（明文）」—— 备份包里会有可直接使用的 API Key。
      上传到网盘前请确认这个位置是你能接受的。
    </p>

    <p v-if="nothingSelected" class="warn">
      一个类别都没勾 —— 导出的会是一个空备份包（恢复时什么也不会变）。
    </p>
  </div>
</template>

<style scoped>
.sel {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.sel__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--sp-2) var(--sp-4);
}

.sel--compact .sel__grid {
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
}

.item {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  cursor: pointer;
}

.item--sub {
  padding-left: var(--sp-3);
}

.item__main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.item__label {
  color: var(--c-text);
  font-size: var(--f-size-sm);
}

.item--warn .item__label {
  color: var(--c-warning);
}

.item__hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.sel__group {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.sel__group-title {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  font-weight: 600;
}

.warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}
</style>

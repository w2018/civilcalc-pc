<script setup lang="ts">
/**
 * 导出落盘位置选择器（P3-14）。
 *
 * 见 docs/08-IPC契约.md 组 13 的 `DocumentOutputTarget` 四档。
 *
 * | 档位 | 取路径方式 | 自动加序号 |
 * |---|---|---|
 * | `privateCache` | 应用缓存（后端定） | ✅ |
 * | `scopedDownloads` | 系统「下载」（后端定） | ✅ |
 * | `chosenDirectory` | 原生目录对话框 | ✅ |
 * | `chosenFile` | 原生保存对话框 | ❌ |
 *
 * 后两档需要前端用 Tauri dialog 插件取路径；取不到（取消）时**保持上一档**，
 * 不让用户的选择凭空消失。
 */
import { pickDirectory, pickSaveDocx } from '@/api/dialog'
import type { DocumentOutputTarget } from '@/types/report'

const props = defineProps<{
  modelValue: DocumentOutputTarget
  /** 保存对话框的默认文件名（如 `矩形面积.docx`） */
  defaultName: string
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: DocumentOutputTarget): void
}>()

async function choose(kind: DocumentOutputTarget['kind']): Promise<void> {
  if (kind === 'chosenDirectory') {
    const dir = await pickDirectory()
    if (dir) emit('update:modelValue', { kind: 'chosenDirectory', dir })
    return
  }
  if (kind === 'chosenFile') {
    const path = await pickSaveDocx(props.defaultName)
    if (path) emit('update:modelValue', { kind: 'chosenFile', path })
    return
  }
  emit('update:modelValue', { kind } as DocumentOutputTarget)
}

function isActive(kind: DocumentOutputTarget['kind']): boolean {
  return props.modelValue.kind === kind
}
</script>

<template>
  <div class="loc">
    <button
      v-for="opt in ([
        { kind: 'privateCache', label: '应用缓存', hint: '临时，可能被清理' },
        { kind: 'scopedDownloads', label: '系统下载', hint: 'Downloads 目录' },
        { kind: 'chosenDirectory', label: '选择文件夹', hint: '原生对话框' },
        { kind: 'chosenFile', label: '选择文件', hint: '原生保存对话框' },
      ] as const)"
      :key="opt.kind"
      type="button"
      class="loc__item"
      :class="{ 'loc__item--on': isActive(opt.kind) }"
      @click="choose(opt.kind)"
    >
      <span class="loc__label">{{ opt.label }}</span>
      <span class="loc__hint">{{ opt.hint }}</span>
    </button>

    <p v-if="modelValue.kind === 'chosenDirectory'" class="loc__path">
      已选目录：{{ modelValue.dir }}
    </p>
    <p v-else-if="modelValue.kind === 'chosenFile'" class="loc__path">
      已选文件：{{ modelValue.path }}
    </p>
  </div>
</template>

<style scoped>
.loc {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.loc__item {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: var(--sp-2) var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  background: var(--c-surface);
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}
.loc__item--on {
  border-color: var(--c-primary);
  background: var(--c-primary-light);
}

.loc__label {
  font-size: var(--f-size-sm);
  color: var(--c-text);
}
.loc__hint {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

.loc__path {
  margin: 0;
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  color: var(--c-text-2);
  word-break: break-all;
}
[data-theme='dark'] .loc__item--on {
  background: var(--c-primary-light);
}
</style>

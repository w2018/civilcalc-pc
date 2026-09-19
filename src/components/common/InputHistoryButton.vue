<script setup lang="ts">
/**
 * InputHistoryButton —— 「输入历史」图标按钮 + 选择弹窗（需求 5）。
 *
 * 三处输入（新建公式 / 模型测试 / 公式微调）共用这一个组件，
 * 只是 `items` 来源不同（各自的偏好键）。
 *
 * ## 为什么把「恢复」做成事件而不是内部处理
 *
 * 恢复要同时改**文本**与**附图**，而这两样在三个页面里的 ref 名与
 * 处理逻辑都不一样（比如模型测试要同时清掉待发送状态）。
 * 组件只管选，怎么落地交给页面 —— 否则这里会堆满 if。
 *
 * ## 空态也要给
 *
 * 一条都没有时不能只显示空列表：用户会以为按钮坏了。
 * 明确说「提交过一次之后才会出现在这里」。
 */
import { computed, ref } from 'vue'
import { formatDateTime } from '@/utils/format'
import type { InputHistoryItem } from '@/composables/useInputHistory'

const props = withDefaults(
  defineProps<{
    items: InputHistoryItem[]
    /** 弹窗标题 */
    title?: string
    /** 按钮提示文案 */
    hint?: string
    disabled?: boolean
  }>(),
  { title: '输入历史', hint: '从历史里选一条恢复', disabled: false },
)

const emit = defineEmits<{
  (e: 'restore', item: InputHistoryItem): void
  /** 删掉第 `index` 条（需求 6） */
  (e: 'remove', index: number): void
  (e: 'clear'): void
}>()

const open = ref(false)

const hasItems = computed(() => props.items.length > 0)

/** 一行里显示的文本（换行压成空格，避免多行把列表撑得很高） */
function oneLine(t: string, max = 90): string {
  const s = t.replace(/\s+/g, ' ').trim()
  return s.length > max ? `${s.slice(0, max)}…` : s
}

// 时间显示统一走 `utils/format` 的 `formatDateTime`（完整年月日时分秒）。
// 这里原本自己写了一个「今天只给时分」的 `when()`，精度与其他页面不一致，已删。

function pick(it: InputHistoryItem): void {
  emit('restore', it)
  open.value = false
}

/**
 * 单条删除 —— **不弹确认**。
 *
 * 需求 6 只要求「批量删除要二次确认」：删一条是明确的小动作，
 * 每次都弹框反而让人烦躁（而且误删一条的代价很低）。
 */
function removeOne(index: number): void {
  emit('remove', index)
}

/**
 * 清空 —— **必须二次确认**（需求 6：批量删除需要二次确认）。
 *
 * 确认框放在这里而不是三个调用方：它是这个弹窗的 UI 行为，
 * 复制到三处迟早会漏一处。
 */
async function confirmClear(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `将删除全部 ${props.items.length} 条输入历史。这条记录里的文本与附图引用会一起消失，且不可撤销。`,
      '确认清空输入历史',
      { type: 'warning', confirmButtonText: '清空', cancelButtonText: '取消' },
    )
  } catch {
    return // 用户取消
  }
  emit('clear')
  open.value = false
}
</script>

<template>
  <el-tooltip :content="hint" placement="top" :show-after="300">
    <button
      class="hist-btn"
      type="button"
      :disabled="disabled"
      :aria-label="hint"
      @click="open = true"
    >
      <span aria-hidden="true">🕘</span>
      <span v-if="hasItems" class="hist-btn__count">{{ items.length }}</span>
    </button>
  </el-tooltip>

  <el-dialog v-model="open" :title="title" width="560px">
    <div v-if="!hasItems" class="empty">
      <p class="empty__title">还没有输入历史</p>
      <p class="empty__text">成功提交过一次之后，这里会记下当时的文本与附图，随时可以恢复。</p>
    </div>

    <ul v-else class="list">
      <li v-for="(it, i) in items" :key="`${it.at}-${i}`" class="row">
        <button class="item" type="button" @click="pick(it)">
          <span class="item__top">
            <span class="item__time">{{ formatDateTime(it.at) }}</span>
            <span v-if="it.imageIds.length > 0" class="item__imgs">
              🖼 {{ it.imageIds.length }} 张
            </span>
          </span>
          <span class="item__text">{{ oneLine(it.text) || '（仅附图，无文字）' }}</span>
        </button>
        <el-tooltip content="删除这一条" placement="top" :show-after="300">
          <button
            class="row__del"
            type="button"
            :aria-label="`删除第 ${i + 1} 条历史`"
            @click="removeOne(i)"
          >
            ✕
          </button>
        </el-tooltip>
      </li>
    </ul>

    <template #footer>
      <el-button v-if="hasItems" @click="confirmClear()">清空历史</el-button>
      <el-button type="primary" @click="open = false">关闭</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.hist-btn {
  position: relative;
  display: inline-grid;
  place-items: center;
  width: 34px;
  height: 34px;
  padding: 0;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-btn);
  background: var(--c-surface);
  font-size: var(--f-size-base);
  line-height: 1;
  cursor: pointer;
  transition:
    border-color 0.12s ease,
    background 0.12s ease;
}

.hist-btn:hover:not(:disabled) {
  border-color: var(--c-primary);
  background: var(--c-surface-hover);
}

.hist-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

/* 条数角标：让人知道里面有东西 */
.hist-btn__count {
  position: absolute;
  top: -6px;
  right: -6px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--r-pill);
  background: var(--c-primary);
  color: #fff;
  font-size: 10px;
  line-height: 16px;
  text-align: center;
}

.empty {
  padding: var(--sp-5) var(--sp-3);
  text-align: center;
}

.empty__title {
  margin: 0 0 var(--sp-2);
  color: var(--c-text);
  font-size: var(--f-size-base);
  font-weight: 600;
}

.empty__text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 52vh;
  overflow-y: auto;
}

/* 一行 = 可点的条目 + 右侧删除按钮 */
.row {
  display: flex;
  align-items: stretch;
  border-bottom: var(--hairline) solid var(--c-divider);
}

.row:last-child {
  border-bottom: none;
}

.row:hover .row__del {
  opacity: 1;
}

.item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
  padding: var(--sp-3);
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
}

.item:hover {
  background: var(--c-surface-hover);
}

/* 删除按钮平时淡出，hover 整行时出现 —— 避免列表看起来全是 ✕ */
.row__del {
  flex-shrink: 0;
  width: 36px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-text-3);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.row__del:hover {
  background: var(--c-surface-hover);
  color: var(--c-danger);
}

/* 触屏 / 键盘用户没有 hover —— 焦点可见时也要显出来 */
.row__del:focus-visible {
  opacity: 1;
}

.item__top {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.item__time {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.item__imgs {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
  line-height: 18px;
}

.item__text {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  line-height: 1.6;
}
</style>

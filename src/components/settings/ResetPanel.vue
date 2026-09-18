<script setup lang="ts">
/**
 * ResetPanel —— 重置软件（9 类 + 真实存量 + 二次确认）。
 *
 * 见 docs/05-项目开发方案.md 「流程 8：重置软件」与 `civilcalc_core::reset`。
 *
 * ## 默认勾选：数据类全勾，**配置类不勾**
 *
 * 前 7 项（公式 / 历史 / 用量 / 图片 / 外观 / 提示词 / 输入历史）默认勾；
 * `LLM_CONFIG` 与 `WEBDAV_CONFIG` 默认不勾 ——
 * 把模型配置和云端账号一起清掉是很容易后悔的操作，要清得用户自己伸手。
 *
 * ## 二次确认的正文**来自后端**
 *
 * `reset_confirm_lines(selection)` 逐项写清将删除什么（含「本机暂无」），
 * 与 `reset_execute` 是同一份逻辑生成的 —— 弹窗里写的和实际做的不会漂移。
 * 前端只负责把这份文案排版好，**不自己拼**。
 *
 * ## 每行的副标题是前端拼的（已知的重复）
 *
 * 契约只有 3 个 reset 命令，没有「逐行副标题」的命令，
 * 所以 `subtitleOf` 在 `types/reset.ts` 里镜像了一份 Rust 实现。
 * 改一边必须改另一边（Rust 侧有单测钉住格式）。
 */
import { computed, onActivated, onMounted, ref } from 'vue'
import { resetApi } from '@/api'
import {
  RESET_SECTIONS,
  defaultResetSelection,
  describeSummary,
  emptyResetSelection,
  hasAnySelected,
  selectedSections,
  subtitleOf,
  type ResetCounts,
  type ResetSelection,
} from '@/types/reset'
import { errorMessage } from '@/types/error'

const emit = defineEmits<{
  /** 重置完成（父组件据此重载公式 / 历史 / 外观等） */
  (e: 'done'): void
}>()

const counts = ref<ResetCounts | null>(null)
const loading = ref(false)
const selection = ref<ResetSelection>(defaultResetSelection())

/** 二次确认弹窗 */
const confirmOpen = ref(false)
const confirmLines = ref<string[]>([])
const executing = ref(false)

onMounted(reload)

/**
 * 设置页被 `keep-alive` 缓存，`onMounted` 只跑一次。
 *
 * 「存量」是会变的（在别处生成了公式、算了历史、导入了图），
 * 切回来时必须重新数一遍 —— 否则副标题上写着「暂无插图」，
 * 实际点下去删掉一堆。
 */
onActivated(reload)

async function reload(): Promise<void> {
  loading.value = true
  try {
    counts.value = await resetApi.resetCounts()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    loading.value = false
  }
}

const selectedCount = computed(() => selectedSections(selection.value).length)
const canReset = computed(() => hasAnySelected(selection.value) && !executing.value)

function toggle(key: keyof ResetSelection, on: boolean): void {
  selection.value = { ...selection.value, [key]: on }
}

/** 只选数据类（配置类不动） */
function selectDataOnly(): void {
  const next = emptyResetSelection()
  for (const s of RESET_SECTIONS) next[s.key] = !s.configLike
  selection.value = next
}

function selectNone(): void {
  selection.value = emptyResetSelection()
}

function subtitleOfKey(key: keyof ResetSelection): string {
  return counts.value ? subtitleOf(counts.value, key) : ''
}

/** 打开二次确认（文案来自后端） */
async function askConfirm(): Promise<void> {
  if (!canReset.value) return
  try {
    confirmLines.value = await resetApi.resetConfirmLines(selection.value)
    confirmOpen.value = true
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

/** 真正执行 */
async function execute(): Promise<void> {
  if (executing.value) return
  executing.value = true
  try {
    const summary = await resetApi.resetExecute(selection.value)
    confirmOpen.value = false
    ElMessage.success(describeSummary(summary))
    // 存量变了 + 数据没了 → 通知父组件刷新
    await reload()
    selection.value = defaultResetSelection()
    emit('done')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    executing.value = false
  }
}
</script>

<template>
  <div class="reset">
    <el-skeleton v-if="loading && !counts" :rows="4" animated />

    <template v-else>
      <p class="reset__lead">
        把本机数据恢复到出厂状态。**只动本机** —— 云端备份一条也不碰。
      </p>

      <div class="reset__list">
        <!--
          🔴 这里**不能**用 `<label>` 包 `el-checkbox`（需求 9 的 BUG 根因）。

          `el-checkbox` 自己渲染的就是一个 `<label>`，套在外层 `<label>` 里
          构成嵌套 label：点复选框时，内层 label 先把点击转发给它的 input
          （切换一次），事件继续冒泡到外层 label，外层再把点击转发给同一个
          input（**又切换一次**）—— 两次抵消，表现为「勾选框点了没反应」。

          改成普通 `div` + 显式 toggle：整行可点，复选框自身用 `@click.stop`
          防止同一次点击被处理两遍。
        -->
        <div
          v-for="s in RESET_SECTIONS"
          :key="s.key"
          class="item"
          role="checkbox"
          :aria-checked="selection[s.key]"
          :tabindex="0"
          @click="toggle(s.key, !selection[s.key])"
          @keydown.space.prevent="toggle(s.key, !selection[s.key])"
          @keydown.enter.prevent="toggle(s.key, !selection[s.key])"
        >
          <el-checkbox
            class="item__box"
            :model-value="selection[s.key]"
            @click.stop
            @keydown.stop
            @update:model-value="(v: boolean | string | number) => toggle(s.key, !!v)"
          />
          <span class="item__main">
            <span class="item__label">
              {{ s.label }}
              <span v-if="s.configLike" class="item__tag">默认不勾</span>
            </span>
            <span class="item__sub">{{ subtitleOfKey(s.key) }}</span>
          </span>
        </div>
      </div>

      <div class="reset__ops">
        <button class="link" type="button" @click="selectDataOnly()">只选数据</button>
        <button class="link" type="button" @click="selectNone()">全不选</button>
        <button class="link" type="button" :disabled="loading" @click="reload()">刷新存量</button>
        <span class="reset__count">已选 {{ selectedCount }} 项</span>
      </div>

      <div class="reset__foot">
        <el-button type="danger" :disabled="!canReset" @click="askConfirm()">重置所选</el-button>
      </div>
    </template>

    <!-- 二次确认 -->
    <el-dialog v-model="confirmOpen" title="确认重置" width="480px">
      <p class="confirm__lead">将要清空/回退以下内容：</p>
      <ul class="confirm__lines">
        <li v-for="(l, i) in confirmLines" :key="i">{{ l }}</li>
      </ul>
      <p class="confirm__warn">
        <strong>此操作不可撤销。</strong>
        云端备份（WebDAV 上的包）不受影响，需要时可以再导入回来。
      </p>

      <template #footer>
        <el-button :disabled="executing" @click="confirmOpen = false">取消</el-button>
        <el-button type="danger" :loading="executing" @click="execute()">确认重置</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.reset {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.reset__lead {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.reset__list {
  display: flex;
  flex-direction: column;
}

.item {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  padding: var(--sp-2) 0;
  border-bottom: var(--hairline) solid var(--c-divider);
  cursor: pointer;
  /* 整行可点 —— 但要有可见的键盘焦点（它是 role="checkbox" 的自定义控件） */
  border-radius: var(--r-sm);
}

.item:hover {
  background: var(--c-surface-hover);
}

/* 复选框只作指示：点击/按键都由外层行统一处理（`@click.stop` + `@keydown.stop`） */
.item__box {
  flex-shrink: 0;
}

.item:last-child {
  border-bottom: none;
}

.item__main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.item__label {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--c-text);
  font-size: var(--f-size-sm);
}

.item__tag {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
  line-height: 18px;
}

.item__sub {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.reset__ops {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

.reset__count {
  margin-left: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.reset__foot {
  display: flex;
}

.confirm__lead {
  margin: 0 0 var(--sp-2);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.confirm__lines {
  margin: 0 0 var(--sp-3);
  padding-left: var(--sp-5);
  color: var(--c-text);
  font-size: var(--f-size-sm);
  line-height: 1.9;
}

.confirm__warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-sm);
  line-height: 1.7;
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

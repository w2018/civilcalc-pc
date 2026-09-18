<script setup lang="ts">
/**
 * ThinkingBox —— AI 思考过程（流式 + 可折叠 + 吸附置顶 + 全屏）。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「ThinkingBox」。
 *
 * ## 流式文本由**父组件**累加
 *
 * 本组件只负责显示与交互，`text` 是已经累加好的全文
 * （事件载荷是**增量**，`+=` 在父组件里做 —— 见 `api/ai.ts` 的说明）。
 * 这样同一个组件能同时服务「AI 生成」与「模型测试」两条链路。
 *
 * ## 自动滚到底只在**流式中**做
 *
 * 结束后还自动滚动会让用户没法往回翻 —— 一旦用户手动往上滚，
 * 就停止自动跟随（`followBottom` 标志）。
 *
 * ## 耗时怎么算
 *
 * `startedAt` 由父组件在发命令**之前**打点（不是本组件挂载时）——
 * 否则会把「等 IPC 调度」的时间也算进去。
 * 结束后冻结在最终值，不再走秒。
 *
 * ## 🔴 折叠随流式自动切换（需求 1）
 *
 * 「思考中自动展开、结束后自动折叠」——这是全局约定，所有用到思考框的地方
 * 都一样。所以 `autoCollapse` 默认为 `true`：
 *
 * | `streaming` | 行为 |
 * |---|---|
 * | `true`（开始） | 展开（用户要能看见模型在动） |
 * | `false`（结束） | 折叠（把版面让给正文） |
 *
 * 用户手动点过之后仍会被下一次状态切换覆盖 —— 这是有意的：
 * 折叠是**一次性动作**，不该把「上一次手动展开」的记忆带到下一轮生成。
 * 传 `:auto-collapse="false"` 可关掉（用于纯静态展示的历史思考）。
 *
 * ## token 数
 *
 * `tokens` 由父组件给。**流式中给的是估算值**（字符数 / 2），
 * 所以那时加「约」字；拿到厂商真实 usage 后由父组件换掉，
 * `tokensLive` 置 `false`，文案变成确定值。不要在没有数据时编一个数出来。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import StickyHeader from '@/components/common/StickyHeader.vue'
import { copyText } from '@/utils/clipboard'
import { useIsActive } from '@/composables/useIsActive'
import { formatTokens } from '@/utils/format'

const props = withDefaults(
  defineProps<{
    /** 已累加的思考全文 */
    text: string
    /** 是否还在流式输出 */
    streaming?: boolean
    /** 开始时刻（`Date.now()`）；为 `null` 时不显示耗时 */
    startedAt?: number | null
    /** 标题 */
    title?: string
    /** 初始是否折叠 */
    collapsed?: boolean
    /** 吸附置顶（嵌在长页面里时开） */
    sticky?: boolean
    /** 本次消耗的 token 数（`null` = 不显示） */
    tokens?: number | null
    /** token 数是否为**估算值**（流式中为 `true`，拿到真实 usage 后置 `false`） */
    tokensLive?: boolean
    /** 流式结束是否自动折叠（默认开，见上方说明） */
    autoCollapse?: boolean
  }>(),
  {
    streaming: false,
    startedAt: null,
    title: 'AI 思考过程',
    collapsed: false,
    sticky: false,
    tokens: null,
    tokensLive: false,
    autoCollapse: true,
  },
)

const open = ref(!props.collapsed)
const fullscreen = ref(false)
const bodyRef = ref<HTMLElement | null>(null)
/** 是否自动跟随到底部（用户往上滚后置 false） */
const followBottom = ref(true)

/** 耗时显示（流式中每 100ms 走一次） */
const now = ref(Date.now())
let timer: number | null = null

function startTimer(): void {
  stopTimer()
  timer = window.setInterval(() => {
    now.value = Date.now()
  }, 100)
}

function stopTimer(): void {
  if (timer !== null) {
    window.clearInterval(timer)
    timer = null
  }
}

/** 结束后冻结耗时：记下最后一刻的时间戳 */
const frozenAt = ref<number | null>(null)

watch(
  () => props.streaming,
  (s) => {
    if (s) {
      followBottom.value = true
      frozenAt.value = null
      startTimer()
      // 需求 1：思考中自动展开
      if (props.autoCollapse) open.value = true
    } else {
      frozenAt.value = Date.now()
      stopTimer()
      // 需求 1：结束后自动折叠，把版面让给正文
      if (props.autoCollapse) open.value = false
    }
  },
  { immediate: true },
)

onBeforeUnmount(stopTimer)

/** token 文案（`约 1.2k` / `1.2k`）；无数据时为空串 */
const tokensText = computed(() => {
  const t = props.tokens
  if (t === null || t === undefined || !Number.isFinite(t) || t <= 0) return ''
  return props.tokensLive ? `约 ${formatTokens(t)} tokens` : `${formatTokens(t)} tokens`})

const elapsedMs = computed(() => {
  if (props.startedAt === null) return null
  const end = props.streaming ? now.value : (frozenAt.value ?? now.value)
  return Math.max(0, end - props.startedAt)
})

const elapsedText = computed(() => {
  const ms = elapsedMs.value
  if (ms === null) return ''
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)}s`
  const m = Math.floor(s / 60)
  return `${m}m${(s % 60).toFixed(0)}s`
})

/** 流式中自动滚到底（用户手动上滚后停） */
watch(
  () => props.text,
  async () => {
    if (!props.streaming || !open.value || !followBottom.value) return
    await Promise.resolve()
    const el = bodyRef.value
    if (el) el.scrollTop = el.scrollHeight
  },
)

function onScroll(): void {
  const el = bodyRef.value
  if (!el) return
  // 距底部 24px 以内视为「还在跟随」
  followBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 24
}

async function copy(): Promise<void> {
  const ok = await copyText(props.text)
  if (ok) ElMessage.success('已复制思考过程')
  else ElMessage.warning('复制失败，请手动选中')
}

/**
 * 全屏层的 Esc 挂在 `window` 上。
 *
 * 容器 `tabindex="-1"` 不会自动获得焦点，挂在元素上的 `keydown` 收不到事件。
 */
function onFullscreenKey(e: KeyboardEvent): void {
  if (!active.value) return
  if (e.key === 'Escape') fullscreen.value = false
}

/**
 * 本页是否当前可见。
 *
 * 全屏层是 `<Teleport to="body">` 出去的，`keep-alive` 停用组件时
 * **收不走它** —— 不判 `active` 的话，切页后全屏遮罩会留在屏幕上。
 */
const active = useIsActive()

watch(fullscreen, (open) => {
  if (open) window.addEventListener('keydown', onFullscreenKey)
  else window.removeEventListener('keydown', onFullscreenKey)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onFullscreenKey)
})
</script>

<template>
  <section v-if="text" class="think" :class="{ 'think--streaming': streaming }">
    <StickyHeader v-if="sticky" :top="0" bg="var(--c-surface)">
      <button class="think__head" type="button" :aria-expanded="open" @click="open = !open">
        <span class="think__caret" aria-hidden="true">{{ open ? '▾' : '▸' }}</span>
        <span class="think__title">{{ title }}</span>
        <span v-if="streaming" class="think__live">思考中…</span>
        <span class="think__meta">
          <span v-if="tokensText" class="think__tokens">{{ tokensText }}</span>
          <span v-if="elapsedText" class="think__time">{{ elapsedText }}</span>
        </span>
      </button>
    </StickyHeader>

    <button
      v-else
      class="think__head"
      type="button"
      :aria-expanded="open"
      @click="open = !open"
    >
      <span class="think__caret" aria-hidden="true">{{ open ? '▾' : '▸' }}</span>
      <span class="think__title">{{ title }}</span>
      <span v-if="streaming" class="think__live">思考中…</span>
      <span class="think__meta">
        <span v-if="tokensText" class="think__tokens">{{ tokensText }}</span>
        <span v-if="elapsedText" class="think__time">{{ elapsedText }}</span>
      </span>
    </button>

    <div v-show="open" class="think__wrap">
      <div ref="bodyRef" class="think__body" @scroll="onScroll">{{ text }}</div>
      <div class="think__tools">
        <button class="link" type="button" @click="copy()">复制</button>
        <button class="link" type="button" @click="fullscreen = true">全屏查看</button>
      </div>
    </div>

    <!-- 全屏查看 -->
    <Teleport to="body">
      <div
        v-if="fullscreen && active"
        class="fs"
        role="dialog"
        aria-modal="true"
        aria-label="思考过程全屏查看"
      >
        <div class="fs__backdrop" @click="fullscreen = false" />
        <div class="fs__panel">
          <div class="fs__head">
            <span class="fs__title">{{ title }}</span>
            <span v-if="tokensText" class="fs__meta">{{ tokensText }}</span>
            <span v-if="elapsedText" class="fs__time">{{ elapsedText }}</span>
            <button class="fs__close" type="button" aria-label="关闭" @click="fullscreen = false">
              ✕
            </button>
          </div>
          <pre class="fs__body">{{ text }}</pre>
        </div>
      </div>
    </Teleport>
  </section>
</template>

<style scoped>
.think {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.think--streaming {
  /* 流式中给一条左侧主色细线，一眼看出还在动 */
  border-left: 2px solid var(--c-primary);
}

.think__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  padding: var(--sp-3) var(--sp-4);
  border: none;
  background: transparent;
  font-family: inherit;
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
  cursor: pointer;
  text-align: left;
}

.think__caret {
  width: 12px;
  flex-shrink: 0;
  color: var(--c-text-3);
}

.think__title {
  font-weight: 500;
}

.think__live {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-primary-light);
  color: var(--c-primary);
  font-size: var(--f-size-xs);
}

.think__meta {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  /* 靠右：把「思考中…」之后的所有信息推到行尾 */
  margin-left: auto;
}

.think__tokens {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.think__time {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.think__wrap {
  padding: 0 var(--sp-4) var(--sp-3);
}

.think__body {
  max-height: 240px;
  overflow-y: auto;
  padding: var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
}

.think__tools {
  display: flex;
  gap: var(--sp-3);
  margin-top: var(--sp-2);
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

.link:hover {
  text-decoration: underline;
}

/* ---- 全屏 ---- */

.fs {
  position: fixed;
  inset: 0;
  z-index: 3000;
  outline: none;
}

.fs__backdrop {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
}

.fs__panel {
  position: absolute;
  inset: 6vh 8vw;
  display: flex;
  flex-direction: column;
  border-radius: var(--r-dialog);
  background: var(--c-surface);
  overflow: hidden;
}

.fs__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border-bottom: var(--hairline) solid var(--c-divider);
}

.fs__title {
  font-weight: 600;
  color: var(--c-text);
}

.fs__time {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.fs__meta {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.fs__close {
  margin-left: auto;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--r-btn);
  background: transparent;
  color: var(--c-text-2);
  font-size: var(--f-size-base);
  cursor: pointer;
}

.fs__close:hover {
  background: var(--c-surface-hover);
}

.fs__body {
  flex: 1;
  margin: 0;
  padding: var(--sp-4);
  overflow-y: auto;
  color: var(--c-text-2);
  font-family: var(--f-sans);
  font-size: var(--f-size-sm);
  line-height: 1.8;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>

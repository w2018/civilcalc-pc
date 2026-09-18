<script setup lang="ts">
/**
 * ImageViewer —— 全屏图片查看器（滚轮缩放 / 拖动平移 / 前后切换）。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「ImageViewer」。
 *
 * ## 为什么不用 `el-image-viewer`
 *
 * Element Plus 的查看器只能看**一组 URL**，我们要的是
 * 「单图 + 保存回调 + 被 ExplanationPanel 与 ImagesView 共用」。
 * 自己写 60 行比适配它更省。
 *
 * ## 缩放与平移的实现要点
 *
 * - 缩放用 `transform: scale()`，**不重排**（大图重排会掉帧）
 * - 拖动用 `translate()`，与缩放合成在同一个 transform 里
 * - 切换图片时**必须重置**缩放与位移，否则看下一张会「莫名其妙放大着」
 * - 只有放大后才允许拖动（`scale <= 1` 时拖了也没意义，还会误触关闭）
 *
 * ## 关闭方式给足
 *
 * Esc / 点遮罩 / 右上角按钮 —— 全屏层最常见的挫败是「关不掉」。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useIsActive } from '@/composables/useIsActive'

const props = withDefaults(
  defineProps<{
    /** 是否显示（`v-model`） */
    modelValue: boolean
    /** 图片源（data URL 或 asset: URL） */
    images: string[]
    /** 打开时定位到第几张（0 起） */
    startIndex?: number
    /** 每张的标题（可选，显示在底部） */
    titles?: string[]
    /** 是否显示「另存为」按钮 */
    savable?: boolean
  }>(),
  { startIndex: 0, titles: () => [], savable: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  /** 点「另存为」，父组件负责弹保存对话框与写文件 */
  (e: 'save', index: number): void
}>()

const index = ref(0)
const scale = ref(1)
const tx = ref(0)
const ty = ref(0)

/** 拖动中（避免与点击遮罩关闭冲突） */
let dragging = false
let dragMoved = false
let startX = 0
let startY = 0

const current = computed(() => props.images[index.value] || null)
const total = computed(() => props.images.length)
const title = computed(() => props.titles[index.value] ?? '')

/** 打开时重置到起始图与原始缩放 */
watch(
  () => props.modelValue,
  (open) => {
    if (!open) return
    index.value = Math.min(Math.max(0, props.startIndex), Math.max(0, props.images.length - 1))
    reset()
  },
  { immediate: true },
)

// 图片集合变化（如删除后）时夹住当前下标
watch(
  () => props.images.length,
  (n) => {
    if (n === 0) {
      close()
      return
    }
    if (index.value >= n) index.value = n - 1
  },
)

function reset(): void {
  scale.value = 1
  tx.value = 0
  ty.value = 0
}

function close(): void {
  emit('update:modelValue', false)
}

/**
 * 前后切换。
 *
 * ⚠️ 会**跳过还没加载出来的图**（数组里的空串）——
 * 九宫格是懒加载的，用户点开查看器时相邻的图可能还没取到。
 * 直接切过去只会看到一片空白，不如跳到下一张能看的。
 */
function step(delta: number): void {
  const n = total.value
  if (n <= 1) return
  let i = index.value
  for (let k = 0; k < n; k += 1) {
    i = (i + delta + n) % n
    if (props.images[i]) break
  }
  index.value = i
  reset()
}

/** 滚轮缩放（以光标为锚点，缩放时不跑偏） */
function onWheel(e: WheelEvent): void {
  e.preventDefault()
  const next = Math.min(8, Math.max(1, scale.value * (e.deltaY < 0 ? 1.15 : 1 / 1.15)))
  scale.value = Number(next.toFixed(3))
  if (scale.value <= 1) {
    tx.value = 0
    ty.value = 0
  }
}

function onPointerDown(e: PointerEvent): void {
  if (scale.value <= 1) return
  dragging = true
  dragMoved = false
  startX = e.clientX - tx.value
  startY = e.clientY - ty.value
  ;(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId)
}

function onPointerMove(e: PointerEvent): void {
  if (!dragging) return
  const nx = e.clientX - startX
  const ny = e.clientY - startY
  if (Math.abs(nx - tx.value) > 2 || Math.abs(ny - ty.value) > 2) dragMoved = true
  tx.value = nx
  ty.value = ny
}

function onPointerUp(): void {
  dragging = false
}

/** 点遮罩关闭；拖动结束的那一次点击不算 */
function onBackdrop(): void {
  if (dragMoved) {
    dragMoved = false
    return
  }
  close()
}

/** 双击：放大 ⇄ 复位 */
function onDoubleClick(): void {
  if (scale.value > 1) reset()
  else scale.value = 2
}

function onKey(e: KeyboardEvent): void {
  // `active`：本页不是当前可见页时不响应 ——
  // keep-alive 下两个页面的查看器可能同时处于「打开」状态（见 useIsActive）
  if (!props.modelValue || !active.value) return
  if (e.key === 'Escape') close()
  else if (e.key === 'ArrowLeft') step(-1)
  else if (e.key === 'ArrowRight') step(1)
  else if (e.key === '0') reset()
}

/**
 * 本页是否当前可见。
 *
 * 🔴 查看器的遮罩是 `<Teleport to="body">` 出去的 ——
 * `keep-alive` 停用组件时移走的是组件自己的 DOM 子树，
 * **teleport 到 body 的节点不在那棵子树里**，于是「在 A 页打开查看器 →
 * 切到 B 页」时遮罩会**留在屏幕上**。所以模板里要判 `modelValue && active`。
 */
const active = useIsActive()

/**
 * 键盘事件挂在 `window` 上，而不是容器元素上。
 *
 * 容器虽然 `tabindex="-1"`，但没有任何东西会给它聚焦 ——
 * 挂在元素上的 `keydown` 根本收不到事件（踩过一次）。
 */
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <Teleport to="body">
    <div
      v-if="modelValue && active"
      class="viewer"
      role="dialog"
      aria-modal="true"
      aria-label="图片查看"
      @wheel="onWheel"
    >
      <!-- 遮罩层：点击关闭 -->
      <div class="viewer__backdrop" @click="onBackdrop()" />

      <div class="viewer__stage">
        <img
          v-if="current"
          class="viewer__img"
          :src="current"
          alt=""
          :style="{ transform: `translate(${tx}px, ${ty}px) scale(${scale})` }"
          :class="{ 'viewer__img--grabbable': scale > 1 }"
          @pointerdown="onPointerDown"
          @pointermove="onPointerMove"
          @pointerup="onPointerUp"
          @pointercancel="onPointerUp"
          @dblclick="onDoubleClick()"
        />
        <p v-else class="viewer__pending">这张图还没加载出来</p>
      </div>

      <!-- 工具条 -->
      <div class="viewer__bar">
        <button class="viewer__btn" type="button" :disabled="total <= 1" @click="step(-1)">‹</button>
        <span class="viewer__pos">{{ index + 1 }} / {{ total }}</span>
        <button class="viewer__btn" type="button" :disabled="total <= 1" @click="step(1)">›</button>
        <span class="viewer__zoom">{{ Math.round(scale * 100) }}%</span>
        <button class="viewer__btn viewer__btn--text" type="button" @click="reset()">复位</button>
        <button
          v-if="savable"
          class="viewer__btn viewer__btn--text"
          type="button"
          @click="emit('save', index)"
        >
          另存为
        </button>
        <button class="viewer__btn" type="button" aria-label="关闭" @click="close()">✕</button>
      </div>

      <p v-if="title" class="viewer__title">{{ title }}</p>
    </div>
  </Teleport>
</template>

<style scoped>
.viewer {
  position: fixed;
  inset: 0;
  z-index: 3000;
  outline: none;
}

.viewer__backdrop {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.86);
}

.viewer__stage {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  pointer-events: none;
}

.viewer__img {
  max-width: 92vw;
  max-height: 84vh;
  object-fit: contain;
  pointer-events: auto;
  transition: transform 0.06s linear;
  user-select: none;
  -webkit-user-drag: none;
}

.viewer__img--grabbable {
  cursor: grab;
}

.viewer__img--grabbable:active {
  cursor: grabbing;
}

.viewer__pending {
  margin: 0;
  color: rgba(255, 255, 255, 0.7);
  font-size: var(--f-size-sm);
}

.viewer__bar {
  position: absolute;
  bottom: var(--sp-5);
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-pill);
  background: rgba(0, 0, 0, 0.62);
  color: #fff;
  font-size: var(--f-size-sm);
}

.viewer__btn {
  min-width: 28px;
  height: 28px;
  padding: 0 var(--sp-2);
  border: none;
  border-radius: var(--r-pill);
  background: transparent;
  color: #fff;
  font-family: inherit;
  font-size: var(--f-size-lg);
  line-height: 1;
  cursor: pointer;
}

.viewer__btn--text {
  font-size: var(--f-size-sm);
}

.viewer__btn:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.18);
}

.viewer__btn:disabled {
  color: rgba(255, 255, 255, 0.35);
  cursor: default;
}

.viewer__pos,
.viewer__zoom {
  min-width: 44px;
  text-align: center;
  color: rgba(255, 255, 255, 0.86);
}

.viewer__title {
  position: absolute;
  top: var(--sp-5);
  left: 50%;
  transform: translateX(-50%);
  margin: 0;
  max-width: 70vw;
  color: rgba(255, 255, 255, 0.86);
  font-size: var(--f-size-sm);
  text-align: center;
}
</style>

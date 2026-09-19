<script setup lang="ts">
/**
 * ImageCachePanel —— 图片缓存管理（九宫格 + 批量删除 + 引用提示 + 占用统计）。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「图片缓存」与 §3.3.8 图片数据流。
 *
 * ## 🔴 删被引用的图要**明确提示**，不是拦着不让删
 *
 * `image_delete` 对「被引用」的图**照删**（后端不做保护）——
 * 删完公式/历史里的 `{{img:N}}` 会渲染成「图片已删除」占位。
 * 这是合理的（用户想清空间），但必须让他知道后果：
 * 确认框里逐张列出「被引用 N 处」，而不是一句笼统的「确定删除？」。
 *
 * ## 占用统计是「文件真实大小」之和
 *
 * `image_total_size` 给的是落盘字节，不是 base64 后的体积
 * （base64 会膨胀约 1/3，用那个数字会吓到用户）。
 *
 * ## 缩略图懒加载
 *
 * 与相册选择器共用 `useImageThumbs` —— 几百张图不会在打开页面时一起读盘。
 */
import { computed, h, onActivated, ref } from 'vue'
import { imageApi } from '@/api'
import { pickImageFiles, pickImageSavePath } from '@/api/dialog'
import { saveDataUrlToFile } from '@/api/asset'
import { useImageThumbs } from '@/composables/useImageThumbs'
import ImageViewer from './ImageViewer.vue'
import { formatDateTime } from '@/utils/format'
import { errorMessage } from '@/types/error'
import type { ImageCacheItem } from '@/types/image'

const emit = defineEmits<{
  /** 缓存有变动（父组件据此刷新统计等） */
  (e: 'changed'): void
}>()

const items = ref<ImageCacheItem[]>([])
const totalBytes = ref(0)
const loading = ref(true)
/** 已勾选的 id */
const selected = ref<string[]>([])

const { thumbs, bindLazy, prune, load, urlOf } = useImageThumbs()

/** 查看器 */
const viewerOpen = ref(false)
const viewerIndex = ref(0)

const importing = ref(false)

/**
 * 用 `onActivated` 而不是 `onMounted`。
 *
 * 本组件嵌在「图片缓存」页里，那一页被 `keep-alive` 缓存 ——
 * 切走再切回**不会重新挂载**，`onMounted` 不会再跑，列表会是旧快照。
 * `onActivated` 在**首次挂载**与每次重新激活时都会触发，
 * 两种情况都覆盖，所以不需要再挂一个 `onMounted`（否则首屏会拉两次）。
 */
onActivated(reload)

async function reload(): Promise<void> {
  loading.value = true
  try {
    const [list, size] = await Promise.all([imageApi.imageList(), imageApi.imageTotalSize()])
    items.value = list
    totalBytes.value = size
    // 已删的图不该还留着缩略图缓存
    prune(list.map((i) => i.id))
    // 选中的图若已不在列表里，去掉
    selected.value = selected.value.filter((id) => list.some((i) => i.id === id))
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    loading.value = false
  }
}

// ---------------------------------------------------------------- 展示

/** 按入库时间倒序（新图在前） */
const sorted = computed(() => [...items.value].sort((a, b) => b.createdAt - a.createdAt))

const referencedCount = computed(() => items.value.filter((i) => i.refCount > 0).length)

const selectedBytes = computed(() =>
  items.value.filter((i) => selected.value.includes(i.id)).reduce((s, i) => s + i.sizeBytes, 0),
)

const allSelected = computed(
  () => items.value.length > 0 && selected.value.length === items.value.length,
)

function humanSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${bytes} B`
}

// 时间显示统一走 `utils/format` 的 `formatDateTime`（完整年月日时分秒）。
// 这里原本自己抄了一份、且只到分 —— 两处精度不一致，已经删掉。

// ---------------------------------------------------------------- 选择

function toggle(id: string): void {
  selected.value = selected.value.includes(id)
    ? selected.value.filter((x) => x !== id)
    : [...selected.value, id]
}

function toggleAll(): void {
  selected.value = allSelected.value ? [] : items.value.map((i) => i.id)
}

function clearSelection(): void {
  selected.value = []
}

// ---------------------------------------------------------------- 查看与另存

/**
 * 查看器的图源数组。
 *
 * ⚠️ **不能过滤掉未加载的项** —— 数组下标必须与 `sorted` 一一对应，
 * 否则 `viewerIndex` 会指到别的图。未加载的位置留空串，
 * `ImageViewer` 会显示「还没加载出来」并在切换时跳过。
 */
const viewerImages = computed(() => sorted.value.map((i) => urlOf(i.id) ?? ''))

/** 打开查看器：先把点的那张加载出来，否则会看到「还没加载」 */
async function openViewer(id: string): Promise<void> {
  const list = sorted.value
  const i = list.findIndex((x) => x.id === id)
  if (i < 0) return
  if (!urlOf(id)) await load(id)
  viewerIndex.value = i
  viewerOpen.value = true
}

async function saveAs(index: number): Promise<void> {
  const item = sorted.value[index]
  if (!item) return
  const url = urlOf(item.id)
  if (!url) {
    ElMessage.warning('这张图还没加载出来，稍后再试')
    return
  }
  try {
    const path = await pickImageSavePath(`civilcalc-image-${item.id.slice(0, 8)}.jpg`)
    if (!path) return
    await saveDataUrlToFile(url, path)
    ElMessage.success('已保存')
  } catch (e) {
    ElMessage.error(`保存失败：${String(e)}`)
  }
}

// ---------------------------------------------------------------- 删除

async function removeSelected(): Promise<void> {
  if (selected.value.length === 0) return

  const picked = items.value.filter((i) => selected.value.includes(i.id))
  const referenced = picked.filter((i) => i.refCount > 0)

  // 二次确认：把「被引用」的图逐张列出来
  const lines: string[] = [`将删除 ${picked.length} 张图片，释放 ${humanSize(selectedBytes.value)}。`]
  if (referenced.length > 0) {
    lines.push(
      `其中 ${referenced.length} 张**正在被引用** —— 删除后，引用它们的公式与历史记录里对应位置会显示「图片已删除」占位：`,
    )
    for (const r of referenced.slice(0, 5)) {
      lines.push(`· ${r.id.slice(0, 8)}… 被引用 ${r.refCount} 处`)
    }
    if (referenced.length > 5) lines.push(`· …等共 ${referenced.length} 张`)
  }

  try {
    // 用 VNode 而不是 `customStyle` —— `ElMessageBoxOptions.customStyle`
    // 的类型是 Vue 的 `CSSProperties`，`whiteSpace` 走不过去（实测报
    // `'whiteSpace' does not exist in type 'CSSProperties'`）。
    // 直接给一个 `white-space: pre-line` 的 div，语义一样且类型安全。
    await ElMessageBox.confirm(
      h('div', { style: 'white-space: pre-line; line-height: 1.7' }, lines.join('\n')),
      '确认删除图片',
      {
        type: 'warning',
        confirmButtonText: '删除',
        cancelButtonText: '取消',
      },
    )
  } catch {
    return
  }

  try {
    const removed = await imageApi.imageDelete([...selected.value])
    ElMessage.success(`已删除 ${removed} 张`)
    selected.value = []
    await reload()
    emit('changed')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

// ---------------------------------------------------------------- 导入

async function importFiles(): Promise<void> {
  if (importing.value) return
  const paths = await pickImageFiles(true)
  if (paths.length === 0) return

  importing.value = true
  try {
    const ids: string[] = []
    for (const p of paths) ids.push(await imageApi.imageSaveFromPath(p))
    // 内容去重：同一张图重复导入只存一份
    const unique = new Set(ids)
    ElMessage.success(
      unique.size < ids.length
        ? `已导入 ${ids.length} 张（其中 ${ids.length - unique.size} 张内容重复，已去重）`
        : `已导入 ${ids.length} 张`,
    )
    await reload()
    emit('changed')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    importing.value = false
  }
}
</script>

<template>
  <div class="icp">
    <!-- 工具条 -->
    <div class="icp__bar">
      <div class="icp__stats">
        <span>{{ items.length }} 张 · 占用 {{ humanSize(totalBytes) }}</span>
        <span v-if="referencedCount > 0" class="icp__ref">
          {{ referencedCount }} 张被公式或历史引用
        </span>
      </div>
      <div class="icp__ops">
        <el-button size="small" :loading="importing" @click="importFiles()">导入图片</el-button>
        <el-button size="small" :disabled="items.length === 0" @click="toggleAll()">
          {{ allSelected ? '取消全选' : '全选' }}
        </el-button>
        <el-button
          size="small"
          type="danger"
          :disabled="selected.length === 0"
          @click="removeSelected()"
        >
          删除所选（{{ selected.length }}）
        </el-button>
        <el-button size="small" :disabled="loading" @click="reload()">刷新</el-button>
      </div>
    </div>

    <p v-if="selected.length > 0" class="icp__sel">
      已选 {{ selected.length }} 张，合计 {{ humanSize(selectedBytes) }}
      <button class="link" type="button" @click="clearSelection()">取消选择</button>
    </p>

    <!-- 内容 -->
    <el-skeleton v-if="loading" :rows="5" animated />

    <div v-else-if="items.length === 0" class="icp__empty">
      <p class="icp__empty-title">还没有缓存的图片</p>
      <p class="icp__empty-text">
        在「描述」或「粘贴」页附图，或在这里直接导入，图片会自动存进缓存并可复用。
      </p>
      <el-button type="primary" :loading="importing" @click="importFiles()">导入图片</el-button>
    </div>

    <div v-else class="grid">
      <div v-for="it in sorted" :key="it.id" class="cell" :class="{ 'cell--on': selected.includes(it.id) }">
        <button class="cell__pick" type="button" :aria-label="`选择图片 ${it.id.slice(0, 8)}`" @click="toggle(it.id)">
          <span class="cell__box" aria-hidden="true">{{ selected.includes(it.id) ? '✓' : '' }}</span>
        </button>

        <button class="cell__view" type="button" :aria-label="`查看图片 ${it.id.slice(0, 8)}`" @click="openViewer(it.id)">
          <img
            v-if="thumbs[it.id]"
            :ref="(el) => bindLazy(el as Element | null, it.id)"
            :src="thumbs[it.id]"
            alt=""
          />
          <span v-else :ref="(el) => bindLazy(el as Element | null, it.id)" class="cell__ph" />
        </button>

        <div class="cell__meta">
          <span class="cell__size">{{ humanSize(it.sizeBytes) }}</span>
          <span v-if="it.refCount > 0" class="cell__ref" :title="`被引用 ${it.refCount} 处`">
            引用 {{ it.refCount }}
          </span>
        </div>
        <span class="cell__time">{{ formatDateTime(it.createdAt) }}</span>
      </div>
    </div>

    <ImageViewer
      v-model="viewerOpen"
      :images="viewerImages"
      :start-index="viewerIndex"
      savable
      @save="saveAs"
    />
  </div>
</template>

<style scoped>
.icp {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.icp__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.icp__stats {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.icp__ref {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

.icp__ops {
  display: flex;
  gap: var(--sp-2);
  flex-wrap: wrap;
}

.icp__sel {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  margin: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.icp__empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-7) var(--sp-4);
  text-align: center;
}

.icp__empty-title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-lg);
  font-weight: 600;
}

.icp__empty-text {
  margin: 0 0 var(--sp-2);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

/* 九宫格 */
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: var(--sp-3);
}

.cell {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-2);
  border: 2px solid transparent;
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.cell--on {
  border-color: var(--c-primary);
}

.cell__pick {
  position: absolute;
  left: 6px;
  top: 6px;
  z-index: 1;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
}

.cell__box {
  display: block;
  width: 22px;
  height: 22px;
  border-radius: var(--r-sm);
  border: var(--hairline) solid rgba(255, 255, 255, 0.8);
  background: rgba(0, 0, 0, 0.45);
  color: #fff;
  font-size: 13px;
  line-height: 20px;
  text-align: center;
}

.cell--on .cell__box {
  background: var(--c-primary);
  border-color: var(--c-primary);
}

.cell__view {
  padding: 0;
  border: none;
  border-radius: var(--r-sm);
  background: var(--c-surface);
  overflow: hidden;
  cursor: zoom-in;
  aspect-ratio: 1;
}

.cell__view img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.cell__ph {
  display: block;
  width: 100%;
  height: 100%;
}

.cell__meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}

.cell__size {
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.cell__ref {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
  line-height: 16px;
}

.cell__time {
  color: var(--c-text-3);
  font-size: 11px;
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
</style>

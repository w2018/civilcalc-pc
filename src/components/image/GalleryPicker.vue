<script setup lang="ts">
/**
 * GalleryPicker —— 应用内相册（按天分组九宫格，多选，序号 = 附图顺序）。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「GalleryPicker」与「流程 1」的选图分支。
 *
 * ## 序号就是附图顺序
 *
 * 选中项右下角显示 **1、2、3…**，这个序号直接决定
 * `normalize_from_query(desc, images)` 里 `images` 的顺序，
 * 也就决定模型看到的附图顺序与详解里 `{{img:N}}` 的对应关系。
 * 所以**点选顺序**必须被保留（不是按 id 排序，也不是按时间）。
 *
 * ## 为什么用懒加载缩略图
 *
 * 图片缓存可能有几百张，`image_load_data_url` 每张都要读盘 + base64。
 * 一次性全取会在打开相册时卡住好几秒。这里用 `IntersectionObserver`
 * 只在格子进入视口时才去取 —— 打开相册几乎是瞬时的。
 *
 * ## 相册里没有想要的图
 *
 * 底部给「从本地文件添加」：走系统文件选择器 → `image_save_from_path`
 * 落盘（EXIF 方向 / 长边 1024 / JPEG q80 / 内容去重）后自动选中。
 * 这也是「无相册权限时的回落路径」—— Windows 上不存在系统相册权限，
 * 应用内缓存就是相册。
 */
import { computed, ref, watch } from 'vue'
import { imageApi } from '@/api'
import { pickImageFiles } from '@/api/dialog'
import { useImageThumbs } from '@/composables/useImageThumbs'
import { usePasteImage } from '@/composables/usePasteImage'
import { errorMessage } from '@/types/error'
import type { ImageCacheItem } from '@/types/image'

const props = withDefaults(
  defineProps<{
    /** 是否显示（`v-model:open`） */
    open: boolean
    /** 已选图片 id（**顺序即附图顺序**） */
    selected: string[]
    /** 最多可选张数 */
    max?: number
  }>(),
  { max: 5 },
)

const emit = defineEmits<{
  (e: 'update:open', v: boolean): void
  (e: 'update:selected', v: string[]): void
}>()

const items = ref<ImageCacheItem[]>([])
const loading = ref(false)
/** 本地暂存的勾选（点「确定」才提交给父组件） */
const draft = ref<string[]>([])

/** 缩略图懒加载（与图片缓存页共用同一个 composable） */
const { thumbs, bindLazy, prune } = useImageThumbs()

/** 按天分组（今天 / 昨天 / M月D日），组内按入库时间倒序 */
const groups = computed(() => {
  const map = new Map<string, ImageCacheItem[]>()
  const sorted = [...items.value].sort((a, b) => b.createdAt - a.createdAt)
  for (const it of sorted) {
    const key = dayKey(it.createdAt)
    const arr = map.get(key)
    if (arr) arr.push(it)
    else map.set(key, [it])
  }
  return [...map.entries()].map(([key, list]) => ({ key, label: dayLabel(key), list }))
})

const totalBytes = computed(() => items.value.reduce((s, i) => s + i.sizeBytes, 0))

/**
 * 相册弹窗里也能直接粘贴（需求 3）。
 *
 * 粘贴进来的是**新图**，会立刻出现在列表顶部并被勾选 ——
 * 与「从本地文件添加」等价，只是入口不同（剪贴板图片没有路径）。
 * 弹窗关闭时不监听，避免在别的页面误触。
 */
usePasteImage({
  enabled: () => props.open && !importing.value,
  remaining: () => props.max - draft.value.length,
  onImported: async (ids) => {
    // 新图先落进 draft（勾选），再刷新列表让它显示出来
    draft.value = [...draft.value, ...ids].slice(0, props.max)
    await reload()
  },
})

function dayKey(ms: number): string {
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}

function dayLabel(key: string): string {
  const today = dayKey(Date.now())
  const y = new Date()
  y.setDate(y.getDate() - 1)
  if (key === today) return '今天'
  if (key === dayKey(y.getTime())) return '昨天'
  const [, m, d] = key.split('-')
  return `${Number(m)} 月 ${Number(d)} 日`
}

function humanSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${bytes} B`
}

/** 打开时刷新列表并把父组件的选中状态拷进来 */
async function reload(): Promise<void> {
  loading.value = true
  try {
    items.value = await imageApi.imageList()
    // 已删掉的图不该还留着缩略图缓存
    prune(items.value.map((i) => i.id))
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
    items.value = []
  } finally {
    loading.value = false
  }
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      draft.value = [...props.selected]
      void reload()
    }
  },
)

// ---------------------------------------------------------------- 选择

/** 该图的序号（1 起）；未选中返回 0 */
function orderOf(id: string): number {
  return draft.value.indexOf(id) + 1
}

function toggle(id: string): void {
  const i = draft.value.indexOf(id)
  if (i >= 0) {
    draft.value = draft.value.filter((x) => x !== id)
    return
  }
  if (draft.value.length >= props.max) {
    ElMessage.warning(`最多只能附 ${props.max} 张图`)
    return
  }
  // 追加在末尾 → 序号即点选顺序
  draft.value = [...draft.value, id]
}

function confirm(): void {
  emit('update:selected', [...draft.value])
  emit('update:open', false)
}

function cancel(): void {
  emit('update:open', false)
}

/** 从本地文件导入：落盘后直接选中 */
const importing = ref(false)
async function importFromFiles(): Promise<void> {
  if (importing.value) return
  const paths = await pickImageFiles(true)
  if (paths.length === 0) return

  importing.value = true
  try {
    const added: string[] = []
    for (const p of paths) {
      if (draft.value.length + added.length >= props.max) {
        ElMessage.warning(`最多只能附 ${props.max} 张图，多余的已跳过`)
        break
      }
      added.push(await imageApi.imageSaveFromPath(p))
    }
    if (added.length === 0) return
    // 同一张图重复导入会去重成同一 id → 这里再滤一次，避免序号重复
    draft.value = [...draft.value, ...added.filter((id) => !draft.value.includes(id))]
    await reload()
    ElMessage.success(`已添加 ${added.length} 张`)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    importing.value = false
  }
}
</script>

<template>
  <el-dialog
    :model-value="open"
    title="选择附图"
    width="640px"
    :close-on-click-modal="false"
    @update:model-value="(v: boolean) => emit('update:open', v)"
  >
    <div class="gal">
      <div class="gal__head">
        <span class="gal__hint">
          点击选择，右下角序号即附图顺序；最多 {{ max }} 张
        </span>
        <span class="gal__meta">
          {{ items.length }} 张 · {{ humanSize(totalBytes) }}
        </span>
      </div>

      <el-skeleton v-if="loading" :rows="4" animated />

      <div v-else-if="items.length === 0" class="gal__empty">
        <p class="gal__empty-text">相册还是空的。附过的图会自动存到这里，随时可以复用。</p>
        <el-button type="primary" :loading="importing" @click="importFromFiles()">
          从本地文件添加
        </el-button>
      </div>

      <div v-else class="gal__body">
        <section v-for="g in groups" :key="g.key" class="gal__group">
          <h3 class="gal__group-title">{{ g.label }}</h3>
          <div class="gal__grid">
            <button
              v-for="it in g.list"
              :key="it.id"
              class="gal__cell"
              :class="{ 'gal__cell--on': orderOf(it.id) > 0 }"
              type="button"
              :aria-pressed="orderOf(it.id) > 0"
              :aria-label="`选择图片，已选第 ${orderOf(it.id) || '未选'}`"
              @click="toggle(it.id)"
            >
              <img
                v-if="thumbs[it.id]"
                :ref="(el) => bindLazy(el as Element | null, it.id)"
                :src="thumbs[it.id]"
                alt=""
              />
              <span v-else :ref="(el) => bindLazy(el as Element | null, it.id)" class="gal__ph" />
              <span v-if="it.refCount > 0" class="gal__ref" :title="`被引用 ${it.refCount} 处`">
                {{ it.refCount }}
              </span>
              <span v-if="orderOf(it.id) > 0" class="gal__order">{{ orderOf(it.id) }}</span>
            </button>
          </div>
        </section>
      </div>
    </div>

    <template #footer>
      <div class="gal__footer">
        <el-button :loading="importing" @click="importFromFiles()">从本地文件添加</el-button>
        <span class="gal__spacer" />
        <el-button @click="cancel()">取消</el-button>
        <el-button type="primary" @click="confirm()">确定（{{ draft.length }}）</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<style scoped>
.gal {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-height: 56vh;
}

.gal__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
}

.gal__hint,
.gal__meta {
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.gal__empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-6) var(--sp-4);
}

.gal__empty-text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  text-align: center;
}

.gal__body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.gal__group + .gal__group {
  margin-top: var(--sp-4);
}

.gal__group-title {
  margin: 0 0 var(--sp-2);
  font-size: var(--f-size-sm);
  font-weight: 500;
  color: var(--c-text-3);
}

.gal__grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--sp-2);
}

.gal__cell {
  position: relative;
  aspect-ratio: 1;
  padding: 0;
  border: 2px solid transparent;
  border-radius: var(--r-card);
  background: var(--c-surface-2);
  overflow: hidden;
  cursor: pointer;
}

.gal__cell--on {
  border-color: var(--c-primary);
}

.gal__cell img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.gal__ph {
  display: block;
  width: 100%;
  height: 100%;
}

/* 序号：右下角，与附图顺序一一对应 */
.gal__order {
  position: absolute;
  right: 4px;
  bottom: 4px;
  min-width: 20px;
  height: 20px;
  padding: 0 5px;
  border-radius: var(--r-pill);
  background: var(--c-primary);
  color: #fff;
  font-size: var(--f-size-xs);
  line-height: 20px;
  text-align: center;
}

/* 引用数：左上角小标 */
.gal__ref {
  position: absolute;
  left: 4px;
  top: 4px;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border-radius: var(--r-pill);
  background: rgba(0, 0, 0, 0.55);
  color: #fff;
  font-size: 11px;
  line-height: 18px;
  text-align: center;
}

.gal__footer {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.gal__spacer {
  flex: 1;
}
</style>

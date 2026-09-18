<script setup lang="ts">
/**
 * ImageAttachRow —— 附图行（缩略图 + 序号 + 删除 + 添加）。
 *
 * 见 docs/05-项目开发方案.md §2.1.3 与「流程 1」。
 *
 * ## 序号是**语义**，不是装饰
 *
 * 缩略图左下角的 1/2/3… 与 `normalize_from_*` 收到的 `images` 顺序、
 * 以及模型输出里 `{{img:N}}` 的 N 是**同一个东西**。
 * 所以这里既不能排序、也不能因为删除而重排语义
 * （删掉第 2 张后，原第 3 张就变成第 2 张 —— 这是对的，
 * 因为提交给模型的顺序本来就该跟着变）。
 *
 * ## 缩略图缓存
 *
 * `image_load_data_url` 每次读盘 + base64。同一批图在
 * 「附图行 → 相册 → 查看器」之间来回切换时不该重复读，所以缓存住。
 * 删除图片 id 时顺手清缓存，避免删了还能看到。
 *
 * ## 支持剪贴板粘贴（需求 3）
 *
 * 三种添加方式并存：**选文件** / **从相册选** / **直接 Ctrl+V 粘贴**。
 * 粘贴由 `usePasteImage` 挂在 `window` 上处理 —— 用户可能刚点完按钮
 * 就按 Ctrl+V，焦点不在任何输入框里，挂在元素上会接不住。
 * 文本粘贴会被放行，不会把输入框的粘贴功能吃掉。
 */
import { ref, watch } from 'vue'
import { imageApi } from '@/api'
import { pickImageFiles, pickImageSavePath } from '@/api/dialog'
import { saveDataUrlToFile } from '@/api/asset'
import { usePasteImage } from '@/composables/usePasteImage'
import ImageViewer from './ImageViewer.vue'
import GalleryPicker from './GalleryPicker.vue'
import { errorMessage } from '@/types/error'

const props = withDefaults(
  defineProps<{
    /** 已附图 id（**顺序即附图顺序**） */
    modelValue: string[]
    /** 最多张数 */
    max?: number
    /** 禁用（AI 生成中） */
    disabled?: boolean
    /**
     * 紧凑模式：全部控件压成**一行**、缩略图缩小到 44px。
     *
     * 用于「输入区高度敏感」的场景（模型测试的对话框）——
     * 那里附图是可选功能，不该占掉半屏。默认（非紧凑）是
     * 缩略图一行 + 操作行一行，适合「新建公式」这种主角是附图的页面。
     */
    compact?: boolean
  }>(),
  { max: 5, disabled: false, compact: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: string[]): void
}>()

/** id → data URL 缓存 */
const thumbs = ref<Record<string, string>>({})

const galleryOpen = ref(false)
const viewerOpen = ref(false)
const viewerIndex = ref(0)
const importing = ref(false)

/** 缩略图 URL 列表（供查看器用，顺序与附图一致） */
function urlList(): string[] {
  return props.modelValue.map((id) => thumbs.value[id] ?? '')
}

watch(
  () => props.modelValue,
  async (ids) => {
    // 清理已移除的缓存（避免「删了还看得到」）
    const kept: Record<string, string> = {}
    for (const id of ids) if (thumbs.value[id]) kept[id] = thumbs.value[id]
    thumbs.value = kept

    for (const id of ids) {
      if (thumbs.value[id]) continue
      try {
        const url = await imageApi.imageLoadDataUrl(id)
        if (url) thumbs.value = { ...thumbs.value, [id]: url }
      } catch {
        // 单张失败不阻断其余（图片可能已被清理）
      }
    }
  },
  { immediate: true },
)

function removeAt(i: number): void {
  if (props.disabled) return
  emit(
    'update:modelValue',
    props.modelValue.filter((_, idx) => idx !== i),
  )
}

function onPicked(ids: string[]): void {
  emit('update:modelValue', ids)
}

/**
 * 剪贴板粘贴导入（需求 3）。
 *
 * 与「选文件」是两条不同的后端入口：粘贴的图**没有路径**，
 * 走 `image_save_from_bytes`；但两者处理管线一致，库里看不出区别。
 */
usePasteImage({
  enabled: () => !props.disabled,
  remaining: () => props.max - props.modelValue.length,
  onImported: (ids) => {
    emit('update:modelValue', [...props.modelValue, ...ids].slice(0, props.max))
  },
})

function openViewer(i: number): void {
  viewerIndex.value = i
  viewerOpen.value = true
}

/** 直接走系统文件选择器（不进相册） */
async function addFromFiles(): Promise<void> {
  if (props.disabled || importing.value) return
  const paths = await pickImageFiles(true)
  if (paths.length === 0) return

  importing.value = true
  try {
    const out = [...props.modelValue]
    let skipped = 0
    for (const p of paths) {
      if (out.length >= props.max) {
        skipped += 1
        continue
      }
      const id = await imageApi.imageSaveFromPath(p)
      // 内容去重后可能返回已有 id → 不重复占位
      if (!out.includes(id)) out.push(id)
    }
    emit('update:modelValue', out)
    if (skipped > 0) ElMessage.warning(`最多只能附 ${props.max} 张图，已跳过 ${skipped} 张`)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    importing.value = false
  }
}

/** 「另存为」：把当前图的 data URL 交给原生保存对话框 */
async function saveImage(i: number): Promise<void> {
  const url = thumbs.value[props.modelValue[i] ?? '']
  if (!url) return
  try {
    const path = await pickImageSavePath(`附图-${i + 1}.jpg`)
    if (!path) return
    await saveDataUrlToFile(url, path)
    ElMessage.success('已保存')
  } catch (e) {
    ElMessage.error(`保存失败：${String(e)}`)
  }
}
</script>

<template>
  <div class="attach" :class="{ 'attach--compact': compact }">
    <div class="attach__list">
      <div v-for="(id, i) in modelValue" :key="id" class="attach__item">
        <button class="attach__thumb" type="button" :aria-label="`查看第 ${i + 1} 张附图`" @click="openViewer(i)">
          <img v-if="thumbs[id]" :src="thumbs[id]" alt="" />
          <span v-else class="attach__ph" />
        </button>
        <span class="attach__order">{{ i + 1 }}</span>
        <button
          v-if="!disabled"
          class="attach__del"
          type="button"
          :aria-label="`移除第 ${i + 1} 张附图`"
          @click="removeAt(i)"
        >
          ✕
        </button>
      </div>

      <button
        v-if="modelValue.length < max"
        class="attach__add"
        type="button"
        :disabled="disabled"
        :aria-label="'从相册添加附图'"
        @click="galleryOpen = true"
      >
        <span class="attach__add-icon">＋</span>
        <span v-if="!compact" class="attach__add-text">相册</span>
      </button>
    </div>

    <div class="attach__actions">
      <button class="link" type="button" :disabled="disabled || importing" @click="addFromFiles()">
        {{ compact ? '本地图片' : '从本地文件添加' }}
      </button>
      <span class="attach__paste-hint">
        <kbd>Ctrl</kbd>+<kbd>V</kbd> 粘贴
      </span>
      <span class="attach__count">{{ modelValue.length }} / {{ max }}</span>
    </div>

    <GalleryPicker
      v-model:open="galleryOpen"
      :selected="modelValue"
      :max="max"
      @update:selected="onPicked"
    />

    <ImageViewer
      v-model="viewerOpen"
      :images="urlList()"
      :start-index="viewerIndex"
      savable
      @save="saveImage"
    />
  </div>
</template>

<style scoped>
.attach {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

/* ---------------------------------------------------------------------------
 * 紧凑模式：全部控件压成一行、缩略图缩小
 *
 * 用于「输入区高度敏感」的场景（模型测试的对话框）。那里附图是可选功能，
 * 按常规模式会占掉 ~200px（一行 72px 缩略图 + 一行操作），
 * 而输入区是 `flex-shrink: 0` 的，等于直接把对话区挤没。
 * ------------------------------------------------------------------------- */
.attach--compact {
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.attach--compact .attach__list {
  flex-wrap: nowrap;
  gap: var(--sp-2);
}

.attach--compact .attach__item {
  width: 44px;
  height: 44px;
}

.attach--compact .attach__add {
  width: 44px;
  height: 44px;
}

.attach--compact .attach__actions {
  margin-top: 0;
}

.attach--compact .attach__paste-hint {
  /* 紧凑模式下父容器已经是横排，不需要再靠 margin 推到右边 */
  margin-left: 0;
}

.attach__list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.attach__item {
  position: relative;
  width: 72px;
  height: 72px;
}

.attach__thumb {
  width: 100%;
  height: 100%;
  padding: 0;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
  overflow: hidden;
  cursor: zoom-in;
}

.attach__thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.attach__ph {
  display: block;
  width: 100%;
  height: 100%;
}

.attach__order {
  position: absolute;
  left: 4px;
  bottom: 4px;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  border-radius: var(--r-pill);
  background: rgba(0, 0, 0, 0.55);
  color: #fff;
  font-size: 11px;
  line-height: 18px;
  text-align: center;
  pointer-events: none;
}

.attach__del {
  position: absolute;
  right: -6px;
  top: -6px;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
}

.attach__del:hover {
  background: var(--c-danger);
}

.attach__add {
  width: 72px;
  height: 72px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  border: var(--hairline) dashed var(--c-divider);
  border-radius: var(--r-card);
  background: transparent;
  color: var(--c-text-3);
  font-family: inherit;
  cursor: pointer;
}

.attach__add:hover:not(:disabled) {
  border-color: var(--c-primary);
  color: var(--c-primary);
}

.attach__add:disabled {
  cursor: default;
  opacity: 0.6;
}

.attach__add-icon {
  font-size: var(--f-size-xl);
  line-height: 1;
}

.attach__add-text {
  font-size: var(--f-size-xs);
}

.attach__actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.attach__count {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

/* 「Ctrl+V 可粘贴」提示 —— 弱化，不抢主操作 */
.attach__paste-hint {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-left: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.attach__paste-hint kbd {
  padding: 0 4px;
  border: var(--hairline) solid var(--c-divider);
  border-bottom-width: 2px;
  border-radius: 4px;
  background: var(--c-surface-2);
  font-family: var(--f-sans);
  font-size: 11px;
  line-height: 16px;
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

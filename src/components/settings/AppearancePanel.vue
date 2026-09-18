<script setup lang="ts">
/**
 * AppearancePanel —— 外观（主题三态 / 页面底色 / 背景图 / 透明度 / 文字色）。
 *
 * 见 docs/05-项目开发方案.md §2.3.6 与「流程 7：外观设置」。
 *
 * ## 三个数据属性正交（这是本组件的核心难点）
 *
 * | 属性 | 决定什么 | 谁负责 |
 * |---|---|---|
 * | `data-theme` | 卡片 / 边框 / 控件色板 | `useTheme` |
 * | `data-bg` | 页面底色是否被覆盖 | `useBackground` |
 * | `data-bg-contrast` | **文字色组** | 由背景亮度自动推导 |
 *
 * 「浅色主题 + 深色背景」时文字必须自动变浅 —— 否则深底黑字看不清。
 * 所以「自动」档的文字色不能简单等于主题文字色，要按**背景亮度**推。
 *
 * ## 对比度校验要在**这里**给结论
 *
 * 用户选了一组「浅灰底 + 浅灰字」时，界面必须在选择处就提示，
 * 而不是等他抱怨看不清。阈值用 WCAG 正文下限 4.5:1。
 *
 * ## 背景图与页面底色是两回事
 *
 * 设了背景图时页面底色被图盖住，此时底色选择器的意义下降 ——
 * 所以有背景图时把底色那一行标注出来（不隐藏，用户可能想换回去）。
 */
import { computed } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useTheme } from '@/composables/useTheme'
import {
  CONTRAST_MIN,
  contrastGroupOf,
  contrastRatio,
  hexToRgb,
} from '@/composables/useBackground'
import { pickImageFiles } from '@/api/dialog'
import ColorRectPicker from './ColorRectPicker.vue'
import { errorMessage } from '@/types/error'
import type { ThemeMode } from '@/types/system'

const ui = useUiStore()
const { isDark } = useTheme()

const THEMES: { key: ThemeMode; label: string; hint: string }[] = [
  { key: 'SYSTEM', label: '跟随系统', hint: '系统切暗色时自动跟随' },
  { key: 'LIGHT', label: '亮色', hint: '始终使用浅色卡片' },
  { key: 'DARK', label: '暗色', hint: '始终使用深色卡片' },
]

/** 生效的页面底色（未自定义时按主题推） */
const effectiveBg = computed(() => {
  if (ui.backgroundColor && hexToRgb(ui.backgroundColor)) return ui.backgroundColor
  return isDark.value ? '#111111' : '#f2f2f2'
})

/** 生效的文字色（`AUTO` 时按背景亮度推） */
const effectiveText = computed(() => {
  if (ui.textColor !== 'AUTO' && hexToRgb(ui.textColor)) return ui.textColor
  return contrastGroupOf(effectiveBg.value) === 'light-text' ? '#ededed' : '#191919'
})

const ratio = computed(() => contrastRatio(effectiveBg.value, effectiveText.value))

/** 对比度是否达标（背景图开启时不判 —— 图上的文字对比度没法算） */
const contrastOk = computed(() => (ratio.value ?? 0) >= CONTRAST_MIN)

async function onTheme(next: ThemeMode): Promise<void> {
  await ui.setTheme(next)
}

async function onPickImage(): Promise<void> {
  const paths = await pickImageFiles(false)
  if (paths.length === 0) return
  try {
    await ui.setBackgroundImage(paths[0], ui.transparency)
    ElMessage.success('背景已更新')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function onClearImage(): Promise<void> {
  try {
    await ui.clearBackground()
    ElMessage.success('已清除背景图，文字色与透明度一并回默认')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function onTransparency(v: number | number[]): Promise<void> {
  const n = Array.isArray(v) ? v[0] : v
  await ui.setTransparency(n)
}
</script>

<template>
  <div class="ap">
    <!-- 主题 -->
    <section class="ap__block">
      <h4 class="ap__label">主题</h4>
      <div class="seg">
        <button
          v-for="t in THEMES"
          :key="t.key"
          class="seg__btn"
          :class="{ 'seg__btn--on': ui.themeMode === t.key }"
          type="button"
          :title="t.hint"
          @click="onTheme(t.key)"
        >
          {{ t.label }}
        </button>
      </div>
    </section>

    <!-- 页面底色 -->
    <section class="ap__block">
      <h4 class="ap__label">
        页面底色
        <span v-if="ui.hasBackground" class="ap__note">（已设背景图，底色被图盖住）</span>
      </h4>
      <ColorRectPicker
        :model-value="ui.backgroundColor ?? ''"
        @update:model-value="(v: string) => ui.setBackgroundColor(v)"
      />
      <button
        v-if="ui.backgroundColor"
        class="link"
        type="button"
        @click="ui.setBackgroundColor(null)"
      >
        恢复默认底色
      </button>
    </section>

    <!-- 背景图 -->
    <section class="ap__block">
      <h4 class="ap__label">背景图</h4>
      <div class="bg">
        <div class="bg__thumb" :class="{ 'bg__thumb--empty': !ui.hasBackground }">
          <img v-if="ui.backgroundUrl" :src="ui.backgroundUrl" alt="当前背景" />
          <span v-else>未设置</span>
        </div>
        <div class="bg__ops">
          <el-button @click="onPickImage()">
            {{ ui.hasBackground ? '更换图片' : '选择图片' }}
          </el-button>
          <el-button v-if="ui.hasBackground" @click="onClearImage()">清除</el-button>
          <p class="bg__hint">
            图片会**复制**进应用数据目录（不会引用你原来的文件），
            重装或移动原图都不影响。
          </p>
        </div>
      </div>

      <div class="bg__trans">
        <span class="bg__trans-label">透明度</span>
        <el-slider
          :model-value="ui.transparency"
          :min="0"
          :max="100"
          :disabled="!ui.hasBackground"
          @update:model-value="onTransparency"
        />
        <span class="bg__trans-val">{{ ui.transparency }}%</span>
      </div>
      <p v-if="!ui.hasBackground" class="bg__hint">先选一张背景图，透明度才有意义。</p>
    </section>

    <!-- 文字色 -->
    <section class="ap__block">
      <h4 class="ap__label">文字颜色</h4>
      <ColorRectPicker
        :model-value="ui.textColor"
        allow-auto
        auto-label="自动（按背景亮度）"
        @update:model-value="(v: string) => ui.setTextColor(v)"
      />
      <p class="ap__contrast" :class="{ 'ap__contrast--bad': !contrastOk && !ui.hasBackground }">
        <template v-if="ui.hasBackground">
          当前对比度 {{ ratio ? ratio.toFixed(1) : '—' }}:1
          —— 背景图上的文字对比度无法自动判定，请以实际观感为准。
        </template>
        <template v-else-if="contrastOk">
          当前对比度 {{ ratio ? ratio.toFixed(1) : '—' }}:1，可读性达标（下限
          {{ CONTRAST_MIN }}:1）。
        </template>
        <template v-else>
          当前对比度 {{ ratio ? ratio.toFixed(1) : '—' }}:1，**低于**可读性下限
          {{ CONTRAST_MIN }}:1 —— 正文会看不清，建议换深/浅一点的字色或底色。
        </template>
      </p>
    </section>
  </div>
</template>

<style scoped>
.ap {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
}

.ap__block {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.ap__label {
  margin: 0;
  font-size: var(--f-size-sm);
  font-weight: 600;
  color: var(--c-text-2);
}

.ap__note {
  font-weight: 400;
  color: var(--c-text-3);
}

/* 三态分段控件 */
.seg {
  display: inline-flex;
  padding: 2px;
  border-radius: var(--r-btn);
  background: var(--c-surface-2);
}

.seg__btn {
  padding: var(--sp-2) var(--sp-4);
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.seg__btn--on {
  background: var(--c-surface);
  color: var(--c-primary);
  font-weight: 500;
}

.bg {
  display: flex;
  gap: var(--sp-3);
  align-items: flex-start;
}

.bg__thumb {
  width: 120px;
  height: 72px;
  flex-shrink: 0;
  border-radius: var(--r-card);
  border: var(--hairline) solid var(--c-divider);
  background: var(--c-surface-2);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.bg__thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.bg__ops {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  align-items: center;
}

.bg__hint {
  width: 100%;
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.6;
}

.bg__trans {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

.bg__trans-label {
  flex-shrink: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.bg__trans-val {
  flex-shrink: 0;
  width: 44px;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.ap__contrast {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.6;
}

.ap__contrast--bad {
  color: var(--c-warning);
}

.link {
  align-self: flex-start;
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

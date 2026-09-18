<script setup lang="ts">
/**
 * 图片缓存 —— 九宫格预览 + 批量删除 + 占用统计。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「图片缓存」与 §3.3.8 图片数据流。
 *
 * ## 为什么这里只是一层薄壳
 *
 * 全部逻辑（懒加载缩略图、批量选择、引用提示、二次确认、另存为）
 * 都在 `ImageCachePanel` 里。视图只负责页面标题与布局 ——
 * 这样将来要把这块嵌进「设置」页也只是一行标签的事。
 *
 * ## 与「相册选择器」的分工
 *
 * | 组件 | 场景 | 能否删除 |
 * |---|---|---|
 * | `GalleryPicker` | 附图时**选**图（弹窗） | 否 |
 * | `ImageCachePanel` | **管理**缓存（整页） | 是 |
 *
 * 两者共用 `useImageThumbs`，缩略图的加载与缓存策略不会漂移。
 */
import ImageCachePanel from '@/components/image/ImageCachePanel.vue'
</script>

<template>
  <div class="view">
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">图片缓存</span>
        <span class="panel__hint">附过的图都存在这里，删掉后引用处会显示「图片已删除」</span>
      </div>
      <div class="panel__body">
        <ImageCachePanel />
      </div>
    </section>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: none;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.panel__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-4) 0;
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}
</style>

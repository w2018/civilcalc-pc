<script setup lang="ts">
/**
 * 计算书 HTML 预览（P3-14 / ADR-018）。
 *
 * `report_preview` 返回**完整 HTML 文档**，直接塞进 `iframe.srcdoc` 即可。
 *
 * ## 安全
 *
 * 内容由**后端**生成（受信任），但预览页**永不执行脚本**：
 * `sandbox=""`（最严格）—— 仅渲染 HTML/CSS，禁脚本/表单/同源。
 * 计算书 HTML 本身不含脚本，所以视觉不受影响。
 *
 * ## 标注
 *
 * 页面顶部固定一条「预览仅供参考，实际以导出的 Word 文件为准」——
 * 因为预览与导出是**两套渲染**，存在视觉不一致风险（ADR-018）。
 */
defineProps<{
  /** 预览 HTML（完整文档） */
  html: string
  /** 是否正在生成 */
  loading?: boolean
}>()
</script>

<template>
  <div class="preview">
    <div class="preview__bar">预览仅供参考，实际以导出的 Word 文件为准</div>
    <div class="preview__body">
      <div v-if="loading" class="preview__loading">生成预览中…</div>
      <iframe
        v-else
        class="preview__frame"
        title="计算书预览"
        sandbox=""
        :srcdoc="html"
      ></iframe>
    </div>
  </div>
</template>

<style scoped>
.preview {
  display: flex;
  flex-direction: column;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  overflow: hidden;
  background: var(--c-surface);
}

.preview__bar {
  flex-shrink: 0;
  padding: var(--sp-1) var(--sp-3);
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  background: var(--c-surface-2);
  border-bottom: var(--hairline) solid var(--c-divider);
}

.preview__body {
  flex: 1;
  min-height: 320px;
  display: flex;
}

.preview__loading {
  margin: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.preview__frame {
  flex: 1;
  width: 100%;
  border: none;
  background: #fff;
}
[data-theme='dark'] .preview__frame {
  background: #fff;
}
</style>

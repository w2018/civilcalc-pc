<script setup lang="ts">
/**
 * 单个排版节点的渲染器（递归）。
 *
 * 见 docs/08-IPC契约.md 组 12b 与 `types/math.ts` 的 `MathNode`。
 *
 * ## 设计要点
 *
 * - 后端**只出数据**，绘制全在前端（浏览器最擅长文本测量）。
 * - 没有 `subscript` / `row` 节点：下标是后端在文本层做的 Unicode 替换
 *   （`a11` → `a₁₁`），`MathNode[]` 序列本身就是一行。
 * - 本组件**自我递归**（Vue 3 `<script setup>` 通过文件名推导组件名），
 *   渲染子节点时直接引用 `<MathNodeView>`。
 *
 * ## 颜色（语义上色）
 *
 * 文本节点按 `kind` 上色，色值来自 `tokens.css` 的 `--c-math-*` 变量：
 * `symbol`(绿) / `function`(紫) / `operator`(蓝灰) / `number`(正文) / `normal`(正文)。
 */
import type { MathNode } from '@/types/math'

defineOptions({ name: 'MathNodeView' })

defineProps<{ node: MathNode }>()
</script>

<template>
  <!-- 普通文本：按 kind 上色 -->
  <span
    v-if="node.type === 'text'"
    class="mn"
    :class="`mn--${node.kind}`"
    >{{ node.text }}</span
  >

  <!-- 分数：分子 / 横线 / 分母 -->
  <span v-else-if="node.type === 'fraction'" class="mn-fraction">
    <span class="mn-fraction__num">
      <MathNodeView v-for="(n, i) in node.numerator" :key="`n${i}`" :node="n" />
    </span>
    <span class="mn-fraction__rule" aria-hidden="true"></span>
    <span class="mn-fraction__den">
      <MathNodeView v-for="(n, i) in node.denominator" :key="`d${i}`" :node="n" />
    </span>
  </span>

  <!-- 上标（幂）：base 与 exp 基线对齐，exp 上移缩小 -->
  <span v-else-if="node.type === 'superscript'" class="mn-super">
    <span class="mn-super__base">
      <MathNodeView v-for="(n, i) in node.base" :key="`b${i}`" :node="n" />
    </span>
    <span class="mn-super__exp">
      <MathNodeView v-for="(n, i) in node.exponent" :key="`e${i}`" :node="n" />
    </span>
  </span>

  <!-- 根号：n 次根时 index 在 √ 左上方 -->
  <span v-else-if="node.type === 'radical'" class="mn-radical">
    <span v-if="node.index != null && node.index > 0" class="mn-radical__idx">{{
      node.index
    }}</span>
    <span class="mn-radical__sign" aria-hidden="true">√</span>
    <span class="mn-radical__body">
      <MathNodeView v-for="(n, i) in node.radicand" :key="`r${i}`" :node="n" />
    </span>
  </span>

  <!-- 绝对值 |x|：用两根竖线（避免字符宽度不稳） -->
  <span v-else-if="node.type === 'absolute'" class="mn-abs">
    <span class="mn-abs__bar" aria-hidden="true"></span>
    <span class="mn-abs__inner">
      <MathNodeView v-for="(n, i) in node.inner" :key="`a${i}`" :node="n" />
    </span>
    <span class="mn-abs__bar" aria-hidden="true"></span>
  </span>

  <!-- 函数调用：name(arg, arg) -->
  <span v-else-if="node.type === 'function'" class="mn-fn">
    <span class="mn-fn__name">{{ node.name }}</span>
    <span class="mn-fn__paren">(</span>
    <template v-for="(arg, ai) in node.args" :key="`f${ai}`">
      <span v-if="ai > 0" class="mn-fn__comma">,</span>
      <span class="mn-fn__arg">
        <MathNodeView v-for="(n, i) in arg" :key="`fa${i}`" :node="n" />
      </span>
    </template>
    <span class="mn-fn__paren">)</span>
  </span>

  <!-- 括号分组 ( items ) -->
  <span v-else-if="node.type === 'group'" class="mn-group">
    <span class="mn-group__paren">(</span>
    <span class="mn-group__inner">
      <MathNodeView v-for="(n, i) in node.items" :key="`g${i}`" :node="n" />
    </span>
    <span class="mn-group__paren">)</span>
  </span>

  <!-- 未知节点兜底：渲染成原始文本，避免整段消失 -->
  <span v-else class="mn mn--normal">{{ JSON.stringify(node) }}</span>
</template>

<style scoped>
.mn {
  white-space: nowrap;
}

/* ---- 文本按 kind 上色 ---- */
.mn--normal {
  color: var(--c-text);
}
.mn--number {
  color: var(--c-math-number);
}
.mn--symbol {
  color: var(--c-math-symbol);
}
.mn--function {
  color: var(--c-math-function);
}
.mn--operator {
  color: var(--c-math-operator);
}

/* ---- 分数 ---- */
.mn-fraction {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  vertical-align: middle;
  margin: 0 2px;
  text-align: center;
}
.mn-fraction__num {
  display: inline-flex;
  padding: 0 4px;
}
.mn-fraction__rule {
  width: 100%;
  min-width: 12px;
  height: 1px;
  background: var(--c-math-bracket);
  margin: 1px 0;
}
.mn-fraction__den {
  display: inline-flex;
  padding: 0 4px;
}

/* ---- 上标 ---- */
.mn-super {
  display: inline-flex;
  align-items: flex-start;
  white-space: nowrap;
}
.mn-super__base {
  display: inline-flex;
}
.mn-super__exp {
  display: inline-flex;
  font-size: 0.72em;
  line-height: 1;
  margin-left: 1px;
  transform: translateY(-0.18em);
}

/* ---- 根号 ---- */
.mn-radical {
  display: inline-flex;
  align-items: flex-start;
  white-space: nowrap;
}
.mn-radical__idx {
  font-size: 0.62em;
  line-height: 1;
  transform: translateY(0.1em);
  margin-right: 1px;
  color: var(--c-math-bracket);
}
.mn-radical__sign {
  font-size: 1.05em;
  line-height: 1;
  color: var(--c-math-bracket);
}
.mn-radical__body {
  display: inline-flex;
  align-items: center;
  border-top: 1px solid var(--c-math-bracket);
  padding: 1px 3px 0;
  margin-top: 0.12em;
}

/* ---- 绝对值 ---- */
.mn-abs {
  display: inline-flex;
  align-items: stretch;
  white-space: nowrap;
}
.mn-abs__bar {
  width: 1px;
  background: var(--c-math-bracket);
  margin: 2px 1px;
}
.mn-abs__inner {
  display: inline-flex;
  align-items: center;
}

/* ---- 函数 ---- */
.mn-fn {
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
}
.mn-fn__name {
  color: var(--c-math-function);
  font-style: italic;
}
.mn-fn__paren,
.mn-fn__comma {
  color: var(--c-math-bracket);
}
.mn-fn__arg {
  display: inline-flex;
}

/* ---- 分组 ---- */
.mn-group {
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
}
.mn-group__paren {
  color: var(--c-math-bracket);
}
.mn-group__inner {
  display: inline-flex;
  align-items: center;
}
</style>

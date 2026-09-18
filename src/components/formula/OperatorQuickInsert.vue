<script setup lang="ts">
/**
 * 运算符快插 —— 在参数输入框末尾追加运算符。
 *
 * ## ⚠️ 只能插 ASCII 运算符
 *
 * 引擎的词法分析（`engine/lexer.rs`）只认这些字符：
 *
 * ```
 * +  -  *  /  ^  %  (  )  ,
 * ```
 *
 * 插入 `×` 或 `÷` 会直接报「未知字符: '×' at position N」。
 * 所以按钮上**显示什么就插入什么** —— 不显示 `×` 而插 `*`，
 * 否则用户会疑惑「我点的是 ×，框里怎么是 *」。
 * 工程用户对 `*` `/` 很熟悉（Excel 也是这套）。
 *
 * `√` 是唯一的例外：它无法用单字符 ASCII 表达，插入 `sqrt(` ——
 * tooltip 里写明，避免用户看到括号困惑。
 *
 * ## 为什么不放 `%`
 *
 * 引擎的 `%` 是**取模**（`7%3 = 1`），不是百分号。
 * 放上来会让想输「50%」的用户踩坑（`50%` 会因缺右操作数而报错）。
 * 需要取模的场景极少，用户手打即可。
 */
import { OPERATORS } from './operators'

const emit = defineEmits<{
  /** 用户点了某个运算符（`insert` 是要插入到输入框的文本） */
  (e: 'insert', text: string): void
}>()
</script>

<template>
  <div class="ops" role="group" aria-label="运算符快插">
    <el-tooltip
      v-for="op in OPERATORS"
      :key="op.insert"
      :content="op.tip"
      placement="top"
      :show-after="400"
    >
      <button
        class="ops__btn"
        type="button"
        :aria-label="op.tip"
        @mousedown.prevent
        @click="emit('insert', op.insert)"
      >
        {{ op.label }}
      </button>
    </el-tooltip>
  </div>
</template>

<style scoped>
.ops {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}

.ops__btn {
  min-width: 26px;
  height: 26px;
  padding: 0 var(--sp-1);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  background: var(--c-surface);
  color: var(--c-text-2);
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  line-height: 1;
  cursor: pointer;
}

.ops__btn:hover {
  border-color: var(--c-primary);
  color: var(--c-primary);
}

.ops__btn:active {
  background: var(--c-primary-light);
}
</style>

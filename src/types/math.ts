/**
 * 组 12b：二维数学排版布局树。
 *
 * 见 docs/08-IPC契约.md §3 组 12 与 ADR-019。
 *
 * ## 设计：后端只出数据，前端负责画
 *
 * 后端把一维表达式（`x = (b1*a22 - a12*b2)/(a11*a22 - a12*a21)`）按运算树
 * 整理成**布局树**；文本测量与绘制全在前端（浏览器最擅长这个）。
 *
 * ## ⚠️ 两个反直觉点
 *
 * 1. **没有 `subscript` 节点** —— 下标是后端在**文本层**做的 Unicode 替换
 *    （`a11` → `a₁₁`），渲染层只管画字符串
 * 2. **没有 `row` 节点** —— `MathNode[]` 序列本身就是一行
 *
 * ## 🔴 降级约定（必须遵守）
 *
 * 排版失败**不报错**。`mathLayout` 失败时坏段不进 `lines` 但记进 `warnings`；
 * `mathLayoutEquation` 失败时返回 `null`。
 * **两种情况都必须回落成等宽原文展示** —— 绝不静默丢内容。
 */

/** 文本节点的语义分类（用于上色） */
export type TextKind = 'normal' | 'number' | 'symbol' | 'operator' | 'function'

/**
 * 排版树节点。判别字段是 `type`（`Text` 的样式字段是 `kind`，与 `type` 不冲突）。
 */
export type MathNode =
  /** 普通文本；`text` 已做显示美化（`pi`→`π`、`a11`→`a₁₁`） */
  | { type: 'text'; text: string; kind: TextKind }
  /** 分数：分子 / 分母 */
  | { type: 'fraction'; numerator: MathNode[]; denominator: MathNode[] }
  /** 上标（幂） */
  | { type: 'superscript'; base: MathNode[]; exponent: MathNode[] }
  /** 根号；`index` 非空表示 n 次根（`cbrt` → `index: 3`） */
  | { type: 'radical'; radicand: MathNode[]; index?: number | null }
  /** 绝对值 `|x|` */
  | { type: 'absolute'; inner: MathNode[] }
  /** 函数调用：`name(arg, arg)` */
  | { type: 'function'; name: string; args: MathNode[][] }
  /** 括号分组（一期未产出，保留位） */
  | { type: 'group'; items: MathNode[] }

/** 排版后的一行 */
export interface MathLine {
  /** 赋值目标（多结果公式每段一行） */
  symbol?: string | null
  /** 该行的节点串 */
  nodes: MathNode[]
  /** 行尾编号（`①`/`②`，方程组用） */
  mark?: string | null
}

/** 整条公式的排版结果 */
export interface MathLayout {
  lines: MathLine[]
  /**
   * 降级警告。**非空时前端应把对应段落回落成等宽原文**
   * （`lines` 里已经不含那些坏段了）。
   */
  warnings: string[]
}

/** `math_layout_result_line` 的入参一项 */
export interface ResultPair {
  /** 结果符号（如 `x`） */
  symbol: string
  /** 数值文本 —— **由调用方格式化**（排版层不管精度口径） */
  value: string
}

/**
 * 判据：能否用排版树渲染。
 *
 * `false` 时必须回落成等宽原文。
 */
export function isUsable(layout: MathLayout | null | undefined): boolean {
  return !!layout && layout.lines.length > 0
}

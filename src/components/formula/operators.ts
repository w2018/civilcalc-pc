/**
 * 运算符快插按钮的定义。
 *
 * 单独成文件（而不是写在组件里）是为了让**测试与其他视图**也能引用，
 * 并保证「按钮顺序」只有一处定义。
 *
 * ## `insert` 必须是 ASCII
 *
 * 见 `OperatorQuickInsert.vue` 的说明 —— 引擎只认
 * `+ - * / ^ % ( ) ,`。这里刻意**不提供** `%`（那是取模，不是百分号）。
 */

export interface OperatorDef {
  /** 按钮上显示的字符 */
  label: string
  /** 实际插入输入框的文本 */
  insert: string
  /** tooltip 说明 */
  tip: string
}

export const OPERATORS: OperatorDef[] = [
  { label: '+', insert: '+', tip: '加' },
  { label: '-', insert: '-', tip: '减' },
  { label: '*', insert: '*', tip: '乘（引擎不支持 ×，请用 *）' },
  { label: '/', insert: '/', tip: '除（引擎不支持 ÷，请用 /）' },
  { label: '^', insert: '^', tip: '幂（2^3 = 8）' },
  { label: '(', insert: '(', tip: '左括号' },
  { label: ')', insert: ')', tip: '右括号' },
  { label: '√', insert: 'sqrt(', tip: '开方（插入 sqrt( ，需自行补右括号）' },
]

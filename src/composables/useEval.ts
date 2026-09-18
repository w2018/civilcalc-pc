/**
 * 实时求值：防抖 + 请求序号防竞态。
 *
 * 见 docs/05-项目开发方案.md §3.3.3 与 §3.3.4。
 *
 * ## 为什么用 `eval_schema` 而不是 `eval_formula`
 *
 * 文档 §3.3.3 的流程图写的是 `eval_formula`，但**同一节的结论是
 * 「参数变动不写 history」**，而 `eval_formula` 的实现会写一条历史。
 * 两者矛盾 —— 这里按「不写历史」的意图实现，理由：
 *
 * 1. 用户拖一次滑块会触发十几轮防抖求值，每轮写一条历史 = 历史页被灌满
 * 2. `eval_schema` 传的是**内存里的 schema**，对「编辑中预览」语义更准
 *    （`eval_formula` 从库读，用户改了没保存时预览的是旧版）
 *
 * 「确认计算」（点保存 / 导出）才调 `eval_formula` 写历史 —— 见
 * `stores/formula.ts` 的 `confirmCompute()`。
 *
 * ## 为什么用序号而不是 `AbortController`
 *
 * Tauri 的 `invoke` **不支持取消**（没有 `AbortSignal` 参数）。
 * 递增序号比对是唯一可靠的方案：新请求发出时序号 +1，
 * 响应回来时序号对不上就丢弃。
 *
 * 典型踩坑场景：用户快速输入 `1` → `12` → `123`，
 * 若「12」的响应晚于「123」到达，结果面板会显示 12 的结果。
 */

import { ref, shallowRef } from 'vue'
import { evalApi } from '@/api'
import {
  evalErrorSymbol,
  type EvalError,
  type EvalResult,
  type FormulaSchema,
} from '@/types/domain'
import type { CommandError } from '@/types/error'

/** 参数变动后的防抖时长（docs §3.3.3） */
export const EVAL_DEBOUNCE_MS = 300

export type EvalStatus = 'idle' | 'loading' | 'success' | 'error'

/**
 * 从求值错误里取出「该聚焦哪个字段」。
 *
 * 只有 `domain` 错误带 `symbol`；编译错误是整条表达式的问题，
 * 没有单一字段可聚焦（返回 `null`，由调用方聚焦整个表达式框）。
 */
export function errorFocusKey(err: EvalError | null): string | null {
  return evalErrorSymbol(err)
}

/**
 * 求值器（自包含状态）。
 *
 * 在 `stores/formula.ts` 里实例化一次，组件通过 store 使用 ——
 * 这样防抖计时器与序号在整个应用里只有一份。
 */
export function useEval() {
  const result = shallowRef<EvalResult | null>(null)
  const status = ref<EvalStatus>('idle')
  /** 命令层错误（网络 / 存储 / 参数非法等） */
  const error = ref<CommandError | null>(null)
  /** 求值错误（定义域 / 超时 / 编译）—— 与 `error` 互斥 */
  const evalError = ref<EvalError | null>(null)
  /** 需要聚焦的字段名（`Domain` 错误时非空） */
  const focusedErrorKey = ref<string | null>(null)

  /** 请求序号：用于丢弃过期响应 */
  let seq = 0
  /** 防抖计时器 */
  let timer: ReturnType<typeof setTimeout> | null = null

  /** 取消待执行的防抖任务（不影响已发出的请求） */
  function cancelPending(): void {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
  }

  /** 清空结果与错误（切换公式 / 重置时用） */
  function reset(): void {
    cancelPending()
    seq += 1 // 让在途请求的结果失效
    result.value = null
    status.value = 'idle'
    error.value = null
    evalError.value = null
    focusedErrorKey.value = null
  }

  /**
   * 立即求值（不等防抖）。
   *
   * @param schema 当前公式（内存里的，可能未保存）
   * @param inputs 参数值（已解析为数字）
   */
  async function evaluateNow(
    schema: FormulaSchema,
    inputs: Record<string, number>,
  ): Promise<EvalResult | null> {
    const mine = ++seq
    status.value = 'loading'

    try {
      const r = await evalApi.evalSchema(schema, inputs)
      if (mine !== seq) return null // 有更新的请求发出，本次结果作废
      result.value = r
      status.value = 'success'
      error.value = null
      evalError.value = null
      focusedErrorKey.value = null
      return r
    } catch (e) {
      if (mine !== seq) return null
      const ce = e as CommandError
      if (ce.kind === 'parse' || ce.kind === 'validation') {
        // 表达式编译失败 / 校验失败：不是「命令失败」，而是用户还没写完
        evalError.value = { kind: 'compile', msg: ce.message }
        error.value = null
        focusedErrorKey.value = null
      } else {
        error.value = ce
        evalError.value = null
        focusedErrorKey.value = null
      }
      status.value = 'error'
      result.value = null
      return null
    }
  }

  /**
   * 防抖求值（参数变动时调）。
   *
   * 连续调用只会执行最后一次 —— 计时器被重置。
   */
  function scheduleEvaluate(
    schema: FormulaSchema,
    inputs: Record<string, number>,
    delay = EVAL_DEBOUNCE_MS,
  ): void {
    cancelPending()
    status.value = 'loading'
    timer = setTimeout(() => {
      timer = null
      void evaluateNow(schema, inputs)
    }, delay)
  }

  return {
    result,
    status,
    error,
    evalError,
    focusedErrorKey,
    reset,
    evaluateNow,
    scheduleEvaluate,
    cancelPending,
  }
}

/**
 * 求值一个「算式型输入」（如用户在参数框里写 `1/2 + 0.5`）。
 *
 * 源项目 `FormulaViewModel.evaluateExpression(raw)` 的等价物：
 * 构造一个只有表达式、没有变量的最小 schema，交给后端引擎算。
 *
 * ## 为什么走后端而不是前端自己算
 *
 * 前端的 JS `eval` 与后端引擎的**运算符优先级、隐式乘法、常量表**
 * 未必一致（例如后端支持 `2r` 表示 `2*r`、`π` 是常量）。
 * 自己算会出现「输入框显示 3.14，结果面板按 3.1415926 算」这类诡异现象。
 *
 * @returns 求值结果；表达式不合法时返回 `null`（调用方保留原文并提示）
 */
export async function evalExpression(raw: string): Promise<number | null> {
  const expr = raw.trim()
  if (!expr) return null

  // 纯数字直接返回，省一次 IPC（绝大多数输入都是这种情况）
  const plain = Number(expr)
  if (Number.isFinite(plain)) return plain

  const minimal: FormulaSchema = {
    id: '__expr__',
    resultName: '表达式',
    resultSymbol: 'R',
    resultOutputs: [],
    expression: expr,
    sourceEquations: [],
    altExpressions: [],
    constants: {},
    variables: [],
    domain: '',
    tags: [],
    imageIds: [],
    source: { kind: 'CUSTOM', verified: false },
    schemaVersion: 3,
    createdAt: 0,
    updatedAt: 0,
  }

  try {
    const r = await evalApi.evalSchema(minimal, {})
    return r.primary
  } catch {
    return null
  }
}

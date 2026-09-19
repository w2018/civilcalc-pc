/**
 * 公式工作台的状态与编排。
 *
 * 见 docs/05-项目开发方案.md §3.3.3 / §3.3.4。
 *
 * ## 三类参数状态（与源项目一致）
 *
 * | 状态 | 内容 | 例 |
 * |---|---|---|
 * | `paramValues` | 用户**原始输入**（字符串，可以是算式） | `"12*3"` |
 * | `paramEvaluated` | 失焦后的**求值显示值** | `"36"` |
 * | `inputs` | 解析后的**数值**，送去求值 | `36` |
 *
 * 为什么要区分「原始输入」与「求值显示」：参数框支持四则运算，
 * 用户输入 `12*3` 后失焦，框里显示 `36`；再次聚焦时**恢复显示 `12*3`**，
 * 否则用户想改成 `12*4` 得从头敲。
 *
 * ## 草稿落盘
 *
 * `paramValues` 会持久化（切页面/重启后回来仍在）。
 * 源项目每次按键都写盘；这里加 500ms 防抖 —— 写盘是同步 SQLite 操作，
 * 每次按键都写会在快速输入时明显卡顿。切换公式前会 `flushDraft()`。
 */

import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import { evalApi, formulaApi, historyApi } from '@/api'
import { evalExpression, useEval } from '@/composables/useEval'
import type { CommandError } from '@/types/error'
import {
  parseHistoryInputs,
  type EvalResult,
  type FormulaSchema,
  type FormulaVar,
} from '@/types/domain'

/** 草稿落盘防抖（比求值的 300ms 长：写盘更贵，且不需要那么及时） */
const DRAFT_DEBOUNCE_MS = 500

/** 参数解析结果 */
interface ParsedParams {
  inputs: Record<string, number>
  errors: Record<string, string>
}

export const useFormulaStore = defineStore('formula', () => {
  // ---------------------------------------------------------------- 公式本体

  const schema = shallowRef<FormulaSchema | null>(null)
  const loading = ref(false)
  /** 公式加载失败（不存在 / 读库失败） */
  const loadError = ref<CommandError | null>(null)

  // ---------------------------------------------------------------- 工作台占用标记
  //
  // 🔴 这几个字段**故意不被 `reset()` 清掉**。
  //
  // 离开「公式计算」页时 `reset()` 会把 `schema` 清空 —— 于是路由守卫
  // 再也没法回答「工作台里现在开着哪条公式」。而「从公式库/历史/收藏/
  // 新生成的公式跳进来会不会顶掉正在用的公式」这个判断（需求 6）
  // 恰恰要在**离开之后**才能做。
  //
  // 所以把「最近打开的是哪条公式的哪条历史」单独留一份，只服务于这个判断。

  /** 最近一次在「公式计算」里打开过的公式 id（空串 = 从没打开过） */
  const lastWorkspaceId = ref('')
  /** 对应名称（弹窗文案用） */
  const lastWorkspaceName = ref('')
  /**
   * 最近打开的是那条公式的**哪条历史**（空串 = 不是从历史进来的）。
   *
   * ⚠️ 路由守卫必须连它一起比：同一条公式换一条历史，同样会**整份替换**
   * 工作台内容（`FormulaView.load()` 会用那条快照重建参数）。
   * 只比公式 id 会把这种切换当成「没换」，静默丢掉当前输入。
   */
  const lastWorkspaceHistoryId = ref('')

  // ---------------------------------------------------------------- 参数

  /** 用户原始输入（字符串，可为算式） */
  const paramValues = ref<Record<string, string>>({})
  /** 失焦后的求值显示值（仅用于「显示」，不参与计算） */
  const paramEvaluated = ref<Record<string, string>>({})
  /** 字段级错误：`symbol → 中文提示` */
  const paramErrors = ref<Record<string, string>>({})

  // ---------------------------------------------------------------- 求值

  const evaluator = useEval()

  // ---------------------------------------------------------------- 草稿

  let draftTimer: ReturnType<typeof setTimeout> | null = null
  /** 待落盘的草稿（`null` 表示没有待写内容） */
  let pendingDraft: string | null = null

  // ================================================================ 派生

  /** 变量列表（`schema` 为空时给空数组，避免调用方到处判空） */
  const variables = computed<FormulaVar[]>(() => schema.value?.variables ?? [])

  /** 是否有未填的必填项 */
  const hasMissingRequired = computed(() =>
    variables.value.some((v) => v.required && !(paramValues.value[v.symbol] ?? '').trim()),
  )

  /** 是否处于可计算状态（无字段错误、无必填缺失） */
  const canCompute = computed(
    () => schema.value !== null && !hasMissingRequired.value && Object.keys(paramErrors.value).length === 0,
  )

  // ================================================================ 参数解析

  /**
   * 把 `paramValues` 解析成数值。
   *
   * 逐个变量处理：
   * 1. 空值：必填 → 报错；非必填 → **跳过**（不进 `inputs`，后端按缺失处理）
   * 2. 纯数字 → 直接用
   * 3. 算式（如 `12*3`）→ 交后端引擎求值（`evalExpression`）
   * 4. 仍失败 → 报错「表达式无法求值」
   * 5. 边界校验：`min` / `max`
   *
   * ⚠️ 边界校验在**两处**发生（文档 P2-9 要求）：失焦时（即时反馈）
   * 与这里（防抖求值前）。重复校验是有意的 —— 用户可能不触发失焦
   * （如用 Tab 键直接跳走、或粘贴后立即点计算）。
   */
  async function parseParams(): Promise<ParsedParams> {
    const inputs: Record<string, number> = {}
    const errors: Record<string, string> = {}

    for (const v of variables.value) {
      const raw = (paramValues.value[v.symbol] ?? '').trim()

      if (!raw) {
        if (v.required) errors[v.symbol] = `${v.desc} 为必填`
        continue
      }

      const value = await evalExpression(raw)
      if (value === null || !Number.isFinite(value)) {
        errors[v.symbol] = '表达式无法求值，请检查'
        continue
      }

      // 边界校验（闭区间）
      if (v.min !== null && v.min !== undefined && value < v.min) {
        errors[v.symbol] = `不能小于 ${v.min}`
        continue
      }
      if (v.max !== null && v.max !== undefined && value > v.max) {
        errors[v.symbol] = `不能大于 ${v.max}`
        continue
      }

      inputs[v.symbol] = value
    }

    return { inputs, errors }
  }

  // ================================================================ 动作

  /** 取消所有待执行任务（切换公式时调） */
  function cancelTimers(): void {
    evaluator.cancelPending()
    if (draftTimer !== null) {
      clearTimeout(draftTimer)
      draftTimer = null
    }
  }

  /**
   * 立即把待写草稿落盘（切换公式 / 离开页面前调）。
   *
   * 失败只记日志 —— 草稿丢了不该打断用户。
   */
  async function flushDraft(): Promise<void> {
    if (draftTimer !== null) {
      clearTimeout(draftTimer)
      draftTimer = null
    }
    const id = schema.value?.id
    if (pendingDraft === null || !id) return
    const json = pendingDraft
    pendingDraft = null
    try {
      await formulaApi.formulaDraftSave(id, json)
    } catch (e) {
      console.warn('[formula] 草稿落盘失败', e)
    }
  }

  /** 安排一次草稿落盘（防抖） */
  function scheduleDraft(): void {
    const id = schema.value?.id
    if (!id) return
    pendingDraft = JSON.stringify(paramValues.value)

    if (draftTimer !== null) clearTimeout(draftTimer)
    draftTimer = setTimeout(() => {
      draftTimer = null
      void flushDraft()
    }, DRAFT_DEBOUNCE_MS)
  }

  /** 触发一次防抖求值（先解析参数，再交给求值器） */
  async function triggerEvaluate(): Promise<void> {
    const s = schema.value
    if (!s) return

    const { inputs, errors } = await parseParams()
    paramErrors.value = errors

    // 参数本身有错时不发请求 —— 后端会返回校验错误，白白多一次 IPC
    if (Object.keys(errors).length > 0) {
      evaluator.reset()
      return
    }
    evaluator.scheduleEvaluate(s, inputs)
  }

  /**
   * 加载公式（含草稿恢复 / 历史回填）。
   *
   * 参数预填的**优先级**：
   *
   * 1. **历史回填**（传了 `historyId` 时）—— 用户明确点了「用这次结果复现」
   * 2. **草稿**（上次编辑留下的输入）
   * 3. **变量默认值**（`default`）
   * 4. 留空
   *
   * ⚠️ 历史优先于草稿是**刻意的**：用户从历史页点进来，意图是
   * 「复现那一次计算」，此时用草稿覆盖会让他看到别的数字。
   */
  async function load(id: string, historyId?: number | null): Promise<void> {
    await flushDraft()
    cancelTimers()
    evaluator.reset()
    loadError.value = null
    loading.value = true
    schema.value = null
    paramValues.value = {}
    paramEvaluated.value = {}
    paramErrors.value = {}

    try {
      const s = await formulaApi.formulaGet(id)
      if (!s) {
        loadError.value = { kind: 'notFound', message: `公式不存在：${id}` }
        return
      }
      schema.value = s

      // 历史回填（最高优先级）
      let fromHistory: Record<string, number> | null = null
      if (historyId !== null && historyId !== undefined) {
        try {
          const h = await historyApi.historyGet(historyId)
          // 取不到（已被清理）时静默回落 —— 不该因为一条历史没了就打不开公式
          if (h) fromHistory = parseHistoryInputs(h)
        } catch (e) {
          console.warn('[formula] 读历史失败，回落草稿', e)
        }
      }

      // 草稿（次优先）
      let draft: Record<string, string> | null = null
      if (!fromHistory) {
        try {
          const json = await formulaApi.formulaDraftLoad(id)
          if (json) draft = JSON.parse(json) as Record<string, string>
        } catch (e) {
          // 草稿坏了不该让公式打不开 —— 回落默认值
          console.warn('[formula] 草稿解析失败，回落默认值', e)
        }
      }

      const next: Record<string, string> = {}
      for (const v of s.variables) {
        const hv = fromHistory?.[v.symbol]
        const fromDraft = draft?.[v.symbol]
        if (typeof hv === 'number' && Number.isFinite(hv)) {
          next[v.symbol] = String(hv)
        } else if (typeof fromDraft === 'string') {
          next[v.symbol] = fromDraft
        } else if (v.default !== null && v.default !== undefined) {
          next[v.symbol] = String(v.default)
        } else {
          next[v.symbol] = ''
        }
      }
      paramValues.value = next

      // 记下「正在用的是哪条公式的哪条历史」—— 供路由守卫判断是否会被顶掉（需求 6）
      lastWorkspaceId.value = s.id
      lastWorkspaceName.value = s.resultName || s.id
      lastWorkspaceHistoryId.value = historyId == null ? '' : String(historyId)

      // 首次求值不防抖（用户刚进来，等 300ms 是白等）
      await triggerEvaluate()
    } catch (e) {
      loadError.value = e as CommandError
    } finally {
      loading.value = false
    }
  }

  /**
   * 修改一个参数（输入框 `input` 事件）。
   *
   * - 更新原始值
   * - **清掉该字段的旧错误**（用户正在改，错误提示先撤掉）
   * - 清掉该字段的求值显示（输入框应显示原文）
   * - 防抖求值 + 防抖存草稿
   */
  function setParam(symbol: string, raw: string): void {
    paramValues.value = { ...paramValues.value, [symbol]: raw }

    if (paramErrors.value[symbol]) {
      const next = { ...paramErrors.value }
      delete next[symbol]
      paramErrors.value = next
    }
    if (paramEvaluated.value[symbol]) {
      const next = { ...paramEvaluated.value }
      delete next[symbol]
      paramEvaluated.value = next
    }

    scheduleDraft()
    void triggerEvaluate()
  }

  /**
   * 失焦：把算式求值并记到 `paramEvaluated`（仅供显示）。
   *
   * 同时做一次**即时**边界校验 —— 这是文档要求的「失焦 + 防抖求值两处校验」
   * 中的前一处，比防抖求值快 300ms，用户能立刻看到问题。
   *
   * @returns 求值后的显示值；无法求值返回 `null`
   */
  async function evaluateOnBlur(symbol: string): Promise<string | null> {
    const raw = (paramValues.value[symbol] ?? '').trim()
    if (!raw) return null

    const value = await evalExpression(raw)
    if (value === null || !Number.isFinite(value)) {
      paramErrors.value = { ...paramErrors.value, [symbol]: '表达式无法求值，请检查' }
      return null
    }

    const v = variables.value.find((x) => x.symbol === symbol)
    if (v) {
      if (v.min !== null && v.min !== undefined && value < v.min) {
        paramErrors.value = { ...paramErrors.value, [symbol]: `不能小于 ${v.min}` }
        return null
      }
      if (v.max !== null && v.max !== undefined && value > v.max) {
        paramErrors.value = { ...paramErrors.value, [symbol]: `不能大于 ${v.max}` }
        return null
      }
    }

    // 只有确实是个算式（与原文不同）才记显示值，避免纯数字时多此一举
    const shown = String(value)
    if (shown !== raw) {
      paramEvaluated.value = { ...paramEvaluated.value, [symbol]: shown }
    }
    if (paramErrors.value[symbol]) {
      const next = { ...paramErrors.value }
      delete next[symbol]
      paramErrors.value = next
    }
    return shown
  }

  /** 运算符快插：在原始输入末尾追加（基于原文，不是求值显示值） */
  function appendOperator(symbol: string, op: string): void {
    setParam(symbol, (paramValues.value[symbol] ?? '') + op)
  }

  /**
   * **确认计算** —— 这是唯一写历史的入口。
   *
   * 参数变动的实时求值走 `eval_schema`（不写历史）；
   * 用户点「保存 / 导出」时调这里，走 `eval_formula` 写一条历史。
   *
   * @param thinkingContent AI 解析时的思考内容（手动创建时为 `undefined`）
   */
  async function confirmCompute(thinkingContent?: string): Promise<EvalResult | null> {
    const id = schema.value?.id
    if (!id) return null

    const { inputs, errors } = await parseParams()
    paramErrors.value = errors
    if (Object.keys(errors).length > 0) return null

    await flushDraft()
    try {
      return await evalApi.evalFormula(id, inputs, thinkingContent)
    } catch (e) {
      console.warn('[formula] 确认计算失败', e)
      return null
    }
  }

  /** 保存公式（并落草稿） */
  async function save(): Promise<void> {
    const s = schema.value
    if (!s) return
    await flushDraft()
    await formulaApi.formulaSave(s)
  }

  /**
   * 改公式名并落库。
   *
   * ⚠️ **必须同步 `lastWorkspaceName`** —— 路由守卫弹「要切换公式吗？」时
   * 用的就是它（见上方占用标记），不同步的话改完名弹窗还显示旧名字。
   *
   * 空名 / 与原名相同 → 直接返回，不写库（避免一次无意义的 upsert + 索引重建）。
   */
  async function renameFormula(name: string): Promise<void> {
    const s = schema.value
    if (!s) return
    const next = name.trim()
    if (!next || next === s.resultName) return
    s.resultName = next
    lastWorkspaceName.value = next
    await save()
  }

  /** 用新的 schema 替换当前公式（编辑表达式后） */
  function replaceSchema(next: FormulaSchema): void {
    schema.value = next
  }

  /**
   * 重置全部状态（离开工作台时调）。
   *
   * ⚠️ **不清** `lastWorkspaceId` / `lastWorkspaceName` / `lastWorkspaceHistoryId` ——
   * 它们是路由守卫判断「会不会顶掉正在用的公式」的依据，
   * 必须在离开之后仍然可读。见上方字段注释。
   */
  async function reset(): Promise<void> {
    await flushDraft()
    cancelTimers()
    evaluator.reset()
    schema.value = null
    paramValues.value = {}
    paramEvaluated.value = {}
    paramErrors.value = {}
    loadError.value = null
    loading.value = false
  }

  /** 彻底忘掉工作台占用（「重置软件」后、或当前公式已被删除时调） */
  function forgetWorkspace(): void {
    lastWorkspaceId.value = ''
    lastWorkspaceName.value = ''
    lastWorkspaceHistoryId.value = ''
  }

  return {
    // 公式
    schema,
    loading,
    loadError,
    variables,
    // 工作台占用标记（路由守卫用）
    lastWorkspaceId,
    lastWorkspaceName,
    lastWorkspaceHistoryId,
    forgetWorkspace,
    renameFormula,
    // 参数
    paramValues,
    paramEvaluated,
    paramErrors,
    hasMissingRequired,
    canCompute,
    // 求值
    result: evaluator.result,
    evalStatus: evaluator.status,
    evalError: evaluator.evalError,
    commandError: evaluator.error,
    focusedErrorKey: evaluator.focusedErrorKey,
    // 动作
    load,
    setParam,
    evaluateOnBlur,
    appendOperator,
    triggerEvaluate,
    confirmCompute,
    save,
    replaceSchema,
    flushDraft,
    reset,
  }
})

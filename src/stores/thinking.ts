/**
 * 「刚生成出来的思考过程」暂存（**仅本次会话**）。
 *
 * 需求 4：AI 生成的公式要能查看思考过程。
 *
 * ## 为什么需要一个专门的暂存
 *
 * 思考内容**不在公式上**（`FormulaSchema` 没有这个字段），
 * 它落在**计算历史**的 `thinkingContent` 里 —— 而历史只在
 * 「记录本次计算」时才写一条。
 *
 * 于是刚生成完、还没算过的那段时间里，公式页拿不到任何思考内容
 * （`history_thinking` 返回 `null`），用户会觉得「AI 想了什么我不知道」。
 *
 * 这里补上这段空窗：`QueryView` / `RefineDialog` 生成成功后按
 * 公式 id 存一份，公式页读不到历史时回落到它。
 *
 * ## 为什么不做持久化
 *
 * 落盘的时机是「用户确认这次计算」——那时思考会随历史一起进库
 * （`FormulaView` 的自动记录 / 「记录本次计算」都会带上它）。
 * 如果在这里也写盘，会出现「有思考但没有任何计算」的孤儿数据，
 * 而历史页是按计算组织的，没地方展示它。
 *
 * 所以：**内存暂存 + 首次记录时随历史落库**。
 * 应用重启后暂存没了，但那时历史里已经有了。
 */
import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useThinkingStore = defineStore('thinking', () => {
  /** `公式 id → 思考全文` */
  const byFormula = ref<Record<string, string>>({})

  /** 记下某个公式「刚生成时」的思考（空串会被忽略） */
  function remember(formulaId: string, thinking: string): void {
    const id = formulaId.trim()
    const text = thinking.trim()
    if (!id || !text) return
    byFormula.value = { ...byFormula.value, [id]: text }
  }

  /** 取某个公式的思考（没有时返回空串） */
  function recall(formulaId: string): string {
    return byFormula.value[formulaId.trim()] ?? ''
  }

  /** 丢弃某个公式的暂存（已随历史落库后调，避免白占内存） */
  function forget(formulaId: string): void {
    const id = formulaId.trim()
    if (!(id in byFormula.value)) return
    const next = { ...byFormula.value }
    delete next[id]
    byFormula.value = next
  }

  /** 清空（「重置软件」时调） */
  function clear(): void {
    byFormula.value = {}
  }

  return { byFormula, remember, recall, forget, clear }
})

/**
 * 输入历史 —— 为「新建公式」「模型测试」「公式微调」三处输入各存一份历史，
 * 点击历史图标可以选一条把**文本与图片**一起恢复到输入框（需求 5）。
 *
 * ## 存哪
 *
 * 存**偏好**（`config.json`），键是后端早就预留好的三个
 * （见 `src-tauri/src/config.rs`）：
 *
 * | 键 | 用于 |
 * |---|---|
 * | `search_input_history` | 新建公式 |
 * | `refine_input_history` | 公式微调 |
 * | `model_test_input_history` | 模型测试 |
 *
 * 放偏好而不是建表，与参数草稿（`formula_drafts`）同一思路：
 * 这是**临时输入**，不是业务数据，跟着偏好一起走、一起被「重置」清掉。
 *
 * ## 🔴 写入必须串行化 —— 用共享的队列
 *
 * `config_save` 是**整体覆盖**（先 `configGet` → 改 → 整份提交）。
 * 三个页面各写各的键，看着互不相干 —— 但只要两次「读-改-写」交错，
 * 后提交的那份就会把前一份的改动**整片盖掉**（因为提交的是整个快照）。
 *
 * 队列已抽到 `utils/prefsWrite.ts`（**全局共用一条**）——
 * 之前只在本文件里串行化，别的模块（如模型测试的上下文设置）各写各的，
 * 照样会互相覆盖。
 *
 * ## 什么时候记
 *
 * **提交成功时**记（AI 解析成功 / 微调成功 / 模型测试发送成功），
 * 不是每次击键。否则历史会被半截输入塞满。
 */
import { ref, type Ref } from 'vue'
import { systemApi } from '@/api'
import { updatePrefs } from '@/utils/prefsWrite'
import type { ConfigSnapshot } from '@/types/system'

/** 一条输入历史 */
export interface InputHistoryItem {
  /** 输入框文本 */
  text: string
  /** 当时的附图 id（顺序即附图顺序） */
  imageIds: string[]
  /** 记录时间（毫秒） */
  at: number
}

/** 每条历史的文本上限（防止一条超长输入把偏好撑爆） */
const MAX_TEXT_CHARS = 4000

/** 从快照里读某个键的历史（解析失败当空） */
function readItems(snap: ConfigSnapshot, key: string): InputHistoryItem[] {
  const raw = snap.strings[key]
  if (!raw) return []
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter(
        (x): x is InputHistoryItem =>
          typeof x === 'object' &&
          x !== null &&
          typeof (x as InputHistoryItem).text === 'string',
      )
      .map((x) => ({
        text: x.text,
        imageIds: Array.isArray(x.imageIds) ? x.imageIds : [],
        at: typeof x.at === 'number' ? x.at : 0,
      }))
  } catch {
    // 坏数据当空处理 —— 不能因为一条历史坏了就让输入框打不开
    return []
  }
}

export interface UseInputHistory {
  /** 历史列表（新→旧） */
  items: Ref<InputHistoryItem[]>
  /** 正在读 */
  loading: Ref<boolean>
  /** 重新从磁盘读一次 */
  load: () => Promise<void>
  /** 记一条（提交成功时调；与上一条完全相同则跳过） */
  push: (text: string, imageIds?: string[]) => Promise<void>
  /**
   * 删掉第 `index` 条（需求 6：单条删除）。
   *
   * 按**下标**而不是按内容删：同一段文本可能被记过两次
   * （中间夹了别的记录），按内容删会把两条一起删掉。
   */
  removeAt: (index: number) => Promise<void>
  /** 清空这份历史（**调用方负责二次确认**，见 `InputHistoryButton`） */
  clear: () => Promise<void>
}

/**
 * @param prefKey 偏好键（用 `PREF_KEYS.searchHistory` 等，**不要手写字符串**）
 * @param max 保留条数（默认 20）
 */
export function useInputHistory(prefKey: string, max = 20): UseInputHistory {
  const items = ref<InputHistoryItem[]>([])
  const loading = ref(false)

  async function load(): Promise<void> {
    loading.value = true
    try {
      const snap = await systemApi.configGet()
      items.value = readItems(snap, prefKey)
    } catch (e) {
      console.warn('[inputHistory] 读取失败', e)
      items.value = []
    } finally {
      loading.value = false
    }
  }

  async function push(text: string, imageIds: string[] = []): Promise<void> {
    const t = text.trim()
    if (!t && imageIds.length === 0) return

    const entry: InputHistoryItem = {
      text: t.slice(0, MAX_TEXT_CHARS),
      imageIds: [...imageIds],
      at: Date.now(),
    }

    // 先更新内存（界面立刻能看到），再排进队列落盘
    const prev = items.value
    // 与最新一条内容相同 → 不重复记（用户反复点「生成」不该刷屏）
    const head = prev[0]
    if (head && head.text === entry.text && sameIds(head.imageIds, entry.imageIds)) {
      return
    }
    items.value = [entry, ...prev].slice(0, max)
    await persist(items.value)
  }

  /** 整份列表落盘（`push` / `removeAt` / `clear` 共用） */
  function persist(list: InputHistoryItem[]): Promise<void> {
    return updatePrefs((snap) => ({
      strings: { ...snap.strings, [prefKey]: JSON.stringify(list) },
      ints: { ...snap.ints },
    }))
  }

  async function removeAt(index: number): Promise<void> {
    const prev = items.value
    if (index < 0 || index >= prev.length) return
    const next = prev.filter((_, i) => i !== index)
    items.value = next
    await persist(next)
  }

  async function clear(): Promise<void> {
    items.value = []
    await persist([])
  }

  return { items, loading, load, push, removeAt, clear }
}

/** 两组 id 是否完全相同（顺序敏感 —— 附图顺序是语义） */
function sameIds(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i])
}

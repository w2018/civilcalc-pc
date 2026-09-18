/**
 * 「当前活跃模型档位」的**全局共享状态**（需求 7）。
 *
 * ## 为什么不能各页各存一份
 *
 * 原来「新建公式」与「模型测试」各自在 `onMounted` 里拉一次
 * `llm_list_profiles` 并存在自己的 `ref` 里。问题有两个：
 *
 * 1. **切了模型别的页不更新** —— 设置页把活跃档位改成 DeepSeek 之后，
 *    这两页显示的还是 Mimo（它们早就挂载完了，不会再拉一次）
 * 2. **`keep-alive` 让问题更明显** —— 页面被缓存后连 `onMounted`
 *    都不会再跑，切回来仍是旧值
 *
 * 所以把「当前是哪一档」提到模块级单例：谁改了谁都看得见，
 * 不需要事件总线，也不需要每页各拉一次。
 *
 * ## 刷新时机
 *
 * - 应用启动后第一次用到时（`ensureLoaded`）
 * - 设置页改完档位/增删档位后**主动**推一次（`applyConfig`）
 * - 页面重新激活时（各页 `onActivated`）—— 兜底：
 *   万一有哪条路径漏了推送，切回来也能纠正
 */
import { computed, ref } from 'vue'
import { llmApi } from '@/api'
import type { LlmConfig, LlmProfile } from '@/types/llm'

/** 档位列表（不含密钥） */
const profiles = ref<LlmProfile[]>([])
/** 活跃档位 id（空串 = 还没读到） */
const activeId = ref('')

/** 是否已经成功拉过一次（避免每页都触发一次 IPC） */
let loaded = false
/** 正在进行的拉取（并发调用共享同一个 Promise） */
let inflight: Promise<void> | null = null

/** 活跃档位（找不到时返回 `null`） */
const activeProfile = computed<LlmProfile | null>(
  () => profiles.value.find((p) => p.id === activeId.value) ?? null,
)

/** 活跃档位名称（空串 = 未配置） */
const activeLabel = computed(() => activeProfile.value?.label ?? '')

/**
 * 从后端拉一次档位配置。
 *
 * @param force 忽略「已经拉过」的短路，强制重拉
 */
export async function refreshActiveModel(force = false): Promise<void> {
  if (loaded && !force) return
  if (inflight) return inflight

  inflight = (async () => {
    try {
      const cfg = await llmApi.llmListProfiles()
      applyConfig(cfg)
      loaded = true
    } catch {
      // 读不到不阻塞页面 —— 真正发送时后端会报明确错误
    } finally {
      inflight = null
    }
  })()

  return inflight
}

/**
 * 直接用一份已知的配置覆盖（设置页增删改后调，省一次往返）。
 *
 * 这是**立即同步**的关键：设置页点「设为活跃」成功后马上调它，
 * 其它页面的模型名就跟着变了，不用等它们重新挂载。
 */
export function applyConfig(cfg: LlmConfig): void {
  profiles.value = cfg.profiles
  activeId.value = cfg.active
}

/** 只改活跃 id（`llm_set_active` 返回后调） */
export function setActiveId(id: string): void {
  activeId.value = id
}

/** 重置（「重置软件」清了模型配置后调） */
export function resetActiveModel(): void {
  profiles.value = []
  activeId.value = ''
  loaded = false
}

export function useActiveModel() {
  return {
    profiles,
    activeId,
    activeProfile,
    activeLabel,
    refreshActiveModel,
    applyConfig,
    setActiveId,
    resetActiveModel,
  }
}

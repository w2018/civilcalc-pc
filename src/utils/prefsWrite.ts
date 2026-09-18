/**
 * 偏好写入队列（**模块级，全局共用**）。
 *
 * ## 🔴 为什么必须有它
 *
 * `config_save` 是**整体覆盖**：前端要先 `config_get` 拿到完整快照、
 * 改掉自己那几个键、再整份提交。
 *
 * 问题在于「读 → 改 → 写」这三步不是原子的。只要两处同时做这件事，
 * 后提交的那份就会把前一份的改动**整片盖掉** —— 因为提交的是**整个快照**，
 * 而它读到的是**旧的**那一份。
 *
 * 症状很有迷惑性：两处各自都「保存成功」，但其中一处的设置过一会儿就变回去了
 * （或者重启后没了）。没有报错、没有日志。
 *
 * ## 解法
 *
 * 把所有偏好写入排进**同一条 promise 链**（串行执行）。
 * 这个队列是模块级的 —— 不管哪个模块调 `updatePrefs`，都走同一条队。
 *
 * ## 什么时候**不要**用它
 *
 * 只写**单个键**且不关心其它键的场景（比如「重置某区块」走
 * `config_reset_section`，那个命令在 Rust 侧直接删键，不需要前端读快照）。
 * 那种情况下多一次往返没意义。
 *
 * ## 失败怎么办
 *
 * 默认**不抛给调用方**（`updatePrefs` 里吞掉并记日志）——
 * 设置保存失败不该打断用户正在做的事。需要感知失败的调用方用
 * `updatePrefsStrict`。
 */
import { systemApi } from '@/api'
import type { ConfigSnapshot } from '@/types/system'

/** 串行链（成功失败都接着跑下一个） */
let chain: Promise<unknown> = Promise.resolve()

/**
 * 「读 → 改 → 整份提交」，并排进队列。
 *
 * 失败只记日志（返回的 promise 永远 resolve）。
 */
export function updatePrefs(mutate: (snap: ConfigSnapshot) => ConfigSnapshot): Promise<void> {
  const run = async (): Promise<void> => {
    try {
      const snap = await systemApi.configGet()
      await systemApi.configSave(mutate(snap))
    } catch (e) {
      console.warn('[prefs] 偏好写入失败', e)
    }
  }
  // 无论前一个成功还是失败，都接着跑下一个
  chain = chain.then(run, run)
  return chain as Promise<void>
}

/**
 * 同 [`updatePrefs`]，但**把错误抛给调用方**（需要提示用户时用）。
 *
 * 注意：它仍然排在队列里，所以不会与其它写入交错。
 */
export function updatePrefsStrict(
  mutate: (snap: ConfigSnapshot) => ConfigSnapshot,
): Promise<void> {
  const run = async (): Promise<void> => {
    const snap = await systemApi.configGet()
    await systemApi.configSave(mutate(snap))
  }
  // 队列里前一个失败不该影响这一个：用 `.then(run, run)` 而非 `.then(run)`
  const next = chain.then(run, run)
  // 链本身要吞掉错误，否则一次失败会让后续所有写入都被拒绝
  chain = next.catch(() => undefined)
  return next
}

/** 只改**字符串**型偏好（其余键原样带过去） */
export function updatePrefStrings(patch: Record<string, string>): Promise<void> {
  return updatePrefs((s) => ({
    strings: { ...s.strings, ...patch },
    ints: { ...s.ints },
  }))
}

/** 只改**整数**型偏好（其余键原样带过去） */
export function updatePrefInts(patch: Record<string, number>): Promise<void> {
  return updatePrefs((s) => ({
    strings: { ...s.strings },
    ints: { ...s.ints, ...patch },
  }))
}

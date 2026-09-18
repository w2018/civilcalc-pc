/**
 * 侧边导航配置 —— 见 docs/05-项目开发方案.md §2.1.1 / §2.1.2。
 *
 * ## 分组依据
 *
 * 按**使用频率**排，不是按功能相似度：
 *
 * 1. **新建** —— 产生公式的唯一入口，最高频
 * 2. **我的公式** —— 收藏、历史、公式库、模型测试（查阅与试模型）
 * 3. **数据与维护** —— 用量、备份、图片缓存（低频但需要能找到）
 * 4. **系统** —— 设置、关于
 *
 * 中间用分割线隔开，视觉上让「新建」区与「维护」区分开。
 *
 * ## 🔴 为什么只剩一个「新建」入口（需求 10 / 11）
 *
 * 原来有 **搜索 / 粘贴 / 描述** 三个入口，都指向「得到一条公式」，
 * 但它们其实是同一件事的三条路径，用户要在三个页面之间做选择：
 *
 * - **「搜索」与顶栏的快捷搜索完全重复** —— 顶栏那个在任何页面都能用
 *   （`Ctrl+K` 唤起），再留一个整页入口只是让人多跳一次
 * - **「粘贴」与「描述」重复** —— 粘贴一段公式原文、或描述一句需求，
 *   本质都是「把这段内容交给 AI 解析」。分成两个页面反而要求用户
 *   先判断自己手里的是哪一种
 *
 * 所以合并为单一入口 **「新建公式」**：一个输入框，
 * 粘公式 / 粘 JSON / 写需求都行，附图和粘贴图片都支持。
 *
 * ## 图标是**内联 SVG**，不是 emoji（需求 9）
 *
 * 原来用 `✨ 🧮 ⭐ …` 这些 emoji。问题是**不受控**：
 * 字形由系统字体决定（换台机器就变样）、全是彩色的（与单色扁平风冲突）、
 * 尺寸与描边粗细都没法统一。
 *
 * 现在 `icon` 存的是**图标名**，由 `components/common/NavIcon.vue`
 * 渲染成内联 SVG —— 用 `currentColor` 跟着主题与选中态走。
 * 改图标只需改那个组件里的路径表。
 */

/**
 * 可用的图标名（与 `NavIcon.vue` 的路径表**逐字对应**）。
 *
 * 用联合类型而不是 `string`：写错名字在 `vue-tsc` 阶段就报错，
 * 而不是运行时静默渲染成空白。
 */
export type NavIconName =
  | 'sparkle'
  | 'calculator'
  | 'star'
  | 'history'
  | 'library'
  | 'flask'
  | 'chart'
  | 'backup'
  | 'image'
  | 'settings'
  | 'info'

/** 导航项 */
export interface NavItem {
  /** 路由名（与 `router/index.ts` 的 `name` 一致） */
  name: string
  /** 跳转路径 */
  to: string
  /** 显示名 */
  label: string
  /**
   * 图标名（见 `components/common/NavIcon.vue` 的路径表）。
   *
   * ⚠️ 是**名字**不是字形 —— 别再填 emoji，填了会渲染成空白。
   */
  icon: NavIconName
  /**
   * 是否需要公式 ID 才能进入。
   *
   * 这类入口在**没有当前公式**时应当禁用（点了会 404），
   * 由 `SideNav` 根据当前路由参数决定。
   */
  needsFormulaId?: boolean
  /**
   * 高亮时用**前缀匹配**而不是精确匹配。
   *
   * 默认精确匹配（本应用路由全是平级，前缀匹配会让 `/formula/xxx`
   * 之类子路径误高亮别的项）。只有「入口指向父路径、实际工作在子路径」
   * 的项才需要开它 —— 目前只有 `/formula`（公式计算）。
   */
  matchPrefix?: boolean
}

/** 导航分组 */
export interface NavGroup {
  /** 无障碍用的组标签（不显示） */
  label: string
  items: NavItem[]
}

export const NAV_GROUPS: NavGroup[] = [
  {
    label: '计算',
    items: [
      { name: 'query', to: '/query', label: '新建公式', icon: 'sparkle' },
      {
        name: 'formula',
        to: '/formula',
        label: '公式计算',
        icon: 'calculator',
        // `/formula` 本身是空态，真正在用公式时路径是 `/formula/<id>` ——
        // 导航高亮要用**前缀匹配**（见 SideNav 的 `matchPrefix`）
        matchPrefix: true,
      },
    ],
  },
  {
    label: '我的公式',
    items: [
      { name: 'favorites', to: '/favorites', label: '收藏', icon: 'star' },
      { name: 'history', to: '/history', label: '历史', icon: 'history' },
      { name: 'library', to: '/library', label: '公式库', icon: 'library' },
      { name: 'modelTest', to: '/model-test', label: '模型测试', icon: 'flask' },
    ],
  },
  {
    label: '数据与维护',
    items: [
      { name: 'usage', to: '/usage', label: '用量', icon: 'chart' },
      { name: 'backup', to: '/backup', label: '备份', icon: 'backup' },
      { name: 'images', to: '/images', label: '图片缓存', icon: 'image' },
    ],
  },
  {
    label: '系统',
    items: [
      { name: 'settings', to: '/settings', label: '设置', icon: 'settings' },
      { name: 'about', to: '/about', label: '关于', icon: 'info' },
    ],
  },
]

/**
 * 路由 → 面包屑文案。
 *
 * 只列**会出现在面包屑里**的页面；`/formula/:id` 与 `/versions/:id`
 * 需要动态内容（公式名），由视图自己覆盖，这里给兜底文案。
 */
export const ROUTE_TITLES: Record<string, string> = {
  query: '新建公式',
  formula: '公式计算',
  favorites: '收藏',
  history: '历史',
  library: '公式库',
  versions: '版本',
  modelTest: '模型测试',
  usage: '用量统计',
  backup: '备份与恢复',
  images: '图片缓存',
  settings: '设置',
  about: '关于',
}

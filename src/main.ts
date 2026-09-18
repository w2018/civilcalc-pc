import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import router from './router'
import { initTheme } from './composables/useTheme'

// 设计令牌必须先于 Element 覆盖加载
import './styles/tokens.css'
import './styles/element-override.scss'
import './styles/global.scss'

/**
 * 主题初始化必须在挂载**之前**跑。
 *
 * 挂载后再设 `data-theme` 会让首帧按浅色渲染，暗色主题下出现一瞬白屏。
 * 这里先用 `SYSTEM`（读系统偏好）；真实值由 `App.vue` 挂载后从偏好里
 * 异步拉取并纠正 —— 那一次赶不上首帧。
 */
initTheme('SYSTEM')

const app = createApp(App)

/**
 * 全局错误兜底。
 *
 * 组件里未捕获的异常会冒到这里。**不弹窗** —— 桌面应用的弹窗很打扰，
 * 且多数是渲染细节问题；记到控制台便于排障即可
 * （真正的业务错误已在 `api/invoke.ts` 归一化为 `CommandError`）。
 */
app.config.errorHandler = (err, _instance, info) => {
  console.error('[全局错误]', err, info)
}

/**
 * Element Plus **不在这里全局注册**。
 *
 * 组件与函数式 API 都由构建期插件按需注入（见 `vite.config.ts`）：
 * - 模板里的 `<el-input>` 等 → `unplugin-vue-components`
 * - `ElMessage` / `ElMessageBox` 等 → `unplugin-auto-import`
 *
 * 好处是产物里只含实际用到的 7 个组件 + 2 个函数，
 * 而不是整个 element-plus（915 kB → 约 300 kB）。
 *
 * ⚠️ 因此**不要**再写 `import ElementPlus from 'element-plus'`
 * 或 `import 'element-plus/dist/index.css'` —— 那会把整包拉回来。
 */
app.use(createPinia()).use(router).mount('#app')

<script setup lang="ts">
/**
 * 收藏 —— 见 docs/05-项目开发方案.md §2.1.2。
 *
 * 列表按**收藏时间倒序**（后端保证），所以最近收藏的在最上面。
 *
 * 星标就地取消：取消后**立即从列表移除**（而不是留在原地变成空心星）——
 * 收藏页的语义就是"我收藏的东西"，取消收藏后它就不该在这里了。
 */
import { computed, onActivated, ref } from 'vue'
import { useRouter } from 'vue-router'
import { favoriteApi } from '@/api'
import FormulaRow from '@/components/common/FormulaRow.vue'
import { errorMessage } from '@/types/error'
import type { FormulaSchema } from '@/types/domain'

const router = useRouter()

const items = ref<FormulaSchema[]>([])
const loading = ref(true)
const error = ref('')

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    items.value = await favoriteApi.favoritesList()
  } catch (e) {
    error.value = errorMessage(e as never)
  } finally {
    loading.value = false
  }
}


/**
 * 被 `keep-alive` 缓存后**不会重新挂载**，切回来时数据仍是旧快照。
 * 所以每次激活都重新拉一次 —— 否则在别处新增/删除了记录，
 * 这里要等应用重启才看得到。
 */
onActivated(() => {
  void load()
})

function open(id: string): void {
  router.push({ name: 'formula', params: { id } })
}

/** 取消收藏 → 立即从列表移除 */
async function unfavorite(id: string): Promise<void> {
  try {
    const now = await favoriteApi.toggleFavorite(id)
    if (!now) {
      items.value = items.value.filter((f) => f.id !== id)
      ElMessage.success('已取消收藏')
    }
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

const isEmpty = computed(() => !loading.value && items.value.length === 0)
</script>

<template>
  <div class="view">
    <p v-if="error" class="state state--error">{{ error }}</p>

    <el-skeleton v-else-if="loading" :rows="5" animated />

    <div v-else-if="isEmpty" class="state">
      <p class="state__text">还没有收藏的公式</p>
      <p class="state__hint">在公式工作台或搜索结果里点 ☆ 即可收藏</p>
    </div>

    <div v-else class="panel">
      <FormulaRow
        v-for="f in items"
        :key="f.id"
        :item="f"
        favorite
        @open="open(f.id)"
        @toggle-favorite="unfavorite(f.id)"
      />
    </div>
  </div>
</template>

<style scoped>
.view {
  max-width: none;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
  overflow: hidden;
}

/* 行之间用分割线（行组件自身不带边框） */
.panel > :deep(.row:not(:last-child)) {
  border-bottom: var(--hairline) solid var(--c-divider);
}

.state {
  margin: 0;
  padding: var(--sp-8) var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface);
  text-align: center;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.state--error {
  color: var(--c-danger);
}

.state__text {
  margin: 0;
}

.state__hint {
  margin: var(--sp-2) 0 0;
  font-size: var(--f-size-xs);
}
</style>

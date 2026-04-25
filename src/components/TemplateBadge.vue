<script setup lang="ts">
import { computed } from 'vue';
import type { TemplatePresence } from '../types';

const props = defineProps<{
  template?: TemplatePresence;
  showPath?: boolean;
}>();

type State = 'local' | 'cache' | 'remote' | 'unknown';

const state = computed<State>(() => {
  if (!props.template) return 'unknown';
  if (props.template.localPresent) return 'local';
  if (props.template.cachePresent) return 'cache';
  return 'remote';
});

const label = computed(() => {
  switch (state.value) {
    case 'local': return '本地';
    case 'cache': return '缓存';
    case 'remote': return '需 clone';
    default: return '未知';
  }
});
</script>

<template>
  <span class="template-badge" :data-state="state">
    <span class="dot" />
    {{ label }}
    <span v-if="showPath && template?.localPresent" class="path">{{ template.localPath }}</span>
  </span>
</template>

<style scoped>
.template-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 500;
  padding: 2px 10px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: var(--surface-2);
  color: var(--text-muted);
  white-space: nowrap;
}

.template-badge .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  display: inline-block;
}

.template-badge[data-state='local'] {
  color: var(--success);
  background: rgba(16, 185, 129, 0.10);
  border-color: rgba(16, 185, 129, 0.25);
}

.template-badge[data-state='cache'] {
  color: var(--warning);
  background: rgba(245, 158, 11, 0.10);
  border-color: rgba(245, 158, 11, 0.25);
}

.template-badge[data-state='remote'] {
  color: var(--danger);
  background: rgba(239, 68, 68, 0.10);
  border-color: rgba(239, 68, 68, 0.25);
}

.template-badge .path {
  margin-left: 6px;
  color: var(--text-subtle);
  font-weight: 400;
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 11px;
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}
</style>

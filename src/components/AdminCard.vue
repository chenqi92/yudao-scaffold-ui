<script setup lang="ts">
import { Check } from '@element-plus/icons-vue';
import TemplateBadge from './TemplateBadge.vue';
import type { TemplatePresence } from '../types';

defineProps<{
  title: string;
  description: string;
  selected: boolean;
  template?: TemplatePresence;
}>();

defineEmits<{ (e: 'select'): void }>();
</script>

<template>
  <button type="button" class="admin-card" :class="{ selected }" @click="$emit('select')">
    <div class="head">
      <span class="title">{{ title }}</span>
      <span class="check" v-if="selected"><el-icon><Check /></el-icon></span>
    </div>
    <p class="desc">{{ description }}</p>
    <div v-if="template" class="badge-row">
      <TemplateBadge :template="template" />
    </div>
  </button>
</template>

<style scoped>
.admin-card {
  position: relative;
  text-align: left;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 14px 16px;
  cursor: pointer;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, background 0.15s ease,
    transform 0.15s ease;
  font: inherit;
  color: inherit;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.admin-card:hover {
  border-color: var(--primary-soft-border);
  box-shadow: var(--shadow-md);
  transform: translateY(-1px);
}

.admin-card.selected {
  border-color: var(--primary);
  background: var(--primary-soft);
  box-shadow: 0 0 0 1px var(--primary) inset;
}

.admin-card .head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.admin-card .title {
  font-weight: 600;
  font-size: 14px;
  color: var(--text);
}

.admin-card .check {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--primary);
  color: var(--primary-fg);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  flex-shrink: 0;
}

.admin-card .desc {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.5;
}

.admin-card .badge-row {
  margin-top: 4px;
}
</style>

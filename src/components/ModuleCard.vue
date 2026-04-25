<script setup lang="ts">
import { computed } from 'vue';
import { Check, Lock } from '@element-plus/icons-vue';
import type { ModuleMeta } from '../types';

const props = defineProps<{
  module: ModuleMeta;
  selected: boolean;
  disabled: boolean;
}>();

defineEmits<{ (e: 'toggle'): void }>();

const stateClass = computed(() => ({
  selected: props.selected,
  required: props.module.required,
  disabled: props.disabled
}));
</script>

<template>
  <button type="button" class="module-card" :class="stateClass" @click="$emit('toggle')">
    <div class="head">
      <span class="title">{{ module.title }}</span>
      <span class="check" v-if="selected || module.required">
        <el-icon v-if="module.required && !selected"><Lock /></el-icon>
        <el-icon v-else><Check /></el-icon>
      </span>
    </div>
    <p class="desc">{{ module.description }}</p>
    <div class="tags">
      <el-tag v-if="module.required" type="info" size="small" effect="plain">必选</el-tag>
      <el-tag v-if="module.composite" type="warning" size="small" effect="plain">composite</el-tag>
      <el-tag v-if="module.jdk17Only" type="danger" size="small" effect="plain">JDK 17</el-tag>
    </div>
    <div v-if="module.deps.length" class="deps">依赖 · {{ module.deps.join(' · ') }}</div>
  </button>
</template>

<style scoped>
.module-card {
  position: relative;
  text-align: left;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: 14px 16px;
  cursor: pointer;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease,
    background 0.15s ease;
  font: inherit;
  color: inherit;
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.module-card:hover:not(.disabled):not(.required) {
  border-color: var(--primary-soft-border);
  box-shadow: var(--shadow-md);
  transform: translateY(-1px);
}

.module-card.selected {
  border-color: var(--primary);
  background: var(--primary-soft);
  box-shadow: 0 0 0 1px var(--primary) inset;
}

.module-card.required {
  cursor: not-allowed;
  background: var(--primary-soft);
  border-color: var(--primary-soft-border);
}

.module-card.disabled {
  opacity: 0.55;
  cursor: not-allowed;
  background: var(--surface-2);
}

.module-card .head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.module-card .title {
  font-weight: 600;
  font-size: 14px;
  color: var(--text);
}

.module-card .check {
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

.module-card.required .check {
  background: var(--text-subtle);
}

.module-card .desc {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.5;
}

.module-card .tags {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}

.module-card .deps {
  font-size: 11px;
  color: var(--text-subtle);
  margin-top: 2px;
}
</style>

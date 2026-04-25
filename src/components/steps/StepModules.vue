<script setup lang="ts">
import { useScaffold } from '../../composables/useScaffold';
import ModuleCard from '../ModuleCard.vue';

const { sortedModules, form, isSelected, toggleModule } = useScaffold();
</script>

<template>
  <section class="step-section">
    <header class="step-header">
      <h2>业务模块</h2>
      <p>
        点击卡片选择/取消。<strong>system / infra 必选</strong>。选中模块时会自动包含其依赖（如 crm 自动加 bpm）。
      </p>
    </header>

    <div class="module-grid">
      <ModuleCard
        v-for="m in sortedModules"
        :key="m.id"
        :module="m"
        :selected="isSelected(m.id)"
        :disabled="m.jdk17Only && form.jdkVersion === '8'"
        @toggle="toggleModule(m.id)"
      />
    </div>

    <div class="selected-bar">
      <span class="label">已选 ({{ form.modules.length }})</span>
      <div class="tags">
        <el-tag v-for="id in form.modules" :key="id" effect="plain">{{ id }}</el-tag>
      </div>
    </div>
  </section>
</template>

<style scoped>
.module-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}

.selected-bar {
  margin-top: 20px;
  padding: 14px 16px;
  background: var(--primary-soft);
  border: 1px solid var(--primary-soft-border);
  border-radius: var(--radius-md);
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.selected-bar .label {
  font-weight: 600;
  color: var(--primary-active);
  font-size: 13px;
}

.selected-bar .tags {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}
</style>

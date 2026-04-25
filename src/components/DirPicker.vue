<script setup lang="ts">
import { Folder } from '@element-plus/icons-vue';

defineProps<{
  modelValue: string;
  placeholder?: string;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', v: string): void;
  (e: 'pick'): void;
}>();
</script>

<template>
  <div class="dir-picker">
    <el-input
      :model-value="modelValue"
      :placeholder="placeholder"
      autocapitalize="off"
      autocorrect="off"
      spellcheck="false"
      @update:model-value="(v: string) => emit('update:modelValue', v)"
    />
    <el-button :icon="Folder" @click="emit('pick')">选择</el-button>
    <slot name="extra" />
  </div>
</template>

<style scoped>
.dir-picker {
  display: flex;
  gap: 8px;
  width: 100%;
}

.dir-picker :deep(.el-input) {
  flex: 1 1 auto;
  min-width: 0;
}

.dir-picker .el-button {
  flex: 0 0 auto;
}
</style>

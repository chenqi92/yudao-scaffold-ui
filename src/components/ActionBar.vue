<script setup lang="ts">
import { computed } from 'vue';
import { ElMessage } from 'element-plus';
import {
  ArrowLeft,
  ArrowRight,
  FolderOpened,
  RefreshLeft,
  VideoPlay
} from '@element-plus/icons-vue';
import { useScaffold } from '../composables/useScaffold';
import { formatError, revealInFinder } from '../api';

const {
  activeStep,
  steps,
  canNext,
  running,
  finished,
  prev,
  next,
  runScaffold,
  resetAll,
  form,
  phase
} = useScaffold();

const isLast = computed(() => activeStep.value === steps.length - 1);

const hint = computed(() => {
  if (running.value && phase.value) {
    return `[${phase.value.index}/${phase.value.total}] ${phase.value.title}`;
  }
  if (finished.value?.ok) {
    return `项目已生成 · ${finished.value.outputDir ?? ''}`;
  }
  if (finished.value?.message) {
    return finished.value.message;
  }
  switch (activeStep.value) {
    case 0: return 'Maven 坐标会用于全局替换';
    case 1: return 'JDK · 端口 · 超管账号';
    case 2: return `已选 ${form.modules.length} 个模块`;
    case 3:
      return `管理后台 · ${form.frontends.find((id) => id.startsWith('admin')) ?? '未选'}`;
    case 4: return '准备就绪';
    default: return '';
  }
});

async function openOutputDir() {
  if (!form.outputDir) return;
  try {
    await revealInFinder(form.outputDir);
  } catch (e) {
    ElMessage.error(`打开失败：${formatError(e)}`);
  }
}
</script>

<template>
  <div class="actionbar">
    <div class="hint" :class="{ 'is-error': finished?.message && !finished?.ok }">
      <span class="hint-text">{{ hint }}</span>
    </div>
    <div class="buttons">
      <!-- finished error: reset + retry -->
      <template v-if="finished?.message && !finished?.ok">
        <el-button :icon="RefreshLeft" :disabled="running" @click="resetAll">重置</el-button>
        <el-button
          type="primary"
          :icon="VideoPlay"
          :loading="running"
          @click="runScaffold"
        >重试</el-button>
      </template>

      <!-- finished ok: open dir + reset -->
      <template v-else-if="finished?.ok">
        <el-button :icon="FolderOpened" @click="openOutputDir">打开输出目录</el-button>
        <el-button type="primary" :icon="RefreshLeft" @click="resetAll">再生成一个</el-button>
      </template>

      <!-- last step idle: prev + start -->
      <template v-else-if="isLast">
        <el-button :icon="ArrowLeft" :disabled="running" @click="prev">上一步</el-button>
        <el-button
          type="primary"
          :icon="VideoPlay"
          :loading="running"
          @click="runScaffold"
        >{{ running ? '生成中…' : '开始生成' }}</el-button>
      </template>

      <!-- normal: prev + next -->
      <template v-else>
        <el-button
          :icon="ArrowLeft"
          :disabled="activeStep === 0 || running"
          @click="prev"
        >上一步</el-button>
        <el-button
          type="primary"
          :disabled="!canNext"
          @click="next"
        >
          下一步
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.actionbar {
  height: 64px;
  padding: 0 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.hint {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  color: var(--text-muted);
}

.hint.is-error {
  color: var(--danger);
}

.hint-text {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  text-overflow: ellipsis;
  word-break: break-all;
}

.buttons {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

@media (max-width: 900px) {
  .actionbar {
    padding: 0 16px;
  }
}
</style>

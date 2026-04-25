<script setup lang="ts">
import { computed } from 'vue';
import { Check, MagicStick, Refresh } from '@element-plus/icons-vue';
import { useScaffold } from '../composables/useScaffold';

const {
  steps,
  activeStep,
  running,
  finished,
  goTo,
  meta,
  settings,
  refreshMeta
} = useScaffold();

const workspacePath = computed(() => settings.workspaceOverride || meta.value?.workspace || '');
const mirrorLabel = computed(() => (settings.mirror === 'gitee' ? 'Gitee' : 'GitHub'));

function status(idx: number): 'done' | 'active' | 'future' {
  if (idx < activeStep.value) return 'done';
  if (idx === activeStep.value) return 'active';
  return 'future';
}

function clickStep(idx: number) {
  if (running.value) return;
  if (idx <= activeStep.value) goTo(idx);
}
</script>

<template>
  <div class="sidebar" :class="{ 'is-running': running }">
    <div class="brand">
      <span class="logo">
        <el-icon :size="20"><MagicStick /></el-icon>
      </span>
      <div class="brand-text">
        <strong>yudao-scaffold</strong>
        <span>从模板裁剪生成精简项目</span>
      </div>
    </div>

    <nav class="steps">
      <button
        v-for="(s, i) in steps"
        :key="s.title"
        type="button"
        class="step"
        :class="`is-${status(i)}`"
        :disabled="running || i > activeStep"
        @click="clickStep(i)"
      >
        <span class="circle">
          <el-icon v-if="status(i) === 'done'"><Check /></el-icon>
          <span v-else>{{ i + 1 }}</span>
        </span>
        <span class="meta">
          <span class="title">{{ s.title }}</span>
          <span class="subtitle">{{ s.subtitle }}</span>
        </span>
      </button>
    </nav>

    <div class="footer">
      <div v-if="finished?.ok" class="finished-pill">
        <span class="dot" />
        已完成
      </div>
      <div class="info-row">
        <span class="info-label">镜像</span>
        <span class="mirror-chip">{{ mirrorLabel }}</span>
      </div>
      <div class="info-row workspace">
        <span class="info-label">workspace</span>
        <span class="workspace-path" :title="workspacePath">{{ workspacePath || '未设置' }}</span>
      </div>
      <button
        type="button"
        class="refresh-btn"
        :disabled="running"
        @click="refreshMeta"
      >
        <el-icon><Refresh /></el-icon>
        刷新模板状态
      </button>
    </div>
  </div>
</template>

<style scoped>
.sidebar {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 20px 16px 16px;
  gap: 16px;
}

.sidebar.is-running .steps {
  pointer-events: none;
  opacity: 0.6;
}

.brand {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 8px;
}

.brand .logo {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md);
  background: var(--primary-soft);
  color: var(--primary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.brand-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.brand-text strong {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
}

.brand-text span {
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.steps {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}

.step {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  background: transparent;
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  text-align: left;
  font: inherit;
  color: var(--text-muted);
  transition: background 0.15s ease, color 0.15s ease;
  width: 100%;
}

.step:disabled {
  cursor: default;
}

.step:hover:not(:disabled) {
  background: var(--surface-2);
  color: var(--text);
}

.step.is-active {
  background: var(--primary-soft);
  color: var(--primary-active);
}

.step.is-active::before {
  content: '';
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 3px;
  border-radius: 2px;
  background: var(--primary);
}

.step.is-done {
  color: var(--text);
}

.circle {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 1.5px solid var(--border-strong);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-subtle);
  background: var(--surface);
  flex-shrink: 0;
  transition: all 0.15s ease;
}

.step.is-active .circle {
  background: var(--primary);
  color: var(--primary-fg);
  border-color: var(--primary);
}

.step.is-done .circle {
  background: var(--success);
  color: #fff;
  border-color: var(--success);
}

.meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.meta .title {
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
}

.meta .subtitle {
  font-size: 11px;
  color: var(--text-subtle);
  line-height: 1.3;
}

.step.is-active .meta .subtitle {
  color: var(--primary);
}

.footer {
  border-top: 1px solid var(--border);
  padding-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.finished-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  color: var(--success);
  background: rgba(16, 185, 129, 0.10);
  align-self: flex-start;
}

.finished-pill .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success);
}

.info-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 4px;
  font-size: 11px;
}

.info-label {
  color: var(--text-subtle);
  width: 64px;
  flex-shrink: 0;
}

.mirror-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--surface-2);
  color: var(--text-muted);
  font-weight: 500;
}

.workspace-path {
  flex: 1;
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}

.refresh-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  margin-top: 4px;
  padding: 6px 10px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
}

.refresh-btn:hover:not(:disabled) {
  background: var(--surface-2);
  border-color: var(--border-strong);
  color: var(--text);
}

.refresh-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* collapsed (icon-only) below 900px */
@media (max-width: 900px) {
  .sidebar {
    padding: 16px 8px;
    align-items: center;
  }
  .brand-text,
  .meta,
  .footer {
    display: none;
  }
  .step {
    justify-content: center;
    padding: 10px 0;
  }
  .step.is-active::before {
    left: 4px;
  }
}
</style>

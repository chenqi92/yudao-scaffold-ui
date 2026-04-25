<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { ArrowDown, CopyDocument } from '@element-plus/icons-vue';
import type { LogLine, PhaseInfo } from '../composables/useScaffold';

const props = defineProps<{
  lines: LogLine[];
  phase?: PhaseInfo | null;
}>();

const scroller = ref<HTMLElement | null>(null);
const stickToBottom = ref(true);

function onScroll() {
  const el = scroller.value;
  if (!el) return;
  const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
  stickToBottom.value = atBottom;
}

function jumpToBottom() {
  const el = scroller.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
  stickToBottom.value = true;
}

watch(
  () => props.lines.length,
  async () => {
    if (!stickToBottom.value) return;
    await nextTick();
    const el = scroller.value;
    if (el) el.scrollTop = el.scrollHeight;
  }
);

async function copyAll() {
  const text = props.lines.map((l) => l.text).join('\n');
  try {
    await navigator.clipboard.writeText(text);
    ElMessage.success('已复制全部日志');
  } catch {
    ElMessage.error('复制失败');
  }
}
</script>

<template>
  <div class="log-pane">
    <div class="log-toolbar">
      <span class="title">运行日志</span>
      <span v-if="phase" class="phase-chip">[{{ phase.index }}/{{ phase.total }}] {{ phase.title }}</span>
      <span class="spacer" />
      <button
        type="button"
        class="icon-btn"
        :disabled="!lines.length"
        title="复制全部"
        @click="copyAll"
      >
        <el-icon><CopyDocument /></el-icon>
      </button>
    </div>

    <div ref="scroller" class="log-body scrollbar-thin" @scroll="onScroll">
      <div
        v-for="(l, i) in lines"
        :key="i"
        class="log-line"
        :class="`type-${l.type}`"
      >
        <span class="dot" />
        <span class="text">{{ l.text }}</span>
      </div>
      <div v-if="!lines.length" class="empty">暂无日志</div>
    </div>

    <button
      v-if="!stickToBottom && lines.length"
      type="button"
      class="follow-pill"
      @click="jumpToBottom"
    >
      <el-icon><ArrowDown /></el-icon>
      跟随底部
    </button>
  </div>
</template>

<style scoped>
.log-pane {
  position: relative;
  background: #0f172a;
  border-radius: var(--radius-md);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  height: 320px;
  border: 1px solid #1e293b;
}

.log-toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  background: #111827;
  border-bottom: 1px solid #1e293b;
  color: #cbd5e1;
  font-size: 12px;
}

.log-toolbar .title {
  font-weight: 600;
  letter-spacing: 0.02em;
}

.log-toolbar .phase-chip {
  background: rgba(99, 102, 241, 0.18);
  color: #c7d2fe;
  border-radius: 999px;
  padding: 2px 10px;
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 11px;
}

.log-toolbar .spacer {
  flex: 1;
}

.icon-btn {
  background: transparent;
  border: 1px solid transparent;
  color: #94a3b8;
  border-radius: var(--radius-sm);
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.icon-btn:hover:not(:disabled) {
  background: rgba(148, 163, 184, 0.12);
  color: #e2e8f0;
}

.icon-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.log-body {
  flex: 1;
  overflow: auto;
  padding: 12px 16px;
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
  line-height: 1.65;
  color: #d4d4d4;
}

.log-body .empty {
  color: #64748b;
  font-style: italic;
}

.log-line {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  white-space: pre-wrap;
  word-break: break-all;
}

.log-line .dot {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  margin-top: 8px;
  background: #94a3b8;
}

.log-line .text {
  flex: 1 1 auto;
}

.log-line.type-info .dot { background: #94a3b8; }
.log-line.type-info .text { color: #cbd5e1; }

.log-line.type-ok .dot { background: #10b981; }
.log-line.type-ok .text { color: #6ee7b7; }

.log-line.type-warn .dot { background: #f59e0b; }
.log-line.type-warn .text { color: #fcd34d; }

.log-line.type-error .dot,
.log-line.type-failed .dot { background: #ef4444; }
.log-line.type-error .text,
.log-line.type-failed .text { color: #fca5a5; }

.log-line.type-phase .dot { background: #6366f1; }
.log-line.type-phase .text {
  color: #c7d2fe;
  font-weight: 600;
}

.log-line.type-done .dot { background: #22c55e; }
.log-line.type-done .text {
  color: #86efac;
  font-weight: 600;
}

.follow-pill {
  position: absolute;
  right: 16px;
  bottom: 16px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  background: var(--primary);
  color: var(--primary-fg);
  border: none;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  box-shadow: var(--shadow-lg);
  transition: background 0.15s ease;
}

.follow-pill:hover {
  background: var(--primary-hover);
}
</style>

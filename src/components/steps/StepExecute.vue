<script setup lang="ts">
import { computed } from 'vue';
import { useScaffold } from '../../composables/useScaffold';
import LogPane from '../LogPane.vue';
import TemplateBadge from '../TemplateBadge.vue';

const {
  form,
  settings,
  phase,
  logs,
  running,
  finished,
  templateStatus,
  backendTemplateName
} = useScaffold();

const progressPercent = computed(() => {
  if (!phase.value) return 0;
  return Math.round((phase.value.index / phase.value.total) * 100);
});

const progressStatus = computed(() => {
  if (finished.value?.ok) return 'success';
  if (finished.value?.message) return 'exception';
  return '';
});

const backendTpl = computed(() => templateStatus(backendTemplateName()));
</script>

<template>
  <section class="step-section">
    <header class="step-header">
      <h2>复核与执行</h2>
      <p>确认配置无误后开始生成。日志会实时流入下面的面板。</p>
    </header>

    <dl class="summary">
      <dt>项目名</dt>
      <dd>{{ form.projectName }} <span class="muted">({{ form.displayName }})</span></dd>

      <dt>输出目录</dt>
      <dd class="mono">{{ form.outputDir }}</dd>

      <dt>后端</dt>
      <dd>
        {{ form.backend === 'monolith' ? '单体' : '微服务' }} · JDK {{ form.jdkVersion }}
        <span class="muted">· 模板 {{ backendTemplateName() }}</span>
        <TemplateBadge :template="backendTpl" />
      </dd>

      <dt>Maven</dt>
      <dd class="mono">{{ form.groupId }}:{{ form.artifactId }}:{{ form.version }}</dd>

      <dt>Java 包</dt>
      <dd class="mono">{{ form.basePackage }}</dd>

      <dt>业务模块</dt>
      <dd class="tags">
        <el-tag v-for="id in form.modules" :key="id" effect="plain">{{ id }}</el-tag>
      </dd>

      <dt>前端</dt>
      <dd class="tags">
        <template v-if="form.frontends.length">
          <el-tag v-for="id in form.frontends" :key="id" effect="plain">{{ id }}</el-tag>
          <el-tag
            v-if="form.frontends.includes('admin-vben')"
            type="info"
            effect="plain"
          >vben/{{ form.vbenVariant }}</el-tag>
        </template>
        <span v-else class="muted">(无)</span>
      </dd>

      <dt>端口</dt>
      <dd>
        <template v-if="form.backend === 'monolith'">{{ form.monolithPort }}</template>
        <template v-else>
          <span>gateway {{ form.gatewayPort }}</span>
          <span
            v-for="id in form.modules"
            :key="id"
            class="port-chip"
          >{{ id }}={{ (form.microservicePorts?.[id] ?? []).join('/') }}</span>
        </template>
      </dd>

      <dt>超管</dt>
      <dd>{{ form.superAdminUsername }} / {{ '*'.repeat(form.superAdminPassword.length) }}</dd>

      <dt>多租户</dt>
      <dd>
        <el-tag v-if="form.tenantEnabled" type="info" effect="plain">启用</el-tag>
        <el-tag v-else type="warning" effect="plain">禁用 (清理租户管理代码)</el-tag>
      </dd>

      <dt>SQL 裁剪</dt>
      <dd>{{ form.sqlFilter ? '是' : '否' }}</dd>

      <dt>git pull</dt>
      <dd>{{ form.pullExisting ? '是' : '否' }}</dd>

      <dt>镜像</dt>
      <dd>{{ settings.mirror }}</dd>
    </dl>

    <div v-if="phase || running" class="progress">
      <el-progress
        :percentage="progressPercent"
        :status="progressStatus as 'success' | 'exception' | ''"
        :stroke-width="8"
      />
      <div v-if="phase" class="progress-meta">
        [{{ phase.index }}/{{ phase.total }}] {{ phase.title }}
      </div>
    </div>

    <LogPane v-if="logs.length || running" :lines="logs" :phase="phase" class="log" />

    <el-alert
      v-if="finished?.ok"
      type="success"
      :closable="false"
      class="result"
    >
      <template #title>项目已生成于 {{ finished.outputDir }}</template>
      目录结构：<code>backend/</code> + <code>frontend/{admin,mall,dashboard}/</code><br />
      下一步：<code>cd {{ finished.outputDir }}/backend && mvn -pl yudao-server -am package -DskipTests</code>
    </el-alert>

    <el-alert
      v-else-if="finished?.message"
      type="error"
      :closable="false"
      class="result"
      :title="finished.message"
    />
  </section>
</template>

<style scoped>
.summary {
  display: grid;
  grid-template-columns: 110px 1fr;
  gap: 8px 20px;
  margin: 0;
  padding: 16px;
  background: var(--surface-2);
  border-radius: var(--radius-md);
  font-size: 13px;
}

.summary dt {
  color: var(--text-muted);
  font-weight: 500;
  align-self: start;
}

.summary dd {
  margin: 0;
  color: var(--text);
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.summary .muted {
  color: var(--text-muted);
  font-weight: 400;
}

.summary .mono {
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
  color: var(--text);
  word-break: break-all;
}

.summary .tags {
  flex-wrap: wrap;
}

.port-chip {
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
  color: var(--text-muted);
  padding: 1px 6px;
  background: var(--surface);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}

.progress {
  margin-top: 20px;
}

.progress-meta {
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-muted);
  font-family: 'SF Mono', Monaco, Consolas, monospace;
}

.log {
  margin-top: 16px;
}

.result {
  margin-top: 16px;
  border-radius: var(--radius-md);
}

.result code {
  background: rgba(15, 23, 42, 0.05);
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
}
</style>

<script setup lang="ts">
import { useScaffold } from '../../composables/useScaffold';
import AdminCard from '../AdminCard.vue';
import DirPicker from '../DirPicker.vue';
import TemplateBadge from '../TemplateBadge.vue';

const {
  adminFrontends,
  otherFrontends,
  adminPick,
  otherFrontendPicks,
  templateStatus,
  settings,
  form,
  pickWorkspace,
  refreshMeta,
  meta,
  addOverride,
  removeOverride
} = useScaffold();
</script>

<template>
  <section class="step-section">
    <header class="step-header">
      <h2>前端 + 模板源</h2>
      <p>选择管理后台 (单选) 与其它前端 (多选)，并配置模板镜像与 URL 覆盖。</p>
    </header>

    <div class="group-title">管理后台 · 单选</div>
    <div class="card-grid">
      <AdminCard
        title="不需要管理后台"
        description="仅生成后端 + 其它前端"
        :selected="adminPick === ''"
        @select="adminPick = ''"
      />
      <AdminCard
        v-for="fe in adminFrontends"
        :key="fe.id"
        :title="fe.title"
        :description="fe.description"
        :selected="adminPick === fe.id"
        :template="templateStatus(fe.local)"
        @select="adminPick = fe.id"
      />
    </div>

    <div v-if="adminPick === 'admin-vben'" class="vben-variant">
      <div class="vben-variant-header">
        <span class="label">vben UI 变体</span>
        <span class="hint">vben monorepo 含 5 套 UI，仅保留所选变体（其余删除以减小体积）</span>
      </div>
      <el-radio-group v-model="form.vbenVariant" size="default">
        <el-radio-button value="antd">Ant Design Vue</el-radio-button>
        <el-radio-button value="ele">Element Plus</el-radio-button>
        <el-radio-button value="naive">Naive UI</el-radio-button>
        <el-radio-button value="tdesign">TDesign</el-radio-button>
        <el-radio-button value="antdv-next">Ant Design Next</el-radio-button>
      </el-radio-group>
    </div>

    <div class="group-title">其它前端 · 多选</div>
    <div class="other-list">
      <label
        v-for="fe in otherFrontends"
        :key="fe.id"
        class="other-row"
        :class="{ checked: otherFrontendPicks.includes(fe.id) }"
      >
        <el-checkbox
          :model-value="otherFrontendPicks.includes(fe.id)"
          @update:model-value="(v: string | number | boolean) => {
            const next = otherFrontendPicks.filter((x) => x !== fe.id);
            otherFrontendPicks = v ? [...next, fe.id] : next;
          }"
        />
        <div class="info">
          <strong>{{ fe.title }}</strong>
          <span class="desc">{{ fe.description }}</span>
        </div>
        <TemplateBadge :template="templateStatus(fe.local)" show-path />
      </label>
    </div>

    <div class="group-title">SQL 裁剪</div>
    <el-switch v-model="form.sqlFilter" active-text="按所选模块裁剪 SQL（保留原文为 *.full.sql 备份）" />

    <div class="group-title">模板源</div>
    <el-form label-width="120px" label-position="left">
      <el-form-item label="workspace">
        <DirPicker
          v-model="settings.workspaceOverride"
          :placeholder="meta?.workspace ?? ''"
          @pick="pickWorkspace"
        >
          <template #extra>
            <el-button @click="refreshMeta">刷新</el-button>
          </template>
        </DirPicker>
      </el-form-item>

      <el-form-item label="Clone 镜像">
        <el-radio-group v-model="settings.mirror">
          <el-radio-button value="gitee">Gitee</el-radio-button>
          <el-radio-button value="github">GitHub</el-radio-button>
        </el-radio-group>
        <span class="field-help" style="margin-left: 12px">仅在本地与缓存均无源时使用</span>
      </el-form-item>

      <el-form-item label="本地 git pull">
        <el-switch
          v-model="form.pullExisting"
          active-text="复制前对本地 git 仓库执行 pull --ff-only"
        />
      </el-form-item>

      <el-form-item label="URL 覆盖">
        <div class="overrides">
          <div
            v-for="(o, idx) in settings.urlOverrides"
            :key="idx"
            class="override-row"
          >
            <el-input v-model="o.name" placeholder="模板名 (如 ruoyi-vue-pro)" />
            <el-input v-model="o.url" placeholder="git URL" class="url" />
            <el-button type="danger" plain @click="removeOverride(idx)">删除</el-button>
          </div>
          <el-button @click="addOverride">+ 增加覆盖</el-button>
        </div>
      </el-form-item>
    </el-form>
  </section>
</template>

<style scoped>
.card-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}

.vben-variant {
  margin-top: 14px;
  padding: 14px 16px;
  background: var(--primary-soft);
  border: 1px solid var(--primary-soft-border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.vben-variant-header {
  display: flex;
  align-items: baseline;
  gap: 12px;
  flex-wrap: wrap;
}

.vben-variant-header .label {
  font-weight: 600;
  font-size: 13px;
  color: var(--primary-active);
}

.vben-variant-header .hint {
  font-size: 12px;
  color: var(--text-muted);
}

.other-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.other-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.other-row:hover {
  border-color: var(--primary-soft-border);
}

.other-row.checked {
  border-color: var(--primary);
  background: var(--primary-soft);
}

.other-row .info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.other-row .info strong {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.other-row .info .desc {
  font-size: 12px;
  color: var(--text-muted);
}

.overrides {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.override-row {
  display: flex;
  gap: 8px;
}

.override-row :deep(.el-input) {
  flex: 1;
}

.override-row :deep(.url.el-input) {
  flex: 2;
}
</style>

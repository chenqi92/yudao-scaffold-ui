<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Folder, Refresh, MagicStick } from '@element-plus/icons-vue';
import { formatError, loadMeta, pickDirectory, startScaffold } from './api';
import type {
  BackendKind,
  FrontendId,
  FrontendMeta,
  JdkVersion,
  Mirror,
  ModuleId,
  RunPayload,
  ScaffoldAnswers,
  ScaffoldEvent,
  ScaffoldMeta
} from './types';

const meta = ref<ScaffoldMeta | null>(null);
const loading = ref(true);
const activeStep = ref(0);

const form = reactive<ScaffoldAnswers>({
  projectName: 'my-app',
  displayName: '我的项目',
  outputDir: '',
  backend: 'monolith' as BackendKind,
  jdkVersion: '8' as JdkVersion,
  groupId: 'com.example',
  artifactId: 'my-app',
  version: '1.0.0-SNAPSHOT',
  basePackage: 'com.example.myapp',
  modules: ['system', 'infra'] as ModuleId[],
  frontends: ['admin-vue3'] as FrontendId[],
  sqlFilter: true,
  monolithPort: 48080,
  gatewayPort: 48080,
  microservicePorts: {},
  superAdminUsername: 'admin',
  superAdminPassword: 'admin123',
  pullExisting: true
});

const settings = reactive({
  mirror: 'gitee' as Mirror,
  workspaceOverride: '',
  urlOverrides: [] as { name: string; url: string }[]
});

interface LogLine {
  type: ScaffoldEvent['type'];
  text: string;
}

const logs = ref<LogLine[]>([]);
const phase = ref<{ index: number; total: number; title: string } | null>(null);
const running = ref(false);
const finished = ref<{ ok: boolean; outputDir?: string; message?: string } | null>(null);

const sortedModules = computed(() => meta.value?.modules ?? []);
const moduleIndex = computed(() => {
  const m = new Map<ModuleId, ScaffoldMeta['modules'][number]>();
  for (const x of sortedModules.value) m.set(x.id, x);
  return m;
});

const adminFrontends = computed<FrontendMeta[]>(() =>
  (meta.value?.frontends ?? []).filter((f) => f.role === 'admin')
);
const otherFrontends = computed<FrontendMeta[]>(() =>
  (meta.value?.frontends ?? []).filter((f) => f.role !== 'admin')
);

/** Currently picked admin frontend id, or '' for none. */
const adminPick = computed<FrontendId | ''>({
  get: () => (form.frontends.find((id) => isAdmin(id)) as FrontendId) ?? '',
  set: (val: FrontendId | '') => {
    const others = form.frontends.filter((id) => !isAdmin(id));
    form.frontends = val ? [val, ...others] : others;
  }
});

/** Selected non-admin frontends. */
const otherFrontendPicks = computed<FrontendId[]>({
  get: () => form.frontends.filter((id) => !isAdmin(id)),
  set: (val: FrontendId[]) => {
    const admin = form.frontends.find((id) => isAdmin(id));
    form.frontends = admin ? [admin, ...val] : [...val];
  }
});

function isAdmin(id: FrontendId): boolean {
  return adminFrontends.value.some((f) => f.id === id);
}

function expandDeps(seed: ModuleId[]): ModuleId[] {
  const out = new Set<ModuleId>();
  const visit = (id: ModuleId) => {
    if (out.has(id)) return;
    out.add(id);
    const m = moduleIndex.value.get(id);
    if (m) for (const d of m.deps) visit(d);
  };
  for (const x of meta.value?.modules.filter((m) => m.required).map((m) => m.id) ?? []) visit(x);
  for (const x of seed) visit(x);
  return Array.from(out);
}

function isSelected(id: ModuleId): boolean {
  return form.modules.includes(id);
}

function toggleModule(id: ModuleId): void {
  const m = moduleIndex.value.get(id);
  if (!m || m.required) return;
  if (m.jdk17Only && form.jdkVersion === '8') {
    ElMessage.warning(`${id} 模块需要 JDK 17`);
    return;
  }
  if (isSelected(id)) {
    const next = form.modules.filter((x) => x !== id);
    form.modules = expandDeps(next.filter((x) => !dependsOn(x, id)));
  } else {
    form.modules = expandDeps([...form.modules, id]);
  }
}

function dependsOn(child: ModuleId, parent: ModuleId): boolean {
  const m = moduleIndex.value.get(child);
  if (!m) return false;
  if (m.deps.includes(parent)) return true;
  return m.deps.some((d) => dependsOn(d, parent));
}

function defaultBasePackage(): string {
  const cleaned = form.artifactId.replace(/[-_]/g, '').toLowerCase();
  return `${form.groupId.toLowerCase()}.${cleaned}`;
}

function syncBasePackage(): void {
  form.basePackage = defaultBasePackage();
}

function syncArtifactFromProjectName(): void {
  if (!form.artifactId || form.artifactId === '') {
    form.artifactId = form.projectName;
  }
  syncBasePackage();
}

async function pickOutputDir(): Promise<void> {
  const initial = form.outputDir || meta.value?.workspace || meta.value?.homeDir;
  const dir = await pickDirectory(initial);
  if (dir) form.outputDir = dir;
}

async function pickWorkspace(): Promise<void> {
  const dir = await pickDirectory(settings.workspaceOverride || meta.value?.workspace);
  if (dir) {
    settings.workspaceOverride = dir;
    await refreshMeta();
  }
}

function templateStatus(name: string) {
  return meta.value?.templates.find((t) => t.name === name);
}

function backendTemplateName(): string {
  return form.backend === 'monolith' ? 'ruoyi-vue-pro' : 'yudao-cloud';
}

async function refreshMeta(): Promise<void> {
  loading.value = true;
  try {
    meta.value = await loadMeta(settings.workspaceOverride || undefined);
    settings.mirror = meta.value.defaultMirror;
    if (!form.outputDir && meta.value.workspace) {
      form.outputDir = `${meta.value.workspace}/${form.projectName}`;
    }
    form.monolithPort = meta.value.defaultMonolithPort;
    form.gatewayPort = meta.value.defaultGatewayPort;
  } catch (e) {
    ElMessage.error(`加载元数据失败：${formatError(e)}`);
  } finally {
    loading.value = false;
  }
}

onMounted(refreshMeta);

// Re-seed microservicePorts whenever modules / backend changes
watch(
  () => [form.backend, form.modules, meta.value?.modules] as const,
  () => {
    if (form.backend !== 'microservice' || !meta.value) {
      form.microservicePorts = {};
      return;
    }
    const ports: Partial<Record<ModuleId, number[]>> = { ...form.microservicePorts };
    for (const id of form.modules) {
      if (!ports[id]) {
        const m = meta.value.modules.find((x) => x.id === id);
        if (m) ports[id] = [...m.defaultMicroservicePorts];
      }
    }
    for (const id of Object.keys(ports) as ModuleId[]) {
      if (!form.modules.includes(id)) delete ports[id];
    }
    form.microservicePorts = ports;
  },
  { immediate: true }
);

// When JDK switches to 8, drop any previously-selected JDK17-only modules.
watch(
  () => form.jdkVersion,
  (val) => {
    if (val !== '8' || !meta.value) return;
    const before = form.modules.length;
    form.modules = form.modules.filter((id) => {
      const m = meta.value?.modules.find((x) => x.id === id);
      return !m?.jdk17Only;
    });
    if (form.modules.length < before) {
      ElMessage.warning('JDK 8 不支持的模块已自动取消（如 AI）');
    }
  }
);

const steps = [
  { title: '基本信息' },
  { title: '后端配置' },
  { title: '业务模块' },
  { title: '前端 + 镜像' },
  { title: '执行' }
];

const projectNameValid = computed(() => /^[a-z][a-z0-9-]*$/.test(form.projectName));
const artifactIdValid = computed(() => /^[a-z][a-z0-9-]*$/.test(form.artifactId));
const basePackageValid = computed(() => /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/.test(form.basePackage));

const canNext = computed(() => {
  if (activeStep.value === 0) {
    return (
      projectNameValid.value &&
      form.displayName.trim().length > 0 &&
      form.outputDir.trim().length > 0 &&
      form.groupId.trim().length > 0 &&
      artifactIdValid.value &&
      basePackageValid.value
    );
  }
  if (activeStep.value === 1) {
    return form.superAdminUsername.length > 0 && form.superAdminPassword.length > 0;
  }
  return true;
});

function next(): void {
  if (!canNext.value) {
    ElMessage.warning('请补全当前步骤的信息');
    return;
  }
  activeStep.value = Math.min(activeStep.value + 1, steps.length - 1);
}

function prev(): void {
  activeStep.value = Math.max(activeStep.value - 1, 0);
}

function appendLog(e: ScaffoldEvent): void {
  if (e.type === 'phase') {
    phase.value = { index: e.index, total: e.total, title: e.title };
    logs.value.push({ type: 'phase', text: `[${e.index}/${e.total}] ${e.title}` });
  } else if ('message' in e) {
    logs.value.push({ type: e.type, text: e.message });
  } else if (e.type === 'done') {
    logs.value.push({ type: 'done', text: `项目已生成: ${e.outputDir}` });
  }
}

async function runScaffold(): Promise<void> {
  if (running.value) return;
  logs.value = [];
  phase.value = null;
  finished.value = null;
  running.value = true;
  const payload: RunPayload = {
    answers: { ...form },
    workspace: settings.workspaceOverride || undefined,
    mirror: settings.mirror,
    urlOverrides: settings.urlOverrides.reduce<Record<string, string>>((acc, o) => {
      if (o.name && o.url) acc[o.name] = o.url;
      return acc;
    }, {})
  };
  try {
    const { code } = await startScaffold(payload, appendLog);
    if (code === 0) {
      finished.value = { ok: true, outputDir: form.outputDir };
      ElMessage.success('生成完成');
    } else {
      finished.value = { ok: false, message: `引擎退出码 ${code}` };
      ElMessage.error(`引擎退出码 ${code}`);
    }
  } catch (e) {
    const msg = formatError(e);
    finished.value = { ok: false, message: msg };
    ElMessage.error(msg);
  } finally {
    running.value = false;
  }
}

async function resetAll(): Promise<void> {
  await ElMessageBox.confirm('清空所有输入并回到第一步?', '确认重置', { type: 'warning' });
  activeStep.value = 0;
  finished.value = null;
  logs.value = [];
  phase.value = null;
}

function addOverride(): void {
  settings.urlOverrides.push({ name: '', url: '' });
}

function removeOverride(idx: number): void {
  settings.urlOverrides.splice(idx, 1);
}

function setMicroservicePort(id: ModuleId, idx: number, val: number | null): void {
  if (val == null) return;
  const arr = (form.microservicePorts?.[id] ?? []).slice();
  arr[idx] = val;
  form.microservicePorts = { ...form.microservicePorts, [id]: arr };
}

function moduleSubName(id: ModuleId, idx: number): string {
  const m = moduleIndex.value.get(id);
  if (m?.microserviceSubnames && m.microserviceSubnames[idx]) return m.microserviceSubnames[idx]!;
  return id;
}
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <el-icon :size="24"><MagicStick /></el-icon>
      <div>
        <h1>yudao-scaffold</h1>
        <div class="subtitle">从 yudao 模板裁剪生成精简新项目 · 单体 / 微服务 · 模块/前端按需裁剪</div>
      </div>
    </header>

    <main class="app-body" v-loading="loading">
      <el-card class="step-card">
        <el-steps :active="activeStep" finish-status="success" align-center>
          <el-step v-for="s in steps" :key="s.title" :title="s.title" />
        </el-steps>
      </el-card>

      <!-- Step 0: 基本信息 -->
      <el-card v-show="activeStep === 0" class="step-card">
        <h3>基本信息 + Maven 坐标</h3>
        <el-form label-width="120px" label-position="left">
          <el-form-item label="项目名" :error="projectNameValid ? '' : '只能小写字母 / 数字 / 连字符，且以字母开头'">
            <el-input
              v-model="form.projectName"
              placeholder="kebab-case，如 my-app"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              @blur="syncArtifactFromProjectName()"
            />
          </el-form-item>
          <el-form-item label="中文显示名">
            <el-input v-model="form.displayName" />
          </el-form-item>
          <el-form-item label="输出目录">
            <div class="dir-row">
              <el-input v-model="form.outputDir" placeholder="生成项目所在目录" />
              <el-button :icon="Folder" @click="pickOutputDir">选择</el-button>
            </div>
          </el-form-item>
          <el-divider content-position="left">Maven</el-divider>
          <el-form-item label="groupId">
            <el-input
              v-model="form.groupId"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              @blur="syncBasePackage"
            />
          </el-form-item>
          <el-form-item label="artifactId" :error="artifactIdValid ? '' : '只能小写字母 / 数字 / 连字符'">
            <el-input
              v-model="form.artifactId"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              @blur="syncBasePackage"
            />
          </el-form-item>
          <el-form-item label="version">
            <el-input v-model="form.version" autocapitalize="off" autocorrect="off" spellcheck="false" />
          </el-form-item>
          <el-form-item label="Java 包" :error="basePackageValid ? '' : '需为合法 Java 包名 (如 com.demo.app)'">
            <el-input
              v-model="form.basePackage"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
            />
            <div style="font-size: 12px; color: #909399; margin-top: 4px">
              cn.iocoder.yudao 将被全局替换为这个包
            </div>
          </el-form-item>
        </el-form>
      </el-card>

      <!-- Step 1: 后端配置 -->
      <el-card v-show="activeStep === 1" class="step-card">
        <h3>后端架构 + JDK + 端口 + 超管</h3>
        <el-form label-width="140px" label-position="left">
          <el-form-item label="后端架构">
            <el-radio-group v-model="form.backend">
              <el-radio-button value="monolith">单体 (ruoyi-vue-pro)</el-radio-button>
              <el-radio-button value="microservice">微服务 (yudao-cloud)</el-radio-button>
            </el-radio-group>
          </el-form-item>
          <el-form-item label="JDK 版本">
            <el-radio-group v-model="form.jdkVersion">
              <el-radio-button value="8">JDK 8</el-radio-button>
              <el-radio-button value="17">JDK 17</el-radio-button>
            </el-radio-group>
            <span style="margin-left: 12px; color: #909399; font-size: 12px">
              AI 模块仅 JDK 17 支持
            </span>
          </el-form-item>

          <el-divider content-position="left">服务端口</el-divider>

          <template v-if="form.backend === 'monolith'">
            <el-form-item label="后端服务端口">
              <el-input-number
                v-model="form.monolithPort"
                :min="1024"
                :max="65535"
                controls-position="right"
              />
            </el-form-item>
          </template>

          <template v-else>
            <el-form-item label="Gateway 端口">
              <el-input-number
                v-model="form.gatewayPort"
                :min="1024"
                :max="65535"
                controls-position="right"
              />
            </el-form-item>
            <el-form-item label="各微服务端口" v-if="form.modules.length">
              <div class="port-grid" style="width: 100%">
                <template v-for="id in form.modules" :key="id">
                  <template v-for="(p, idx) in form.microservicePorts?.[id] ?? []" :key="`${id}-${idx}`">
                    <span>
                      <el-tag size="small">{{ id }}</el-tag>
                      <span v-if="(form.microservicePorts?.[id]?.length ?? 0) > 1" style="margin-left: 4px; color: #909399">
                        / {{ moduleSubName(id, idx) }}
                      </span>
                    </span>
                    <el-input-number
                      :model-value="p"
                      :min="1024"
                      :max="65535"
                      controls-position="right"
                      @update:model-value="(v: number | null) => setMicroservicePort(id, idx, v)"
                    />
                  </template>
                </template>
              </div>
            </el-form-item>
          </template>

          <el-divider content-position="left">超级管理员</el-divider>
          <el-form-item label="超管账号">
            <el-input
              v-model="form.superAdminUsername"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              placeholder="admin"
            />
          </el-form-item>
          <el-form-item label="超管密码">
            <el-input
              v-model="form.superAdminPassword"
              show-password
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              placeholder="admin123"
            />
            <div style="font-size: 12px; color: #909399; margin-top: 4px">
              密码会以 BCrypt 哈希写入 system_users 表 (id=1) 的 INSERT 语句
            </div>
          </el-form-item>
        </el-form>
      </el-card>

      <!-- Step 2: 业务模块 -->
      <el-card v-show="activeStep === 2" class="step-card">
        <h3>业务模块</h3>
        <p style="color: #606266; font-size: 13px">
          点击卡片选择/取消。
          <strong>system / infra 必选</strong>。选中模块时会自动包含其依赖（如 crm 自动加 bpm）。
        </p>
        <div class="module-grid">
          <div
            v-for="m in sortedModules"
            :key="m.id"
            class="module-card"
            :class="{
              selected: isSelected(m.id),
              required: m.required,
              disabled: m.jdk17Only && form.jdkVersion === '8'
            }"
            @click="toggleModule(m.id)"
          >
            <div class="name">
              {{ m.title }}
              <el-tag v-if="m.required" type="info" size="small">必选</el-tag>
              <el-tag v-if="m.composite" type="warning" size="small">composite</el-tag>
              <el-tag
                v-if="m.jdk17Only"
                :type="form.jdkVersion === '17' ? 'danger' : 'info'"
                size="small"
              >JDK17</el-tag>
            </div>
            <div class="desc">{{ m.description }}</div>
            <div v-if="m.deps.length" class="deps">依赖: {{ m.deps.join(', ') }}</div>
          </div>
        </div>
        <div style="margin-top: 16px; padding: 12px; background: #f0f9ff; border-radius: 6px">
          <strong>已选 ({{ form.modules.length }})</strong>:
          <el-tag
            v-for="id in form.modules"
            :key="id"
            style="margin-left: 6px; margin-bottom: 4px"
          >{{ id }}</el-tag>
        </div>
      </el-card>

      <!-- Step 3: 前端 + 镜像 -->
      <el-card v-show="activeStep === 3" class="step-card">
        <h3>管理后台 (单选，可不选)</h3>
        <div class="admin-radio-grid">
          <div
            class="admin-radio-card"
            :class="{ selected: adminPick === '' }"
            @click="adminPick = ''"
          >
            <div class="name">不需要管理后台</div>
            <div class="desc">仅生成后端 + 其它前端</div>
          </div>
          <div
            v-for="fe in adminFrontends"
            :key="fe.id"
            class="admin-radio-card"
            :class="{ selected: adminPick === fe.id }"
            @click="adminPick = fe.id"
          >
            <div class="name">
              {{ fe.title }}
              <el-tag
                v-if="templateStatus(fe.local)?.localPresent"
                type="success"
                size="small"
              >本地</el-tag>
              <el-tag
                v-else-if="templateStatus(fe.local)?.cachePresent"
                type="warning"
                size="small"
              >缓存</el-tag>
              <el-tag v-else type="danger" size="small">需 clone</el-tag>
            </div>
            <div class="desc">{{ fe.description }}</div>
          </div>
        </div>

        <h3 style="margin-top: 24px">其它前端 (可多选)</h3>
        <el-checkbox-group v-model="otherFrontendPicks">
          <div v-for="fe in otherFrontends" :key="fe.id" style="margin-bottom: 12px">
            <el-checkbox :value="fe.id">
              <strong>{{ fe.title }}</strong>
              <span style="margin-left: 8px; color: #909399; font-size: 12px">
                {{ fe.description }}
              </span>
            </el-checkbox>
            <div class="template-status">
              <template v-if="templateStatus(fe.local)?.localPresent">
                <el-tag type="success" size="small">本地</el-tag>
                <span style="color: #909399">{{ templateStatus(fe.local)?.localPath }}</span>
              </template>
              <template v-else-if="templateStatus(fe.local)?.cachePresent">
                <el-tag type="warning" size="small">缓存</el-tag>
              </template>
              <template v-else>
                <el-tag type="danger" size="small">需 clone</el-tag>
              </template>
            </div>
          </div>
        </el-checkbox-group>

        <el-divider content-position="left">SQL 裁剪</el-divider>
        <el-switch
          v-model="form.sqlFilter"
          active-text="按所选模块裁剪 SQL（保留原文为 *.full.sql 备份）"
        />

        <el-divider content-position="left">模板源</el-divider>
        <el-form label-width="120px" label-position="left">
          <el-form-item label="workspace">
            <div class="dir-row">
              <el-input
                v-model="settings.workspaceOverride"
                :placeholder="meta?.workspace ?? ''"
                autocapitalize="off"
                autocorrect="off"
                spellcheck="false"
              />
              <el-button :icon="Folder" @click="pickWorkspace">选择</el-button>
              <el-button :icon="Refresh" @click="refreshMeta">刷新</el-button>
            </div>
          </el-form-item>
          <el-form-item label="Clone 镜像">
            <el-radio-group v-model="settings.mirror">
              <el-radio-button value="gitee">Gitee</el-radio-button>
              <el-radio-button value="github">GitHub</el-radio-button>
            </el-radio-group>
            <span style="margin-left: 12px; color: #909399; font-size: 12px">
              仅在本地与缓存均无源时使用
            </span>
          </el-form-item>
          <el-form-item label="本地 git pull">
            <el-switch
              v-model="form.pullExisting"
              active-text="复制前对本地 git 仓库执行 pull --ff-only"
            />
          </el-form-item>
          <el-form-item label="URL 覆盖">
            <div style="width: 100%">
              <div
                v-for="(o, idx) in settings.urlOverrides"
                :key="idx"
                style="display: flex; gap: 8px; margin-bottom: 8px"
              >
                <el-input v-model="o.name" placeholder="模板名 (如 ruoyi-vue-pro)" style="flex: 1" />
                <el-input v-model="o.url" placeholder="git URL" style="flex: 2" />
                <el-button type="danger" plain @click="removeOverride(idx)">删除</el-button>
              </div>
              <el-button @click="addOverride">+ 增加覆盖</el-button>
            </div>
          </el-form-item>
        </el-form>
      </el-card>

      <!-- Step 4: 执行 -->
      <el-card v-show="activeStep === 4" class="step-card">
        <h3>复核与执行</h3>
        <div class="summary-grid">
          <span class="label">项目名</span><span>{{ form.projectName }} ({{ form.displayName }})</span>
          <span class="label">输出目录</span><span>{{ form.outputDir }}</span>
          <span class="label">后端</span>
          <span>
            {{ form.backend === 'monolith' ? '单体' : '微服务' }} · JDK {{ form.jdkVersion }}
            (模板 {{ backendTemplateName() }}
            <el-tag size="small" :type="
              templateStatus(backendTemplateName())?.localPresent ? 'success' :
              templateStatus(backendTemplateName())?.cachePresent ? 'warning' : 'danger'
            ">{{
              templateStatus(backendTemplateName())?.localPresent ? '本地' :
              templateStatus(backendTemplateName())?.cachePresent ? '缓存' : '需 clone'
            }}</el-tag>)
          </span>
          <span class="label">Maven</span>
          <span>{{ form.groupId }}:{{ form.artifactId }}:{{ form.version }}</span>
          <span class="label">Java 包</span><span>{{ form.basePackage }}</span>
          <span class="label">业务模块</span>
          <span>
            <el-tag v-for="id in form.modules" :key="id" style="margin-right: 4px">{{ id }}</el-tag>
          </span>
          <span class="label">前端</span>
          <span>
            <el-tag v-for="id in form.frontends" :key="id" style="margin-right: 4px">{{ id }}</el-tag>
            <span v-if="!form.frontends.length" style="color: #909399">(无)</span>
          </span>
          <span class="label">端口</span>
          <span v-if="form.backend === 'monolith'">{{ form.monolithPort }}</span>
          <span v-else>
            gateway {{ form.gatewayPort }}；
            <span v-for="id in form.modules" :key="id" style="margin-right: 8px">
              {{ id }}={{ (form.microservicePorts?.[id] ?? []).join('/') }}
            </span>
          </span>
          <span class="label">超管</span>
          <span>{{ form.superAdminUsername }} / {{ '*'.repeat(form.superAdminPassword.length) }}</span>
          <span class="label">SQL 裁剪</span><span>{{ form.sqlFilter ? '是' : '否' }}</span>
          <span class="label">git pull</span><span>{{ form.pullExisting ? '是' : '否' }}</span>
          <span class="label">镜像</span><span>{{ settings.mirror }}</span>
        </div>

        <div style="margin-top: 16px; display: flex; gap: 8px">
          <el-button
            type="primary"
            :loading="running"
            :disabled="!!finished?.ok"
            @click="runScaffold"
          >{{ finished?.ok ? '已完成' : '开始生成' }}</el-button>
          <el-button v-if="finished?.ok" @click="resetAll">再生成一个</el-button>
        </div>

        <div v-if="phase" style="margin-top: 16px">
          <el-progress
            :percentage="Math.round((phase.index / phase.total) * 100)"
            :status="finished?.ok ? 'success' : finished?.message ? 'exception' : ''"
          />
          <div style="font-size: 12px; color: #606266; margin-top: 4px">
            [{{ phase.index }}/{{ phase.total }}] {{ phase.title }}
          </div>
        </div>

        <div v-if="logs.length" class="log-pane" style="margin-top: 12px">
          <div v-for="(l, i) in logs" :key="i" :class="['log-line', l.type]">{{ l.text }}</div>
        </div>

        <el-alert
          v-if="finished?.ok"
          type="success"
          :closable="false"
          style="margin-top: 12px"
        >
          <template #title>项目已生成于 {{ finished.outputDir }}</template>
          目录结构：<code>backend/</code> + <code>frontend/{admin,mall,dashboard}/</code><br />
          下一步：<code>cd {{ finished.outputDir }}/backend && mvn -pl yudao-server -am package -DskipTests</code>
        </el-alert>
        <el-alert
          v-else-if="finished?.message"
          type="error"
          :closable="false"
          style="margin-top: 12px"
          :title="finished.message"
        />
      </el-card>

      <!-- Footer nav -->
      <el-card class="step-card" v-show="!finished?.ok">
        <div style="display: flex; justify-content: space-between">
          <el-button :disabled="activeStep === 0 || running" @click="prev">上一步</el-button>
          <el-button
            v-if="activeStep < steps.length - 1"
            type="primary"
            :disabled="!canNext"
            @click="next"
          >下一步</el-button>
        </div>
      </el-card>
    </main>
  </div>
</template>

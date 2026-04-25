import { computed, reactive, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { formatError, loadMeta, pathExists, pickDirectory, startScaffold } from '../api';
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
} from '../types';

export interface LogLine {
  type: ScaffoldEvent['type'];
  text: string;
}

export interface PhaseInfo {
  index: number;
  total: number;
  title: string;
}

export interface FinishedState {
  ok: boolean;
  outputDir?: string;
  message?: string;
}

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
  pullExisting: true,
  tenantEnabled: true,
  vbenVariant: 'antd'
});

const settings = reactive({
  mirror: 'gitee' as Mirror,
  workspaceOverride: '',
  urlOverrides: [] as { name: string; url: string }[]
});

const logs = ref<LogLine[]>([]);
const phase = ref<PhaseInfo | null>(null);
const running = ref(false);
const finished = ref<FinishedState | null>(null);

const steps = [
  { title: '基本信息', subtitle: 'Maven 坐标' },
  { title: '后端配置', subtitle: 'JDK · 端口 · 超管' },
  { title: '业务模块', subtitle: '按需裁剪' },
  { title: '前端 + 镜像', subtitle: 'UI · 模板源' },
  { title: '执行', subtitle: '复核 · 生成' }
] as const;

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

function isAdmin(id: FrontendId): boolean {
  return adminFrontends.value.some((f) => f.id === id);
}

const adminPick = computed<FrontendId | ''>({
  get: () => (form.frontends.find((id) => isAdmin(id)) as FrontendId) ?? '',
  set: (val: FrontendId | '') => {
    const others = form.frontends.filter((id) => !isAdmin(id));
    form.frontends = val ? [val, ...others] : others;
  }
});

const otherFrontendPicks = computed<FrontendId[]>({
  get: () => form.frontends.filter((id) => !isAdmin(id)),
  set: (val: FrontendId[]) => {
    const admin = form.frontends.find((id) => isAdmin(id));
    form.frontends = admin ? [admin, ...val] : [...val];
  }
});

function expandDeps(seed: ModuleId[]): ModuleId[] {
  const out = new Set<ModuleId>();
  const visit = (id: ModuleId) => {
    if (out.has(id)) return;
    out.add(id);
    const m = moduleIndex.value.get(id);
    if (m) for (const d of m.deps) visit(d);
  };
  for (const x of meta.value?.modules.filter((m) => m.required).map((m) => m.id) ?? [])
    visit(x);
  for (const x of seed) visit(x);
  return Array.from(out);
}

function isSelected(id: ModuleId): boolean {
  return form.modules.includes(id);
}

function dependsOn(child: ModuleId, parent: ModuleId): boolean {
  const m = moduleIndex.value.get(child);
  if (!m) return false;
  if (m.deps.includes(parent)) return true;
  return m.deps.some((d) => dependsOn(d, parent));
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

function moduleSubName(id: ModuleId, idx: number): string {
  const m = moduleIndex.value.get(id);
  if (m?.microserviceSubnames && m.microserviceSubnames[idx])
    return m.microserviceSubnames[idx]!;
  return id;
}

function setMicroservicePort(id: ModuleId, idx: number, val: number | null): void {
  if (val == null) return;
  const arr = (form.microservicePorts?.[id] ?? []).slice();
  arr[idx] = val;
  form.microservicePorts = { ...form.microservicePorts, [id]: arr };
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

const projectNameValid = computed(() => /^[a-z][a-z0-9-]*$/.test(form.projectName));
const artifactIdValid = computed(() => /^[a-z][a-z0-9-]*$/.test(form.artifactId));
const basePackageValid = computed(() =>
  /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/.test(form.basePackage)
);

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

function goTo(idx: number): void {
  if (running.value) return;
  if (idx < 0 || idx >= steps.length) return;
  if (idx <= activeStep.value) {
    activeStep.value = idx;
  }
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

  let force = false;
  if (form.outputDir && (await pathExists(form.outputDir))) {
    try {
      await ElMessageBox.confirm(
        `输出目录已存在：\n${form.outputDir}\n\n继续将删除该目录的全部内容并重新生成，此操作不可撤销。`,
        '确认强制覆盖',
        {
          type: 'warning',
          confirmButtonText: '强制覆盖',
          cancelButtonText: '取消',
          confirmButtonClass: 'el-button--danger',
          dangerouslyUseHTMLString: false
        }
      );
      force = true;
    } catch {
      return;
    }
  }

  logs.value = [];
  phase.value = null;
  finished.value = null;
  running.value = true;
  const payload: RunPayload = {
    answers: { ...form, force },
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

let initialized = false;
async function init(): Promise<void> {
  if (initialized) return;
  initialized = true;

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

  await refreshMeta();
}

export function useScaffold() {
  return {
    // state
    meta,
    loading,
    activeStep,
    form,
    settings,
    logs,
    phase,
    running,
    finished,
    steps,

    // computed
    sortedModules,
    moduleIndex,
    adminFrontends,
    otherFrontends,
    adminPick,
    otherFrontendPicks,
    projectNameValid,
    artifactIdValid,
    basePackageValid,
    canNext,

    // module helpers
    isAdmin,
    isSelected,
    expandDeps,
    toggleModule,
    dependsOn,
    moduleSubName,
    setMicroservicePort,

    // form helpers
    defaultBasePackage,
    syncBasePackage,
    syncArtifactFromProjectName,
    pickOutputDir,
    pickWorkspace,

    // template helpers
    templateStatus,
    backendTemplateName,

    // lifecycle / actions
    init,
    refreshMeta,
    next,
    prev,
    goTo,
    appendLog,
    runScaffold,
    resetAll,
    addOverride,
    removeOverride
  };
}

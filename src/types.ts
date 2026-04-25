export type ModuleId =
  | 'system' | 'infra' | 'member' | 'bpm' | 'pay' | 'mp' | 'mall'
  | 'crm' | 'erp' | 'iot' | 'mes' | 'report' | 'ai';

export type FrontendId =
  | 'admin-vue3' | 'admin-vben' | 'admin-vue2' | 'admin-uniapp'
  | 'mall-uniapp' | 'go-view';

export type FrontendRole = 'admin' | 'mall' | 'dashboard';

export type Mirror = 'gitee' | 'github';

export type BackendKind = 'monolith' | 'microservice';

export type JdkVersion = '8' | '17';

export interface ModuleMeta {
  id: ModuleId;
  title: string;
  description: string;
  deps: ModuleId[];
  composite: boolean;
  jdk17Only: boolean;
  required: boolean;
  defaultMicroservicePorts: number[];
  microserviceSubnames: string[] | null;
}

export interface FrontendMeta {
  id: FrontendId;
  title: string;
  description: string;
  local: string;
  role: FrontendRole;
  roleSuffix: string;
  modular: boolean;
}

export interface TemplatePresence {
  name: string;
  kind: 'backend' | 'frontend';
  localPath: string;
  localPresent: boolean;
  cachePath: string;
  cachePresent: boolean;
  gitee: string;
  github: string;
  isGitRepo: boolean;
}

export interface ScaffoldMeta {
  workspace: string;
  cacheDir: string;
  homeDir: string;
  defaultMirror: Mirror;
  defaultMonolithPort: number;
  defaultGatewayPort: number;
  modules: ModuleMeta[];
  frontends: FrontendMeta[];
  templates: TemplatePresence[];
}

export interface ScaffoldAnswers {
  projectName: string;
  displayName: string;
  outputDir: string;
  backend: BackendKind;
  jdkVersion: JdkVersion;
  groupId: string;
  artifactId: string;
  version: string;
  basePackage: string;
  modules: ModuleId[];
  frontends: FrontendId[];
  sqlFilter: boolean;
  monolithPort?: number;
  gatewayPort?: number;
  microservicePorts?: Partial<Record<ModuleId, number[]>>;
  superAdminUsername: string;
  superAdminPassword: string;
  pullExisting: boolean;
  /** When true, an existing output directory will be removed before generation. */
  force?: boolean;
  /** When false, set yudao.tenant.enable=false and strip tenant management UI/SQL. */
  tenantEnabled: boolean;
}

export interface RunPayload {
  answers: ScaffoldAnswers;
  workspace?: string;
  mirror?: Mirror;
  urlOverrides?: Record<string, string>;
}

export type ScaffoldEvent =
  | { type: 'phase'; index: number; total: number; title: string }
  | { type: 'info'; message: string }
  | { type: 'ok'; message: string }
  | { type: 'warn'; message: string }
  | { type: 'error'; message: string }
  | { type: 'done'; outputDir: string }
  | { type: 'failed'; message: string };

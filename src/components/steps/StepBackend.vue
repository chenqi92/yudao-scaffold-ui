<script setup lang="ts">
import { useScaffold } from '../../composables/useScaffold';

const { form, setMicroservicePort, moduleSubName } = useScaffold();
</script>

<template>
  <section class="step-section">
    <header class="step-header">
      <h2>后端配置</h2>
      <p>选择后端架构、JDK 版本，并设置端口与超级管理员账号。</p>
    </header>

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
        <span class="field-help" style="margin-left: 12px">AI 模块仅 JDK 17 支持</span>
      </el-form-item>

      <el-form-item label="多租户">
        <el-switch
          v-model="form.tenantEnabled"
          active-text="启用"
          inactive-text="禁用"
        />
        <div class="field-help">
          关闭后会设 <code>yudao.tenant.enable=false</code>，并删除租户管理表/菜单/前端页面。
          新业务表无需 <code>tenant_id</code> 列、无需 <code>@TenantIgnore</code> 注解。
        </div>
      </el-form-item>

      <div class="group-title">服务端口</div>

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

        <el-form-item v-if="form.modules.length" label="各微服务端口">
          <div class="port-grid">
            <template v-for="id in form.modules" :key="id">
              <template
                v-for="(p, idx) in form.microservicePorts?.[id] ?? []"
                :key="`${id}-${idx}`"
              >
                <span class="port-label">
                  <el-tag size="small" effect="plain">{{ id }}</el-tag>
                  <span
                    v-if="(form.microservicePorts?.[id]?.length ?? 0) > 1"
                    class="port-sub"
                  >/ {{ moduleSubName(id, idx) }}</span>
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

      <div class="group-title">超级管理员</div>

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
        <div class="field-help">密码会以 BCrypt 哈希写入 system_users 表 (id=1) 的 INSERT 语句</div>
      </el-form-item>
    </el-form>
  </section>
</template>

<style scoped>
.port-grid {
  display: grid;
  grid-template-columns: 220px 1fr;
  gap: 8px 16px;
  align-items: center;
  font-size: 13px;
  width: 100%;
}

.port-label {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.port-sub {
  color: var(--text-muted);
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
}

.port-grid :deep(.el-input-number) {
  max-width: 200px;
}

code {
  background: var(--surface-2);
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
}
</style>

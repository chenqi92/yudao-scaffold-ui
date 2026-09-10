<script setup lang="ts">
import { useScaffold } from '../../composables/useScaffold';
import DirPicker from '../DirPicker.vue';

const {
  form,
  projectNameValid,
  artifactIdValid,
  basePackageValid,
  gitRemotesValid,
  syncBasePackage,
  syncArtifactFromProjectName,
  pickOutputDir,
  addGitRemote,
  removeGitRemote
} = useScaffold();
</script>

<template>
  <section class="step-section">
    <header class="step-header">
      <h2>基本信息</h2>
      <p>项目名、Maven 坐标和输出目录。<code>cn.iocoder.yudao</code> 会被全局替换为你填写的 Java 包。</p>
    </header>

    <el-form label-width="120px" label-position="left">
      <el-form-item
        label="项目名"
        :error="projectNameValid ? '' : '只能小写字母 / 数字 / 连字符，且以字母开头'"
      >
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
        <DirPicker v-model="form.outputDir" placeholder="生成项目所在目录" @pick="pickOutputDir" />
      </el-form-item>

      <div class="group-title">Maven 坐标</div>

      <el-form-item label="groupId">
        <el-input
          v-model="form.groupId"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          @blur="syncBasePackage"
        />
      </el-form-item>

      <el-form-item
        label="artifactId"
        :error="artifactIdValid ? '' : '只能小写字母 / 数字 / 连字符'"
      >
        <el-input
          v-model="form.artifactId"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          @blur="syncBasePackage"
        />
      </el-form-item>

      <el-form-item label="version">
        <el-input
          v-model="form.version"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
        />
      </el-form-item>

      <el-form-item
        label="Java 包"
        :error="basePackageValid ? '' : '需为合法 Java 包名 (如 com.demo.app)'"
      >
        <el-input
          v-model="form.basePackage"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
        />
        <div class="field-help">cn.iocoder.yudao 将被全局替换为这个包</div>
      </el-form-item>

      <div class="group-title">Git 仓库（可选）</div>
      <p class="field-help git-help">
        可配置多个 remote。生成器会直接初始化标准 <code>.git</code> 目录，不要求系统安装 Git；提交和推送仍需 Git 客户端。
      </p>
      <div class="git-remotes">
        <div v-for="(remote, idx) in form.gitRemotes" :key="idx" class="git-remote-row">
          <el-input v-model="remote.name" placeholder="remote 名称，如 origin" />
          <el-input v-model="remote.url" placeholder="https://... 或 git@host:owner/repo.git" />
          <el-button type="danger" plain @click="removeGitRemote(idx)">删除</el-button>
        </div>
        <el-alert
          v-if="!gitRemotesValid"
          type="error"
          :closable="false"
          title="remote 名称必须合法且不能重复，每项都要填写仓库地址"
        />
        <el-button plain @click="addGitRemote">添加 Git 地址</el-button>
      </div>
    </el-form>
  </section>
</template>

<style scoped>
code {
  background: var(--surface-2);
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  font-size: 12px;
}

.git-help {
  margin: -4px 0 12px;
}

.git-remotes {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 10px;
}

.git-remote-row {
  display: grid;
  grid-template-columns: 180px minmax(320px, 1fr) auto;
  gap: 10px;
  width: 100%;
}
</style>

<script setup lang="ts">
import { useScaffold } from '../../composables/useScaffold';
import DirPicker from '../DirPicker.vue';

const {
  form,
  projectNameValid,
  artifactIdValid,
  basePackageValid,
  syncBasePackage,
  syncArtifactFromProjectName,
  pickOutputDir
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
</style>

<script setup lang="ts">
import { useScaffold } from '../composables/useScaffold';

const { loading } = useScaffold();
</script>

<template>
  <div class="app-shell" v-loading="loading" element-loading-svg-color="#6366f1">
    <aside class="app-sidebar">
      <slot name="sidebar" />
    </aside>
    <div class="app-main">
      <main class="app-content scrollbar-thin">
        <slot />
      </main>
      <footer class="app-actionbar">
        <slot name="actionBar" />
      </footer>
    </div>
  </div>
</template>

<style scoped>
.app-shell {
  height: 100vh;
  width: 100vw;
  display: grid;
  grid-template-columns: 240px 1fr;
  overflow: hidden;
  background: var(--bg);
}

.app-sidebar {
  background: var(--surface);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.app-main {
  display: grid;
  grid-template-rows: 1fr auto;
  overflow: hidden;
  min-width: 0;
}

.app-content {
  overflow-y: auto;
  padding: 32px 40px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  align-items: stretch;
}

.app-content > :deep(*) {
  max-width: 960px;
  width: 100%;
  margin: 0 auto;
}

.app-actionbar {
  background: var(--surface);
  border-top: 1px solid var(--border);
  box-shadow: 0 -4px 16px rgba(15, 23, 42, 0.04);
}

@media (max-width: 900px) {
  .app-shell {
    grid-template-columns: 64px 1fr;
  }
  .app-content {
    padding: 24px 20px;
  }
}
</style>

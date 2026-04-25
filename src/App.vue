<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useScaffold } from './composables/useScaffold';
import AppLayout from './components/AppLayout.vue';
import Sidebar from './components/Sidebar.vue';
import ActionBar from './components/ActionBar.vue';
import StepBasic from './components/steps/StepBasic.vue';
import StepBackend from './components/steps/StepBackend.vue';
import StepModules from './components/steps/StepModules.vue';
import StepFrontend from './components/steps/StepFrontend.vue';
import StepExecute from './components/steps/StepExecute.vue';

const { activeStep, init } = useScaffold();

const stepComponent = computed(() => {
  switch (activeStep.value) {
    case 0: return StepBasic;
    case 1: return StepBackend;
    case 2: return StepModules;
    case 3: return StepFrontend;
    case 4: return StepExecute;
    default: return StepBasic;
  }
});

onMounted(init);
</script>

<template>
  <AppLayout>
    <template #sidebar>
      <Sidebar />
    </template>
    <component :is="stepComponent" />
    <template #actionBar>
      <ActionBar />
    </template>
  </AppLayout>
</template>

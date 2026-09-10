import { isTauri } from '@tauri-apps/api/core';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type DownloadEvent } from '@tauri-apps/plugin-updater';
import { ElLoading, ElMessage, ElMessageBox } from 'element-plus';
import { formatError } from './api';

let updateCheckStarted = false;

/** Check GitHub Releases once per launch and let the user install a signed update. */
export async function checkForAppUpdate(): Promise<void> {
  if (updateCheckStarted || !isTauri()) return;
  updateCheckStarted = true;

  let update;
  try {
    update = await check({ timeout: 30_000 });
  } catch (error) {
    // A background update check should not interrupt the scaffold workflow.
    console.warn(`检查应用更新失败：${formatError(error)}`);
    return;
  }

  if (!update) return;

  const releaseNotes = update.body?.trim();
  const notePreview = releaseNotes
    ? `\n\n更新说明：\n${releaseNotes.slice(0, 600)}${releaseNotes.length > 600 ? '…' : ''}`
    : '';

  try {
    await ElMessageBox.confirm(
      `当前版本：v${update.currentVersion}\n最新版本：v${update.version}${notePreview}`,
      '发现新版本',
      {
        type: 'info',
        confirmButtonText: '下载并更新',
        cancelButtonText: '稍后提醒',
        closeOnClickModal: false,
        dangerouslyUseHTMLString: false
      }
    );
  } catch {
    await update.close();
    return;
  }

  let downloadedBytes = 0;
  let contentLength: number | undefined;
  const loading = ElLoading.service({
    lock: true,
    text: `正在下载 v${update.version}…`,
    background: 'rgba(15, 23, 42, 0.72)'
  });

  const onDownloadEvent = (event: DownloadEvent) => {
    if (event.event === 'Started') {
      contentLength = event.data.contentLength;
      return;
    }
    if (event.event === 'Progress') {
      downloadedBytes += event.data.chunkLength;
      if (contentLength) {
        const percent = Math.min(100, Math.round((downloadedBytes / contentLength) * 100));
        loading.setText(`正在下载 v${update.version}… ${percent}%`);
      }
      return;
    }
    loading.setText('下载完成，正在安装…');
  };

  try {
    await update.downloadAndInstall(onDownloadEvent, {
      timeout: 10 * 60_000,
      restartAfterInstall: true
    });
    loading.close();
    await ElMessageBox.alert('更新安装完成，应用将立即重启。', `v${update.version}`, {
      type: 'success',
      confirmButtonText: '立即重启',
      closeOnClickModal: false,
      closeOnPressEscape: false,
      showClose: false
    });
    await relaunch();
  } catch (error) {
    loading.close();
    ElMessage.error(`自动更新失败：${formatError(error)}`);
  }
}

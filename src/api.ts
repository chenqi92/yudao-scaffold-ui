import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { RunPayload, ScaffoldEvent, ScaffoldMeta } from './types';

/**
 * Tauri's `invoke()` rejects with the raw string returned by `Err(String)` on
 * the Rust side, so caught values are usually plain strings — accessing
 * `.message` on them yields `undefined`. This normalizes any thrown value
 * into a human-readable string.
 */
export function formatError(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === 'object') {
    const msg = (e as { message?: unknown }).message;
    if (typeof msg === 'string') return msg;
    try {
      return JSON.stringify(e);
    } catch {
      return String(e);
    }
  }
  return String(e);
}

/** Fetch catalog metadata + template presence from the engine. */
export async function loadMeta(workspace?: string): Promise<ScaffoldMeta> {
  return invoke<ScaffoldMeta>('load_meta', { workspace });
}

/** Pop a native directory picker. Returns absolute path or null on cancel. */
export async function pickDirectory(initial?: string): Promise<string | null> {
  const result = await open({
    directory: true,
    multiple: false,
    defaultPath: initial
  });
  return typeof result === 'string' ? result : null;
}

/** Reveal a directory in the system file manager (Finder / Explorer). */
export async function revealInFinder(path: string): Promise<void> {
  await invoke('reveal_in_finder', { path });
}

/** Whether the given absolute path currently exists on disk. */
export async function pathExists(path: string): Promise<boolean> {
  return invoke<boolean>('path_exists', { path });
}

export interface RunHandle {
  unlisten: UnlistenFn;
}

/**
 * Start the engine run. Events stream via the `scaffold-event` Tauri event;
 * the returned promise resolves with the final exit code (0 = success).
 */
export async function startScaffold(
  payload: RunPayload,
  onEvent: (e: ScaffoldEvent) => void
): Promise<{ code: number; handle: RunHandle }> {
  const unlisten = await listen<ScaffoldEvent>('scaffold-event', (msg) => {
    onEvent(msg.payload);
  });
  try {
    const code = await invoke<number>('run_scaffold', { payload });
    return { code, handle: { unlisten } };
  } catch (err) {
    unlisten();
    throw err;
  }
}

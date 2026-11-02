import { confirm } from '@tauri-apps/plugin-dialog';

/**
 * Native confirmation for consequential actions. Never call `window.confirm`:
 * in the desktop app the dialog plugin replaces it with an async function, so
 * its Promise is always truthy and the action would run without asking.
 * A dialog that cannot open rejects, and the caller must not proceed.
 */
export function confirmAction(message: string, okLabel: string): Promise<boolean> {
  return confirm(message, { title: 'Rhythm', kind: 'warning', okLabel, cancelLabel: '취소' });
}

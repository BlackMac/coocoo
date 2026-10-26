// Desktop auto-update state driven from the native Tauri shell.
// Active only when running inside the desktop app (`fleet.native === true`).

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type DesktopUpdateStatus =
  | { status: 'idle'; currentVersion: string }
  | { status: 'unsupported'; currentVersion: string; reason: string }
  | { status: 'checking'; currentVersion: string }
  | { status: 'upToDate'; currentVersion: string; checkedAt: number }
  | { status: 'available'; version: string; currentVersion: string; body?: string; date?: string }
  | { status: 'downloading'; version: string; downloaded: number; total?: number; percent?: number }
  | { status: 'readyToRestart'; version: string }
  | { status: 'error'; message: string; currentVersion: string };

class DesktopUpdateState {
  status = $state<DesktopUpdateStatus | null>(null);
  #unlisten: UnlistenFn | null = null;
  #started = false;

  get isChecking(): boolean {
    return this.status?.status === 'checking';
  }

  get isAvailable(): boolean {
    return this.status?.status === 'available';
  }

  get isDownloading(): boolean {
    return this.status?.status === 'downloading';
  }

  get isReady(): boolean {
    return this.status?.status === 'readyToRestart';
  }

  get availableVersion(): string | null {
    if (!this.status) return null;
    if (this.status.status === 'available' || this.status.status === 'downloading' || this.status.status === 'readyToRestart') {
      return this.status.version;
    }
    return null;
  }

  get currentVersion(): string | null {
    if (!this.status) return null;
    if ('currentVersion' in this.status) {
      return this.status.currentVersion;
    }
    return null;
  }

  start() {
    if (this.#started) return;
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    this.#started = true;

    // Listen to real-time update events emitted by Rust.
    listen<DesktopUpdateStatus>('update://status', (event) => {
      this.status = event.payload;
    })
      .then((un) => {
        this.#unlisten = un;
      })
      .catch((err) => {
        console.warn('cannot listen to update://status', err);
      });

    // Query current status on startup.
    invoke<DesktopUpdateStatus>('update_status')
      .then((s) => {
        this.status = s;
      })
      .catch((err) => {
        console.warn('cannot fetch desktop update status', err);
      });
  }

  stop() {
    this.#unlisten?.();
    this.#unlisten = null;
    this.#started = false;
  }
  async check(): Promise<void> {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    try {
      this.status = await invoke<DesktopUpdateStatus>('update_check');
    } catch (err) {
      console.error('desktop update check failed', err);
    }
  }

  async install(): Promise<void> {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    try {
      await invoke('update_install');
    } catch (err) {
      console.error('desktop update install failed', err);
    }
  }
}

export const desktopUpdate = new DesktopUpdateState();

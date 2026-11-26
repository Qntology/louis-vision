// apps/shared-ui/js/bridge-client.js
// Unified dual-app IPC client: uses window.__TAURI__.core.invoke when running inside Tauri v2,
// and falls back seamlessly to mock-dual-backend.js in standalone browser execution.

import { MockDualBackend } from './mock-dual-backend.js';

let singletonBackend = null;

export function hasTauriRuntime() {
  return typeof window !== 'undefined' && Boolean(window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke);
}

export function getMockBackend() {
  if (!singletonBackend) {
    singletonBackend = new MockDualBackend();
  }
  return singletonBackend;
}

export async function invokeDual(cmd, args = {}) {
  if (typeof window !== 'undefined') {
    const tauriInvoke = window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke;
    if (typeof tauriInvoke === 'function') {
      return tauriInvoke(cmd, args);
    }
  }
  return getMockBackend().invoke(cmd, args);
}

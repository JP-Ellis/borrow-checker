import { test as base } from '@playwright/test';

// Stubs window.__TAURI_INTERNALS__ so tauri-sys does not crash outside the
// Tauri webview. Returns null for every command by default. Tests that need
// specific responses can call page.addInitScript() again before navigating —
// later scripts override this default.
const TAURI_STUB = `
  window.__TAURI_INTERNALS__ = {
    isTauri: true,
    metadata: { debug: true, version: '0.0.0' },
    invoke: async (_cmd, _args) => null,
    transformCallback: (fn, once) => {
      const uid = Math.floor(Math.random() * 0x100000000);
      const wrapped = once
        ? (...a) => { delete window['_' + uid]; return fn(...a); }
        : fn;
      window['_' + uid] = wrapped;
      return uid;
    },
  };
`;

export const test = base.extend<{ _ipcStub: void }>({
  _ipcStub: [
    async ({ page }, use) => {
      await page.addInitScript(TAURI_STUB);
      await use();
    },
    { auto: true },
  ],
});

export { expect } from '@playwright/test';

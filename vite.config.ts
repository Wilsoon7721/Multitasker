import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
import { defineConfig } from 'vite';

const ui = resolve(import.meta.dirname, 'ui');

// Tauri serves the build from ../dist and expects the dev server on a fixed port.
export default defineConfig({
  root: ui,
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
  build: {
    outDir: resolve(import.meta.dirname, 'dist'),
    emptyOutDir: true,
    target: 'es2022',
    rollupOptions: {
      input: {
        main: resolve(ui, 'index.html'),
        widget: resolve(ui, 'widget.html'),
        switcher: resolve(ui, 'switcher.html'),
      },
    },
  },
});

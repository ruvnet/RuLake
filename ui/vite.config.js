import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

// GitHub Pages serves the project at /RuLake/ — set base accordingly.
// Override with VITE_BASE for custom-domain deploys.
const base = process.env.VITE_BASE ?? '/RuLake/';

export default defineConfig({
  base,
  plugins: [react()],
  server: {
    port: 5173,
    host: '127.0.0.1',
  },
  build: {
    rollupOptions: {
      input: {
        console: fileURLToPath(new URL('./index.html', import.meta.url)),
        lakes: fileURLToPath(new URL('./lakes/index.html', import.meta.url)),
      },
    },
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2020',
    sourcemap: true,
  },
  // Allow .jsx files (the design ships as JSX)
  esbuild: {
    loader: 'jsx',
    include: /src\/.*\.(jsx|js)$/,
    exclude: [],
  },
  optimizeDeps: {
    esbuildOptions: { loader: { '.js': 'jsx' } },
  },
});

import { defineConfig } from 'vite';

// Minimal config. The Rust sim is built separately by scripts/build-wasm.sh into
// src/wasm/, then imported here; the `.wasm?url` import (see main.ts) lets Vite
// treat the binary as a hashed asset in both dev and production builds.
export default defineConfig({
  server: { port: 5173, open: false },
});

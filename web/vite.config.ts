import path from "path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Served from the todone binary on localhost, so bundle size barely matters.
  build: { chunkSizeWarningLimit: 1500 },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
})

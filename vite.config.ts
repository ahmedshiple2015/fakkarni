import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    watch: {
      // Rust executables may be locked while running on Windows.
      ignored: ["**/src-tauri/**", "**/target/**", "**/.tools/**", "**/artifacts/**"],
    },
    fs: {
      deny: [
        ".env", ".env.*", "*.{crt,pem}", "**/.git/**",
        "**/*.kdbx", "**/*.kdbx.*", "**/*.key", "**/*.keyx",
        "**/.tools/**", "**/target/**", "**/artifacts/**",
      ],
    },
  },
});

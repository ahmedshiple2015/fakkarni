import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const compiler = fileURLToPath(
  new URL("../node_modules/typescript/bin/tsc", import.meta.url),
);
const config = fileURLToPath(new URL("./tsconfig.json", import.meta.url));
const result = spawnSync(process.execPath, [compiler, "-p", config], {
  stdio: "inherit",
});
process.exit(result.status ?? 1);

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const cargoPath = join(
  dirname(fileURLToPath(import.meta.url)),
  "../crates/repoharbor/Cargo.toml",
);

/** VitePress data loader — Node-only, not bundled into the client. */
export default {
  watch: [cargoPath],
  load(): string {
    const cargo = readFileSync(cargoPath, "utf8");
    return cargo.match(/^version = "([^"]+)"/m)?.[1] ?? "0.0.0";
  },
};

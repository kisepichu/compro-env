// @ts-check
import { fileURLToPath } from "node:url";

import { defineConfig } from "astro/config";

const rawSite = process.env.CE_SITE_ORIGIN ?? "http://localhost:4321";
const rawBase = process.env.CE_SITE_BASE ?? "/";

const site = rawSite.endsWith("/") ? rawSite.slice(0, -1) : rawSite;

function normalizeBase(input) {
  if (input === "" || input === "/") return "/";
  let value = input.startsWith("/") ? input : `/${input}`;
  if (!value.endsWith("/")) value = `${value}/`;
  return value;
}

export default defineConfig({
  site,
  base: normalizeBase(rawBase),
  build: {
    format: "directory",
  },
  trailingSlash: "always",
  vite: {
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src/", import.meta.url)),
      },
    },
  },
});

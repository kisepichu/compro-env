/**
 * Resolve `UrlConfig` from build-time env variables.
 *
 * Astro/Vite does not expose arbitrary env variables via `import.meta.env`
 * unless they carry a `PUBLIC_` prefix or `envPrefix` is configured, so
 * server-side page code reads from `process.env` — the same source
 * `astro.config.mjs` uses to compute Astro's own `site` and `base`. Tests
 * always pass a fixed `UrlConfig` directly and do not depend on this
 * helper.
 */

import type { UrlConfig } from "../url.ts";

export interface EnvSource {
  CE_SITE_ORIGIN?: string;
  CE_SITE_BASE?: string;
}

function normalizeBase(input: string): string {
  if (input === "" || input === "/") return "/";
  let value = input.startsWith("/") ? input : `/${input}`;
  if (!value.endsWith("/")) value = `${value}/`;
  return value;
}

export function resolveUrlConfig(env: EnvSource = process.env as EnvSource): UrlConfig {
  const rawOrigin = env.CE_SITE_ORIGIN ?? "http://localhost:4321";
  const origin = rawOrigin.endsWith("/") ? rawOrigin.slice(0, -1) : rawOrigin;
  const base = normalizeBase(env.CE_SITE_BASE ?? "/");
  return { origin, base };
}

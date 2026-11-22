import { defineConfig } from 'astro/config';

// Production is https://robroz.dev with base `/`.
// Override with SITE_URL / BASE_PATH if needed.
const site = process.env.SITE_URL || 'https://robroz.dev';
const base = process.env.BASE_PATH ?? '/';

export default defineConfig({
  site,
  base,
  output: 'static',
  trailingSlash: 'always',
  i18n: {
    defaultLocale: 'en',
    locales: ['en', 'zh'],
    routing: {
      prefixDefaultLocale: false,
    },
  },
});

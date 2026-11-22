import { defineConfig } from 'astro/config';

// Preview at https://pandaquq.github.io/robroz/ uses base `/robroz`.
// For robroz.dev, set SITE_URL=https://robroz.dev and BASE_PATH=/.
const site = process.env.SITE_URL || 'https://pandaquq.github.io';
const base = process.env.BASE_PATH ?? '/robroz';

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

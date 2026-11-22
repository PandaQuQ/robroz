# robroz.dev site

Static landing page for [robroz](https://github.com/PandaQuQ/robroz), built with [Astro](https://astro.build). English is the default at `/`; Simplified Chinese lives at `/zh/`. The language switch does not follow the browser locale.

Copy lives in `src/i18n/en.ts` and `src/i18n/zh.ts`, keyed the same way as the docs draft (`hero.title`, `features.memory.body`, and so on). English is the default; keep the two files aligned. On the English page, UI strings inside demo cards stay in Chinese; the English gloss is a single grey line under the card (hover on desktop, always visible on mobile).

## Local development

Node 22+ is required.

```sh
cd site
npm ci
npm run dev
```

The dev server defaults to base path `/`, matching https://robroz.dev. Open `http://localhost:4321/`.

```sh
npm run build    # writes to dist/
npm run preview  # serves dist/ with the same base path
```

`site/public/CNAME` is `robroz.dev`, so a deploy from `main` publishes the custom domain to GitHub Pages. Do not merge this until DNS is ready.

## Deploy

A GitHub Actions workflow (`.github/workflows/site.yml`) builds this directory and, on pushes to `main`, deploys to GitHub Pages. Pull requests that touch `site/**` only run the build.

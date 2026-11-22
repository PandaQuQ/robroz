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

The dev server defaults to base path `/robroz`, matching the GitHub Pages preview at `https://pandaquq.github.io/robroz/`. Open `http://localhost:4321/robroz/`.

```sh
npm run build    # writes to dist/
npm run preview  # serves dist/ with the same base path
```

To build as if the custom domain were already live:

```sh
SITE_URL=https://robroz.dev BASE_PATH=/ npm run build
```

Do not add a `CNAME` file until DNS for robroz.dev is in place.

## Deploy

A GitHub Actions workflow (`.github/workflows/site.yml`) builds this directory and, on pushes to `main`, deploys to GitHub Pages. Pull requests that touch `site/**` only run the build.

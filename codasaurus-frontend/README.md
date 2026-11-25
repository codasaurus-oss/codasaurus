# Codasaurus Dashboard

Web dashboard for [Codasaurus](https://github.com/codasaurus-oss/codasaurus), a Rust
service that reviews pull requests with LLM agents and reports repository risk.

This repository is the upstream source for the React + shadcn interface. It is a
static single-page app: the Rust backend serves the built `dist/` and exposes the
API it calls. The source is mirrored into the backend monorepo at
`codasaurus/codasaurus-frontend`; Docker users do not need this repository
separately. Clone and run this repo only when developing UI upstream.

## Stack

| Layer | Choice |
| --- | --- |
| Build | Vite 8 |
| UI | React 19, TypeScript 7, Tailwind CSS 4 |
| Components | shadcn, Base UI, neutral base color (`components.json`) |
| Routing | React Router 8, hash mode (`createHashRouter`) |
| Data | TanStack Query 5 |
| Forms | React Hook Form 7 with Zod 4 |
| Unit tests | Vitest 5, Testing Library, jsdom |
| End-to-end tests | Playwright 1.65, Chromium |
| Lint | Oxlint |

## Prerequisites

- Node.js >= 22.12. Installed dependencies declare it: `cn` requires `>=22`,
  Vitest 5 requires `^22.12.0 || ^24.0.0 || >=26.0.0`, Vite 8 requires
  `^20.19.0 || >=22.12.0`.
- npm (the repo commits `package-lock.json`).

## Scripts

| Command | What it does |
| --- | --- |
| `npm run dev` | Vite dev server with API proxy |
| `npm run build` | Type-check with `tsc -b`, then build to `dist/` |
| `npm run preview` | Serve the production build locally |
| `npm run lint` | Oxlint |
| `npm test` | Vitest unit tests, one run |
| `npm run test:e2e` | Playwright suite against the production build |

## Local development

```bash
npm ci
npm run dev
```

Vite proxies `/api` and `/health` to the backend. The default target is
`http://127.0.0.1:3000`; point it elsewhere with `CODASAURUS_API`:

```bash
CODASAURUS_API=http://127.0.0.1:8080 npm run dev
```

Run the backend with `codasaurus serve` from the
[backend repository](https://github.com/codasaurus-oss/codasaurus).

## Production build and backend integration

`npm run build` writes the static bundle to `dist/`. The backend does not build
this repo at runtime:

- The backend release workflow builds the vendored `codasaurus-frontend/` tree,
  then copies `dist` to `svelte-dashboard/dist`.
- The backend Dockerfile does the same in a Node stage and copies the result to
  `/app/svelte-dashboard/dist` in the runtime image.

Keep this repository and the monorepo subtree in sync with `git subtree pull`
and `git subtree push`; the commands are documented in the backend README.

## Routes

Routing uses hash URLs so the backend can serve one `index.html` for every path.
Backend redirects depend on these routes; do not rename them without changing the
backend:

| Route | Reached from |
| --- | --- |
| `#/setup/github` | Setup flow and GitHub App callback |
| `#/setup/complete` | Setup completion redirect |
| `#/invite/:token` | Invite links sent to users |
| `#/app/dashboard` | Post-login redirect |

## Authentication notes

- OIDC sign-in (`/api/auth/oidc/login`) always returns to `#/app/dashboard`: the
  backend callback accepts no `next` or return-path parameter, so deep links are
  not preserved. Password sign-in redirects to the route that triggered it.
- Invite links carry the token in the URL hash fragment. Hash fragments can land
  in browser history, so treat invite URLs as secrets and share them only with
  the intended recipient.
- The settings UI is owner-only. The backend settings read and test endpoints
  also allow maintainers, with secret values masked.

## Testing

```bash
npm test           # unit tests
npm run test:e2e   # Playwright, Chromium only
```

The Playwright suite runs the production build through `vite preview` on
`127.0.0.1:4173`. Critical flows in `src/tests/e2e/critical.spec.ts` intercept
`/api/**` and `/health` with Playwright route mocks, so they exercise the
frontend without a live backend.

Install the browser once before the first run:

```bash
npx playwright install --with-deps chromium
```

## Links

- Backend and release notes: <https://github.com/codasaurus-oss/codasaurus>
- Issues for either repository: <https://github.com/codasaurus-oss/codasaurus/issues>

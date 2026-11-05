# TypeScript Nx workspace

This workspace contains the trading React application in `apps/react-app`, the service-control homepage in `apps/homepage`, and the benchmark report in `apps/benchmarks`.

## Install

From this directory, install dependencies for all npm workspaces:

```sh
npm install
```

## Run the app

```sh
npx nx serve react-app
```

## Live homepage and benchmark development

Run both Vite development servers from the workspace directory:

```sh
npx nx run-many -t serve -p homepage benchmarks --parallel=2
```

Open `http://localhost:5174` for the service-control homepage. Its Benchmarks
link is proxied to the hot-reloading report at `http://localhost:5175`. Edits
to either app's source are picked up by Vite, and edits to
`research/BENCHMARK_BASELINE.md` reload the Benchmarks report. These dev servers
are separate from the production files served at `http://localhost:8080`.

## Nx targets

```sh
npx nx build react-app
npx nx lint react-app
npx nx typecheck react-app
npx nx preview react-app

npx nx serve homepage
npx nx build homepage
npx nx lint homepage
npx nx typecheck homepage

npx nx serve benchmarks
npx nx build benchmarks
npx nx lint benchmarks
npx nx typecheck benchmarks
```

The build target runs the app's TypeScript build and Vite production build. Nx
caches the generated `apps/react-app/dist` output.

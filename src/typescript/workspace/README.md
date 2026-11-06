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

The infrastructure stack runs the Benchmarks Vite server with the report
source mounted from the repository. Open `http://localhost:8080/benchmarks/`
to use the live page alongside the main app; edits to benchmark source files
and `research/BENCHMARK_BASELINE.md` appear without rebuilding.

For standalone homepage development, run its Vite server from the workspace
directory:

```sh
npx nx serve homepage
```

Open `http://localhost:5174` for the service-control homepage. Its Benchmarks
link reaches the live report through the local service-control route.

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

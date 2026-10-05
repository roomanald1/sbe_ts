# TypeScript Nx workspace

This workspace contains the React application in `apps/react-app`.

## Install

From this directory, install dependencies for all npm workspaces:

```sh
npm install
```

## Run the app

```sh
npx nx serve react-app
```

## Nx targets

```sh
npx nx build react-app
npx nx lint react-app
npx nx typecheck react-app
npx nx preview react-app
```

The build target runs the app's TypeScript build and Vite production build. Nx
caches the generated `apps/react-app/dist` output.

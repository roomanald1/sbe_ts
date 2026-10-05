# React app

This React and Vite application is managed by the Nx workspace at
`src/typescript/workspace`.

Run the app and its checks from the workspace root:

```sh
npx nx serve react-app
npx nx build react-app
npx nx lint react-app
npx nx typecheck react-app
```

The app's original npm scripts remain available when working from this
directory.

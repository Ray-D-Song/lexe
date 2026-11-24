# Running TypeScript with Lexe

Same principle as dependencies applies when using TypeScript. TypeScript must be bundled and transpiled into ES2023 JavaScript.

Install [`lexe-types`](../types/package.json) for TypeScript definitions matching Lexe's runtime APIs:

```shell
npm install -D lexe-types
```

> [!NOTE]
> Lexe will not support running TypeScript without transpilation. This is by design for performance reasons. Transpiling requires CPU and memory that adds latency and cost during execution. This can be avoided if done ahead of time during deployment.

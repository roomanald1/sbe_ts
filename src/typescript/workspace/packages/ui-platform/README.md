TODO 

- move ag-grid styles here

## v35 operator dashboard primitives

Import `@workspace/ui-platform/styles/operator_dashboard.css` once from the app
entry point, then compose `OperatorPanel`, `OperatorMetricStrip`,
`OperatorStatus`, and `OperatorTabBar` from the package root. These present only
the values and state supplied by the owning Model/view; they do not calculate
financial metrics, choose defaults, or fetch data. Use an explicit value such
as `N/A` when a measurement is unavailable and omit it only when the owning
widget contract calls for omission.

```tsx
<OperatorPanel title="Execution quality" context="All exchanges · 24h">
  <OperatorMetricStrip
    metrics={[
      { label: 'Submitted', value: view.submitted },
      { label: 'Filled', value: view.filled },
      { label: 'Fill rate', value: view.fillRate ?? 'N/A' },
    ]}
  />
</OperatorPanel>
```

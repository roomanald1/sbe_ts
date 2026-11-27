# Mandatory UI Design Quality Review and Implementation

The current trading widgets do not meet the approved visual design requirements.

Although some functional requirements have been implemented, the presentation resembles a developer diagnostics interface rather than a professional trading terminal.

The previously supplied design screenshots are visual acceptance references, not merely illustrations of the required data fields.

Review and substantially improve all eight widgets against those references.

## 1. Design principles

The target is a professional, information-dense trading terminal with a consistent visual language.

Prioritise:

- High information density without visual clutter.
- Clear visual hierarchy.
- Compact, aligned tables.
- Consistent typography and numeric formatting.
- Small, meaningful summary metrics.
- Purposeful visualisations.
- Minimal unnecessary whitespace.
- Efficient use of horizontal and vertical space.
- Progressive disclosure of technical details.
- Consistent spacing, colours, borders and controls.

Do not simply render every available backend field as a separate card, label or paragraph.

The primary view should answer the operator's question immediately, with secondary technical information available through drill-down.

## 2. Shared UI platform architecture

The existing workspace contains a dedicated shared UI package:

`src/typescript/workspace/packages/ui-platform`

The trading application resides in:

`src/typescript/workspace/apps/react-app`

Maintain a strict separation between the reusable UI platform and application-specific functionality.

### ui-platform responsibilities

All reusable visual components, common styles and design-system functionality must reside in `ui-platform`.

This includes:

- Design tokens.
- Colour palettes and semantic colours.
- Typography.
- Spacing and layout primitives.
- Theme management.
- Common CSS and styling utilities.
- Compact metric summaries.
- Dense data tables.
- Table sorting and filtering presentation.
- Widget toolbars.
- Tabs and segmented controls.
- Time-range selectors.
- Filter controls.
- Status indicators.
- Progress and distribution bars.
- Diverging value bars.
- Heatmaps.
- Generic chart components.
- Expandable table rows.
- Drill-down navigation components.
- Tooltips and popovers.
- Empty, unavailable, stale, error and loading states.
- Generic responsive layout behaviour.

Review the existing package before implementing new components.

Reuse and extend existing components wherever possible. Do not create duplicate components or competing design systems.

### No business-specific logic in ui-platform

The UI platform must remain completely independent of the trading domain.

It must not contain:

- AutoTrader-specific components or logic.
- Strategy-specific logic.
- Trading eligibility calculations.
- Capital allocation calculations.
- Order or position lifecycle logic.
- Financial reconciliation logic.
- P&L or fee calculations.
- Scout or Learning-specific behaviour.
- Exchange-specific assumptions.
- Trading service contracts.
- Trading API requests or subscriptions.
- Application-specific models or state.
- Dependencies on react-app.

Reusable components must accept generic, typed presentation properties and callbacks.

For example:

- A generic diverging value bar belongs in ui-platform.
- Calculating net breakeven, executable P&L and stop-loss positions belongs in the trading application.
- A generic heatmap belongs in ui-platform.
- Constructing a strategy eligibility matrix belongs in the trading application.
- A generic progress bar belongs in ui-platform.
- Calculating effective evidence coverage belongs in the trading application.
- A generic sortable table belongs in ui-platform.
- Defining trading columns, retrieving orders and managing order filters belongs in the trading application.

Do not introduce trading terminology into generic component APIs where domain-independent terminology is appropriate.

### react-app responsibilities

The application owns:

- Trading-specific widgets.
- Application-specific component composition.
- Trading-specific column definitions.
- Domain models and contracts.
- Service communication.
- Data transformation and presentation mapping.
- Business-specific state and lifecycle management.
- Trading-specific interactions and commands.

Application-specific composition should consume shared visual components rather than recreating them.

The dependency direction must remain:

`react-app → ui-platform`

Never introduce a reverse dependency.

### Shared component quality

New or materially changed shared components must have appropriate automated tests covering their generic behaviour, rendering and interactions.

Validate shared components independently of trading-specific data.

Use existing workspace tooling and package conventions. Do not introduce another UI framework or component library without approval.

## 3. Information density and layout

Use the approved design references to establish appropriate visual density and proportions.

- Keep widget padding and spacing compact.
- Use compact table rows and controls.
- Establish a clear typographic hierarchy between headings, primary metrics, table values and secondary labels.
- Right-align numeric table columns.
- Use tabular numeric typography.
- Apply consistent decimal precision appropriate to the instrument and metric.
- Align related information across rows and columns.
- Use horizontal space effectively.
- Group related metrics rather than creating individual containers for every value.
- Use responsive layouts and shared design tokens rather than arbitrary fixed dimensions.

Avoid oversized cards, excessive borders, large headings, nested containers and unnecessary vertical spacing.

Do not stretch summary content simply to fill the available widget area.

Do not duplicate common CSS or introduce widget-specific styling overrides where a shared component or design token would solve the problem.

## 4. Correct the current widget designs

### AutoTrader Controls

Replace the oversized status banner with a compact status and controls row.

Clearly distinguish enabled/disabled status from Paper/Live execution mode. An active paper trader must not misleadingly appear to be trading live capital.

Use a compact two-column settings layout with aligned labels and values.

Keep operational controls immediately accessible.

Remove explanatory paragraphs from the primary view. Represent unavailable allocation details with a concise status and expandable explanation.

### Decision Funnel

The current presentation of repeated cycle IDs and diagnostic paragraphs is unacceptable as the primary view.

Implement the approved visual funnel, breakdown table and heatmap.

Show aggregate stage counts, conversion rates and dominant blockers.

Use compact horizontal bars to communicate progression.

Group repeated rejection reasons into meaningful summaries while preserving the underlying individual records.

Provide sortable tables by symbol, strategy, direction, exchange and profile.

Show raw cycle IDs, timestamps, observed thresholds and detailed rejection evidence only when drilling into the relevant records.

### Orders & Positions

Replace individual field cards with compact, aligned position and order tables.

Implement the approved Net vs Breakeven position bar directly within each open position row.

Show current executable net return, stop loss, take profit and profit protection visually.

Use a compact summary strip above the table.

Move detailed technical metadata into expandable rows.

When there are no positions, show one concise empty state. Do not render multiple empty distribution sections or explanatory paragraphs.

### Execution Quality

Use compact summary metrics, stacked outcome bars and sortable breakdown tables.

Make fill rates, rejection rates, cancellations, expiries and execution costs immediately understandable.

Implement the approved individual order drill-down and price-distance visualisation.

Do not render raw order lifecycle records as the primary presentation.

### Portfolio & Reconciliation

Use compact financial summaries, selectable historical charts and financial breakdown tables.

Visually distinguish gross gains, gross losses, fees and net realised P&L.

Show reconciliation as a compact status indicator with expandable supporting evidence.

### Capital Allocation

Use a shared capital utilisation bar showing deployed, reserved and available capital.

Provide compact allocation and candidate tables.

Make unused capital and binding allocation constraints immediately understandable.

Do not present allocation information as explanatory prose.

### Strategy Performance

Use compact sortable performance tables, heatmaps and selectable historical charts.

Provide clear visual comparisons of wins, losses, fees, net returns and strict versus exploratory performance.

Preserve separate AutoTrader and Scout modes.

### Learning & Evidence Acquisition

Use compact evidence coverage bars, effective sample counts, acquisition status and blocker summaries.

Make it immediately apparent which profiles are learning, blocked, eligible or progressing towards eligibility.

Expose Scout learning effectiveness and accepted evidence gains visually.

Move detailed acquisition diagnostics into expandable records.

## 5. Handle missing data properly

Preserve all existing evidence integrity requirements.

However, unavailable information must not dominate the interface.

- Use a compact dash or unavailable indicator in table cells.
- Provide the reason through a tooltip or expandable detail.
- Consolidate shared missing-data explanations into one concise status.
- Distinguish zero, unknown, stale, unavailable and insufficient evidence.
- Never fabricate values to make a visualisation appear complete.

Implement the intended visual components even when current runtime data is sparse. Use deterministic populated fixtures to verify their appearance.

Generic missing-data presentation belongs in ui-platform. Determining the meaning and cause of missing trading data belongs in react-app.

## 6. Preserve the Model.ts architecture

Every application view must have a corresponding tested Model.ts.

The model owns:

- UI lifecycle.
- Subscriptions.
- Requests.
- Observable state.
- Data transformation.
- Filtering and sorting.
- Pagination.
- Navigation and drill-down state.
- Configuration persistence.
- Error handling.
- Recovery.
- Request cancellation.
- Resource cleanup.

Follow the existing model and observable state architecture, for example:

```tsx
const model = useMemo(
  () => new MarketLiquidityModel({ connection, symbol, streamKey: key }),
  [],
);
const state = useBehaviorSnapshot(model.state$);
```

Use the appropriate dependency and lifecycle handling for each model. Do not blindly copy the empty dependency array when constructor inputs can change.

TSX files must remain presentation-only, binding rendered controls to model state and commands and forwarding necessary React lifecycle events.

Do not move business logic into React components or into ui-platform as part of the redesign.

Application-specific models must remain in react-app.

Generic UI platform components may manage their own purely presentational interaction state, but must not own application lifecycle or business state.

Preserve authoritative backend contracts and existing financial and trading semantics.

## 7. Responsive widget behaviour

All widgets must work within the existing configurable dashboard.

Test each widget at representative small, medium and large sizes.

Use available width intelligently.

At smaller sizes, prioritise summary metrics and essential columns. At larger sizes, expose additional breakdowns and charts.

Do not simply scale up cards, increase whitespace or allow text to overflow.

Preserve independent widget instances and persisted configurations.

Where responsive behaviour is reusable, implement it within ui-platform rather than duplicating it across widgets.

## 8. Mandatory visual validation

Typechecking, unit tests and successful compilation are not sufficient to accept UI work.

For every widget:

1. Review the supplied approved design reference.
2. Render the implemented widget in an actual browser.
3. Capture a screenshot at a comparable widget size.
4. Inspect the rendered screenshot and compare the implementation visually against the approved reference.
5. Identify differences in layout, density, typography, hierarchy, visualisations and interactions.
6. Correct those differences.
7. Repeat until the implementation meets the intended design quality.

Use browser automation and screenshot capture through existing project tooling.

Validate both populated fixtures and actual runtime data.

If visual inspection is unavailable, explicitly report that visual acceptance remains unverified. Do not claim the design is complete.

## 9. Implementation approach

Begin by reviewing the existing ui-platform package and identifying:

- Existing reusable components.
- Existing design tokens and styles.
- Components requiring improvement.
- Missing reusable components.
- Duplicated visual components currently residing in react-app.
- Existing application-specific styling that should be generalised.

Use the Decision Funnel as the first complete visual implementation.

Establish and validate the shared visual components through this widget before applying the same visual language to the remaining widgets.

Avoid speculative abstractions and unnecessary framework development. Extract reusable functionality based on the actual requirements of the eight approved widgets.

Preserve unrelated application functionality when changing shared components.

## 10. Acceptance and review

Deliver the redesigned widgets individually for visual review.

For each widget, provide:

- Screenshot of the rendered implementation.
- Screenshot with representative populated data.
- Evidence of responsive behaviour.
- Summary of remaining deviations from the approved reference.
- Model and interaction test results.
- Shared ui-platform components reused or introduced.
- Confirmation that business-specific logic remains outside ui-platform.

Acceptance also requires:

- No duplicated common visual components.
- No duplicated common styles.
- No trading-specific logic or contracts in ui-platform.
- No reverse dependencies from ui-platform to react-app.
- Appropriate tests for shared components.
- Appropriate tests for application models.
- Presentation-only application TSX files.
- Consistent visual language across all eight widgets.

Do not mark a widget complete solely because its required fields are present.

Do not mark visual acceptance complete without reviewing the rendered implementation against the supplied references.

Continue backend implementation independently. This redesign must not weaken trading safeguards, change trading criteria or compromise authoritative data semantics.

The objective is a professional trading interface that makes complex system behaviour immediately understandable, supported by a reusable, domain-independent UI platform — not a collection of backend diagnostic fields rendered into React.
import type { FeedSource } from '../connection';
import { DASHBOARD_WIDGET_TYPES, WIDGET_LABELS, type DashboardWidgetType, type SavedDashboardLayout } from './dashboard_layout';

type Props = {
  dataView: 'markets' | 'users';
  onDataViewChange: (view: 'markets' | 'users') => void;
  feedSource: FeedSource;
  onFeedSourceChange: (source: FeedSource) => void;
  layouts: SavedDashboardLayout[];
  activeLayoutId: string;
  onChooseLayout: (id: string) => void;
  name: string;
  onNameChange: (name: string) => void;
  onSave: () => void;
  onRenameStart: () => void;
  widgetStoreOpen: boolean;
  onWidgetStoreToggle: () => void;
  onAddWidget: (type: DashboardWidgetType) => void;
};

export function DashboardToolbar(props: Props) {
  const { dataView, onDataViewChange, feedSource, onFeedSourceChange, layouts, activeLayoutId,
    onChooseLayout, name, onNameChange, onSave, onRenameStart, widgetStoreOpen,
    onWidgetStoreToggle, onAddWidget } = props;
  return <nav className="feed-controls" aria-label="Main navigation">
    <div className="view-switcher" role="group" aria-label="Main sections">
      <button type="button" aria-pressed={dataView === 'markets'} onClick={() => onDataViewChange('markets')}>Markets</button>
      <button type="button" aria-pressed={dataView === 'users'} onClick={() => onDataViewChange('users')}>Users</button>
    </div>
    {dataView === 'markets' && <>
      <div className="view-switcher" role="group" aria-label="Choose price feed">
        <button type="button" aria-pressed={feedSource === 'okx'} onClick={() => onFeedSourceChange('okx')}>OKX</button>
        <button type="button" aria-pressed={feedSource === 'synthetic'} onClick={() => onFeedSourceChange('synthetic')}>Synthetic · Fast</button>
      </div>
      <div className="workspace-controls">
        <label className="workspace-selector"><span>WORKSPACE</span>
          <select aria-label="Choose workspace layout" value={activeLayoutId} onChange={(event) => onChooseLayout(event.target.value)}>
            {layouts.map((layout) => <option key={layout.id} value={layout.id}>{layout.name}</option>)}
          </select>
        </label>
        <button className="workspace-save-button" type="button" aria-label="Rename workspace layout" onClick={onRenameStart}>Rename</button>
        <input className="workspace-name-input" aria-label="Workspace layout name" placeholder="Save layout as..." value={name} onChange={(event) => onNameChange(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') onSave(); }} />
        <button className="workspace-save-button" type="button" onClick={onSave} disabled={!name.trim()}>Save</button>
      </div>
      <div className="widget-store-menu">
        <button className="widget-store-toggle" type="button" aria-label="Add widget" aria-expanded={widgetStoreOpen} aria-controls="widget-store-dropdown" onClick={onWidgetStoreToggle}>+</button>
        {widgetStoreOpen && <div className="widget-store-dropdown" id="widget-store-dropdown" aria-label="Add a dashboard widget">
          {DASHBOARD_WIDGET_TYPES.map((type) => <button type="button" key={type} onClick={() => onAddWidget(type)}>Add {WIDGET_LABELS[type]}</button>)}
        </div>}
      </div>
    </>}
  </nav>;
}

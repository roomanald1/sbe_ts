import type { PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { DASHBOARD_COLUMNS, type DashboardWidget } from './dashboard_layout';

type Props = {
  widget: DashboardWidget;
  layout: DashboardWidget;
  description: string;
  dragging: boolean;
  connected: boolean;
  children: ReactNode;
  onDragStart: (widget: DashboardWidget) => void;
  onDragEnd: () => void;
  onToggle: (id: string) => void;
  onRemove: (id: string) => void;
  onResize: (id: string, width: number, height: number) => void;
  onResizePreview: (preview: { id: string; x: number; y: number; w: number; h: number } | null) => void;
};

export function DashboardWidgetFrame(props: Props) {
  const { widget, layout, description, dragging, connected, children, onDragStart, onDragEnd, onToggle, onRemove, onResize, onResizePreview } = props;
  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startY = event.clientY;
    const startW = widget.w;
    const startH = widget.h;
    const dashboard = event.currentTarget.closest('.market-dashboard');
    if (!dashboard) return;
    const rect = dashboard.getBoundingClientRect();
    const styles = window.getComputedStyle(dashboard);
    const gap = parseFloat(styles.columnGap) || 12;
    const rowHeight = parseFloat(styles.gridAutoRows) || 22;
    const unitW = (rect.width - gap * (DASHBOARD_COLUMNS - 1)) / DASHBOARD_COLUMNS + gap;
    const unitH = rowHeight + gap;
    const handle = event.currentTarget;
    let nextW = startW;
    let nextH = startH;
    handle.setPointerCapture(event.pointerId);
    const move = (pointerEvent: PointerEvent) => {
      nextW = Math.max(4, Math.min(DASHBOARD_COLUMNS, startW + Math.round((pointerEvent.clientX - startX) / unitW)));
      nextH = Math.max(4, startH + Math.round((pointerEvent.clientY - startY) / unitH));
      onResizePreview({ id: widget.id, x: Math.min(widget.x, DASHBOARD_COLUMNS - nextW), y: widget.y, w: nextW, h: nextH });
    };
    const finish = (pointerEvent: PointerEvent) => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', finish);
      handle.removeEventListener('pointercancel', finish);
      if (pointerEvent.type === 'pointerup' && (nextW !== startW || nextH !== startH)) onResize(widget.id, nextW, nextH);
      onResizePreview(null);
      if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', finish);
    handle.addEventListener('pointercancel', finish);
  };

  return <div className="dashboard-widget" style={{ gridColumn: layout.x + 1 + ' / span ' + Math.min(layout.w, DASHBOARD_COLUMNS - layout.x), gridRow: layout.y + 1 + ' / span ' + layout.h }} data-dragging={dragging || undefined} data-collapsed={widget.collapsed || undefined}>
    <div className="dashboard-widget-header">
      <button className="dashboard-widget-drag" type="button" draggable aria-label={'Drag ' + description + ' widget'} title="Drag to rearrange" onDragStart={(event) => {
        event.dataTransfer.setData('text/widget-id', widget.id);
        event.dataTransfer.setData('text/plain', widget.id);
        event.dataTransfer.effectAllowed = 'move';
        const ghost = document.createElement('div');
        ghost.className = 'dashboard-drag-ghost';
        ghost.dataset.collapsed = String(widget.collapsed);
        ghost.textContent = description;
        document.body.appendChild(ghost);
        event.dataTransfer.setDragImage(ghost, 18, 18);
        window.setTimeout(() => ghost.remove(), 0);
        onDragStart(widget);
      }} onDragEnd={onDragEnd}><span className="dashboard-widget-drag-icon" aria-hidden="true" /></button>
      <span className="dashboard-widget-title">{description}</span>
      <button className="dashboard-widget-collapse" type="button" aria-label={(widget.collapsed ? 'Expand ' : 'Collapse ') + description + ' widget'} aria-expanded={!widget.collapsed} title={widget.collapsed ? 'Expand widget' : 'Collapse widget'} onClick={() => onToggle(widget.id)}><span className="dashboard-widget-chevron" aria-hidden="true" /></button>
      <button className="dashboard-widget-remove" type="button" aria-label={'Remove ' + description + ' widget'} title="Remove widget" onClick={() => onRemove(widget.id)}><span className="dashboard-widget-remove-icon" aria-hidden="true" /></button>
    </div>
    {connected ? children : <section className="feed-section" aria-label={description}><div className="book-popup-message">Waiting for the market data connection.</div></section>}
    {!widget.collapsed && <div className="dashboard-widget-resize" role="separator" aria-label={'Resize ' + description + ' widget'} title="Drag to resize" onPointerDown={startResize} />}
  </div>;
}

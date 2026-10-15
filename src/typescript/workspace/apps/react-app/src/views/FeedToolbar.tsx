import type { ReactNode } from 'react';

export function FeedToolbar({
  title,
  children,
}: {
  title: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="feed-toolbar">
      <div>
        <span className="live-indicator" />
        <span className="toolbar-title">{title}</span>
      </div>
      {children}
    </div>
  );
}

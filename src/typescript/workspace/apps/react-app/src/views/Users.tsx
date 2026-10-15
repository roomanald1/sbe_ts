import { FeedToolbar } from './FeedToolbar';
import {
  UsersModel,
  type UserGridCell,
  type UserGridRow,
} from '../models/UsersModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import React, { useEffect, useMemo } from 'react';
import { AgGridReact } from 'ag-grid-react';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
  type ColDef,
  type ICellRendererParams,
} from 'ag-grid-community';

ModuleRegistry.registerModules([ClientSideRowModelModule]);

const lastLoginFormatter = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hourCycle: 'h23',
});

const userGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: 'transparent',
});

const defaultUserColumnDef: ColDef<UserGridRow, UserGridCell> = {
  sortable: true,
  resizable: true,
};

function UserGridCellRenderer({
  value,
}: ICellRendererParams<UserGridRow, UserGridCell>) {
  if (!value) return null;
  return (
    <div className="users-grid-card">
      <span className="users-grid-id">
        U{String(value.id).padStart(5, '0')}
      </span>
      <span className="users-grid-last-login">
        {lastLoginFormatter.format(new Date(value.last_logged_in * 1000))}
      </span>
    </div>
  );
}

export function Users() {
  const model = useMemo(() => new UsersModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const columnDefs = useMemo<ColDef<UserGridRow, UserGridCell>[]>(
    () =>
      Array.from({ length: 10 }, (_, columnIndex) => ({
        colId: 'users-' + columnIndex,
        valueGetter: (params) => params.data?.users[columnIndex],
        cellRenderer: UserGridCellRenderer,
        enableCellChangeFlash: true,
        sortable: false,
        resizable: false,
        flex: 1,
        minWidth: 112,
      })),
    [],
  );
  function goToPage(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    model.goToPage();
  }
  return (
    <section className="feed-section users-panel" aria-label="Users">
      <FeedToolbar title="User directory">
        <span className="toolbar-meta">{state.count} USERS</span>
      </FeedToolbar>
      {state.isError && <p className="users-error">{state.error}</p>}
      <div className="users-grid">
        <AgGridReact<UserGridRow>
          suppressColumnMoveAnimation
          rowData={state.rowData}
          columnDefs={columnDefs}
          defaultColDef={defaultUserColumnDef}
          getRowId={(params) =>
            String(params.data.pageIndex) + ':' + String(params.data.rowIndex)
          }
          theme={userGridTheme}
          rowHeight={46}
          headerHeight={0}
          cellFlashDuration={700}
          cellFadeDuration={500}
          suppressCellFocus
        />
      </div>
      <div className="users-pagination">
        <div className="users-page-status">
          <span className="users-page-number">
            PAGE {String(state.page + 1).padStart(2, '0')}
          </span>
          {state.isFetching && (
            <span className="users-fetching is-active" role="status">
              <span className="users-fetching-dot" aria-hidden="true" />
              FETCHING
            </span>
          )}
        </div>
        <div className="users-pagination-actions">
          <button
            type="button"
            disabled={state.page === 0 || state.isFetching}
            onClick={() => model.previous()}
          >
            Previous
          </button>
          <form className="users-page-jump" onSubmit={goToPage}>
            <label htmlFor="users-page-input">GO TO</label>
            <input
              id="users-page-input"
              aria-label="Page number"
              type="number"
              min="1"
              step="1"
              required
              value={state.pageInput}
              onChange={(event) => model.setPageInput(event.target.value)}
              disabled={state.isFetching}
            />
            <button type="submit" disabled={state.isFetching}>
              Go
            </button>
          </form>
          <button
            type="button"
            disabled={!state.hasMore || state.isFetching}
            onClick={() => model.next()}
          >
            Next
          </button>
        </div>
      </div>
    </section>
  );
}

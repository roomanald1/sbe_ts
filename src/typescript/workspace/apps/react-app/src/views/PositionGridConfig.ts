import { themeQuartz, type ColDef } from 'ag-grid-community';
import type { PositionGridRow } from './PositionGridTypes';

export const positionsGridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});

export const defaultPositionColDef: ColDef<PositionGridRow> = {
  sortable: true,
  filter: true,
  resizable: true,
  floatingFilter: false,
  flex: 1,
  minWidth: 68,
};


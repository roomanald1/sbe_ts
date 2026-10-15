import { BehaviorSubject } from 'rxjs';
type User = { id: number; last_logged_in: number };
export type UserGridCell = User | undefined;
export type UserGridRow = {
  pageIndex: number;
  rowIndex: number;
  users: UserGridCell[];
};
type UserPageResponse = { data: User[]; has_more: boolean };
export type UsersState = {
  page: number;
  pageInput: string;
  rowData: UserGridRow[];
  hasMore: boolean;
  isError: boolean;
  error?: string;
  isFetching: boolean;
  count: number;
};
const emptyRows = (page: number): UserGridRow[] =>
  Array.from({ length: 10 }, (_, rowIndex) => ({
    pageIndex: page,
    rowIndex,
    users: Array<UserGridCell>(10).fill(undefined),
  }));
function validPage(value: unknown): value is UserPageResponse {
  if (!value || typeof value !== 'object') return false;
  const page = value as Partial<UserPageResponse>;
  return (
    Array.isArray(page.data) &&
    typeof page.has_more === 'boolean' &&
    page.data.every(
      (user) =>
        !!user &&
        Number.isFinite(user.id) &&
        Number.isFinite(user.last_logged_in) &&
        Number.isFinite(new Date(user.last_logged_in * 1000).getTime()),
    )
  );
}
export class UsersModel {
  readonly state$ = new BehaviorSubject<UsersState>({
    page: 0,
    pageInput: '1',
    rowData: emptyRows(0),
    hasMore: false,
    isError: false,
    isFetching: false,
    count: 0,
  });
  private timer?: ReturnType<typeof setInterval>;
  private request = 0;
  getSnapshot = (): UsersState => this.state$.value;
  subscribe = (notify: () => void): (() => void) => {
    const sub = this.state$.subscribe(notify);
    return () => sub.unsubscribe();
  };
  start(): void {
    if (this.timer) return;
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), 5000);
  }
  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
    this.request++;
  }
  setPageInput(pageInput: string): void {
    this.patch({ pageInput });
  }
  goToPage(): void {
    const number = Number(this.state$.value.pageInput);
    if (Number.isInteger(number) && number > 0) this.changePage(number - 1);
  }
  previous(): void {
    if (this.state$.value.page > 0) this.changePage(this.state$.value.page - 1);
  }
  next(): void {
    if (this.state$.value.hasMore) this.changePage(this.state$.value.page + 1);
  }
  private changePage(page: number): void {
    this.patch({ page, pageInput: String(page + 1) });
    void this.refresh();
  }
  async refresh(): Promise<void> {
    const id = ++this.request;
    const page = this.state$.value.page;
    this.patch({ isFetching: true });
    try {
      const response = await fetch('/users/' + page);
      if (!response.ok)
        throw new Error(
          'Failed to load users: ' +
            response.status +
            ' ' +
            response.statusText,
        );
      const result: unknown = await response.json();
      if (!validPage(result))
        throw new Error(
          'The users API returned invalid data. Restart the Rust API to include last_logged_in timestamps.',
        );
      if (id !== this.request) return;
      const rows = emptyRows(page);
      result.data.slice(0, 100).forEach((user, index) => {
        rows[Math.floor(index / 10)].users[index % 10] = user;
      });
      this.patch({
        rowData: rows,
        hasMore: result.has_more,
        count: result.data.length,
        isError: false,
        error: undefined,
        isFetching: false,
      });
    } catch (reason) {
      if (id === this.request)
        this.patch({
          isError: true,
          error:
            reason instanceof Error ? reason.message : 'Could not load users',
          isFetching: false,
        });
    }
  }
  private patch(patch: Partial<UsersState>): void {
    this.state$.next({ ...this.state$.value, ...patch });
  }
}

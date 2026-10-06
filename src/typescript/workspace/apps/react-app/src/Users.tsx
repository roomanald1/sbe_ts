import { useQuery } from "@tanstack/react-query";
import React from "react";
import { AgGridReact } from "ag-grid-react";
import {
    ClientSideRowModelModule,
    ModuleRegistry,
    themeQuartz,
    type ColDef,
    type ICellRendererParams,
} from "ag-grid-community";

ModuleRegistry.registerModules([ClientSideRowModelModule]);

type User = {
    id: number;
    last_logged_in: number;
};

type UserPageResponse = {
    data: User[];
    has_more: boolean;
};

type UserPage = UserPageResponse & {
    pageIndex: number;
};

type UserGridRow = {
    pageIndex: number;
    rowIndex: number;
    users: (User | undefined)[];
};

type UserGridCell = User | undefined;

function createUserGridRows(users: readonly User[], pageIndex: number): UserGridRow[] {
    const rows = Array.from({ length: 10 }, (_, rowIndex) => ({
        pageIndex,
        rowIndex,
        users: Array<UserGridCell>(10).fill(undefined),
    }));

    users.slice(0, 100).forEach((user, index) => {
        rows[Math.floor(index / 10)].users[index % 10] = user;
    });

    return rows;
}

function updateUserGridRows(
    currentRows: UserGridRow[],
    users: readonly User[],
    pageIndex: number,
): UserGridRow[] {
    if (currentRows[0]?.pageIndex !== pageIndex) {
        return createUserGridRows(users, pageIndex);
    }

    const previousUsers = new Map<number, User>();
    currentRows.forEach(row => {
        row.users.forEach(user => {
            if (user) previousUsers.set(user.id, user);
        });
    });

    const stableUsers = users.slice(0, 100).map(user => {
        const previousUser = previousUsers.get(user.id);
        return previousUser?.last_logged_in === user.last_logged_in ? previousUser : user;
    });
    const nextRows = createUserGridRows(stableUsers, pageIndex);

    return nextRows.map((nextRow, rowIndex) => {
        const previousRow = currentRows[rowIndex];
        return previousRow
            && nextRow.users.every((user, index) => user === previousRow.users[index])
            ? previousRow
            : nextRow;
    });
}

function isUserPage(value: unknown): value is UserPageResponse {
    if (typeof value !== "object" || value === null) return false;
    const candidate = value as Partial<UserPageResponse>;
    return Array.isArray(candidate.data)
        && typeof candidate.has_more === "boolean"
        && candidate.data.every(user =>
            typeof user === "object"
            && user !== null
            && typeof user.id === "number"
            && Number.isFinite(user.id)
            && typeof user.last_logged_in === "number"
            && Number.isFinite(user.last_logged_in)
            && Number.isFinite(new Date(user.last_logged_in * 1000).getTime())
        );
}

async function get_users(page: number): Promise<UserPage> {
    const response = await fetch(`/users/${page}`);
    if (!response.ok) {
        throw new Error(`Failed to load users: ${response.status} ${response.statusText}`);
    }
    const result: unknown = await response.json();
    if (!isUserPage(result)) {
        throw new Error("The users API returned invalid data. Restart the Rust API to include last_logged_in timestamps.");
    }
    return { ...result, pageIndex: page };
}

const lastLoginFormatter = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
});

const userGridTheme = themeQuartz.withParams({
    accentColor: "#8296ad",
    backgroundColor: "#0d0f12",
    borderColor: "#242a31",
    foregroundColor: "#e7e7e7",
    headerBackgroundColor: "#11151a",
    headerTextColor: "#a0a6ad",
    rowHoverColor: "transparent",
});

const emptyUsers: User[] = [];
const defaultUserColumnDef: ColDef<UserGridRow, UserGridCell> = {
    sortable: true,
    resizable: true,
};

function UserGridCellRenderer({ value }: ICellRendererParams<UserGridRow, UserGridCell>) {
    if (!value) return null;

    return (
        <div className="users-grid-card">
            <span className="users-grid-id">U{String(value.id).padStart(5, "0")}</span>
            <span className="users-grid-last-login">
                {lastLoginFormatter.format(new Date(value.last_logged_in * 1000))}
            </span>
        </div>
    );
}

export function Users() {
    const [page, setPage] = React.useState(0);
    const [pageInput, setPageInput] = React.useState("1");
    const [rowData, setRowData] = React.useState<UserGridRow[]>(() => createUserGridRows(emptyUsers, 0));
    const { isError, data, error, isFetching } = useQuery({
        queryKey: ['users', page],
        queryFn: () => get_users(page),
        placeholderData: previousData => previousData,
        refetchInterval: 5000
    });

    React.useEffect(() => {
        if (!data) return;
        setRowData(currentRows => updateUserGridRows(currentRows, data.data, data.pageIndex));
    }, [data]);

    const columnDefs = React.useMemo<ColDef<UserGridRow, UserGridCell>[]>(() =>
        Array.from({ length: 10 }, (_, columnIndex) => ({
            colId: `users-${columnIndex}`,
            valueGetter: params => params.data?.users[columnIndex],
            cellRenderer: UserGridCellRenderer,
            enableCellChangeFlash: true,
            sortable: false,
            resizable: false,
            flex: 1,
            minWidth: 112,
        })),
        []);

    function goToPage(event: React.FormEvent<HTMLFormElement>) {
        event.preventDefault();
        const enteredPage = Number(pageInput);
        if (!Number.isInteger(enteredPage) || enteredPage < 1) return;

        setPage(enteredPage - 1);
        setPageInput(String(enteredPage));
    }

    function goPrevious() {
        const previousPage = Math.max(page - 1, 0);
        setPage(previousPage);
        setPageInput(String(previousPage + 1));
    }

    function goNext() {
        if (!data?.has_more) return;
        const nextPage = page + 1;
        setPage(nextPage);
        setPageInput(String(nextPage + 1));
    }

    return (
        <section className="feed-section users-panel" aria-label="Users">
            <div className="feed-toolbar">
                <div>
                    <span className="live-indicator" />
                    <span className="toolbar-title">User directory</span>
                </div>
                <span className="toolbar-meta">{data?.data.length ?? 0} USERS</span>
            </div>
            {isError && <p className="users-error">{error?.message}</p>}
            <div className="users-grid">
                <AgGridReact<UserGridRow>
                    suppressColumnMoveAnimation
                    rowData={rowData}
                    columnDefs={columnDefs}
                    defaultColDef={defaultUserColumnDef}
                    getRowId={params => `${params.data.pageIndex}:${params.data.rowIndex}`}
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
                    <span className="users-page-number">PAGE {String(page + 1).padStart(2, "0")}</span>
                    {isFetching && (
                        <span className="users-fetching is-active" role="status">
                            <span className="users-fetching-dot" aria-hidden="true" />
                            FETCHING
                        </span>
                    )}
                </div>
                <div className="users-pagination-actions">
                    <button type="button" disabled={page === 0 || isFetching} onClick={goPrevious}>Previous</button>
                    <form className="users-page-jump" onSubmit={goToPage}>
                        <label htmlFor="users-page-input">GO TO</label>
                        <input
                            id="users-page-input"
                            aria-label="Page number"
                            type="number"
                            min="1"
                            step="1"
                            required
                            value={pageInput}
                            onChange={event => setPageInput(event.target.value)}
                            disabled={isFetching}
                        />
                        <button type="submit" disabled={isFetching}>Go</button>
                    </form>
                    <button type="button" disabled={!data?.has_more || isFetching} onClick={goNext}>Next</button>
                </div>
            </div>
        </section>
    )
}
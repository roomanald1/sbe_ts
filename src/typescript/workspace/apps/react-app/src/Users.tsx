import { useQuery } from "@tanstack/react-query";
import React from "react";

type User = {
    id: number;
    last_logged_in: number;
};

type UserPage = {
    data: User[];
    has_more: boolean;
};

type LoginChanges = {
    changedUserIds: number[];
    latestLoginByUser: Map<number, number>;
};

function detectLoginChanges(
    users: readonly User[],
    previousLoginByUser: ReadonlyMap<number, number>,
): LoginChanges {
    const latestLoginByUser = new Map(previousLoginByUser);
    const changedUserIds: number[] = [];

    for (const user of users) {
        const previousLogin = previousLoginByUser.get(user.id);
        if (previousLogin !== undefined && previousLogin !== user.last_logged_in) {
            changedUserIds.push(user.id);
        }
        latestLoginByUser.set(user.id, user.last_logged_in);
    }

    return { changedUserIds, latestLoginByUser };
}

function isUserPage(value: unknown): value is UserPage {
    if (typeof value !== "object" || value === null) return false;
    const candidate = value as Partial<UserPage>;
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
    return result;
}

function formatLastLoggedIn(timestamp: number): string {
    return new Date(timestamp * 1000).toLocaleString(undefined, {
        month: "short",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hourCycle: "h23",
    });
}

export function Users() {
    const [page, setPage] = React.useState(0);
    const [pageInput, setPageInput] = React.useState("1");
    const [flashingUsers, setFlashingUsers] = React.useState<ReadonlySet<number>>(() => new Set());
    const previousLoginByUser = React.useRef(new Map<number, number>());
    const { isError, data, error, isFetching } = useQuery({
        queryKey: ['users', page],
        queryFn: () => get_users(page),
        placeholderData: previousData => previousData,
        refetchInterval: 5000
    });

    React.useEffect(() => {
        if (!data) return;

        const changes = detectLoginChanges(data.data, previousLoginByUser.current);
        previousLoginByUser.current = changes.latestLoginByUser;

        if (changes.changedUserIds.length === 0) return;

        setFlashingUsers(current => new Set([...current, ...changes.changedUserIds]));
    }, [data]);

    function clearUserFlash(userId: number) {
        setFlashingUsers(current => {
            if (!current.has(userId)) return current;
            const next = new Set(current);
            next.delete(userId);
            return next;
        });
    }

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
        <section className="users-panel" aria-label="Users">
            {isError && <p className="users-error">{error?.message}</p>}
            <ul className="users-list" aria-label={`Users on page ${page + 1}`}>
                {data?.data.map((user) => (
                    <li
                        className={`user-card${flashingUsers.has(user.id) ? " user-card-flash" : ""}`}
                        key={user.id}
                        onAnimationEnd={() => clearUserFlash(user.id)}
                    >
                        <span className="user-card-label">USER</span>
                        <span className="user-card-id">{String(user.id).padStart(5, "0")}</span>
                        <span className="user-last-login">
                            LAST SEEN
                            <time dateTime={new Date(user.last_logged_in * 1000).toISOString()}>
                                {formatLastLoggedIn(user.last_logged_in)}
                            </time>
                        </span>
                    </li>
                ))}
            </ul>
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
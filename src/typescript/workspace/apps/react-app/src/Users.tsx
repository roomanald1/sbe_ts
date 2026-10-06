import { useQuery } from "@tanstack/react-query";
import React from "react";

class User {
    constructor(id: number) {
        this.id = id;
    }
    public id: number;
}

type UserPage = {
    data: User[];
    has_more: boolean;
};

async function get_users(page: number): Promise<UserPage> {
    const response = await fetch(`/users/${page}`);
    if (!response.ok) {
        throw new Error(`Failed to load users: ${response.status} ${response.statusText}`);
    }
    return await response.json() as UserPage;
}

export function Users() {
    const [page, setPage] = React.useState(0);
    const [pageInput, setPageInput] = React.useState("1");
    const { isError, data, error, isFetching } = useQuery({
        queryKey: ['users', page],
        queryFn: () => get_users(page),
        placeholderData: previousData => previousData,
    });

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
                    <li className="user-card" key={user.id}>
                        <span className="user-card-label">USER</span>
                        <span className="user-card-id">{String(user.id).padStart(5, "0")}</span>
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
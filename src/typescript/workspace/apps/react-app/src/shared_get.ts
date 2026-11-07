const pendingGets = new Map<string, Promise<Response>>();
const cachedGets = new Map<
  string,
  { body: string; expiresAt: number; headers: [string, string][] }
>();

/** Share concurrent idempotent reads across models; each caller gets its own body. */
export function sharedGet(url: string, cacheForMs = 0): Promise<Response> {
  const cached = cachedGets.get(url);
  if (cached && cached.expiresAt > Date.now()) {
    return Promise.resolve(
      new Response(cached.body, { status: 200, headers: cached.headers }),
    );
  }
  if (cached) cachedGets.delete(url);

  let pending = pendingGets.get(url);
  if (!pending) {
    pending = fetch(url)
      .then(async (response) => {
        if (
          cacheForMs > 0 &&
          response.ok &&
          typeof response.clone === 'function'
        ) {
          cachedGets.set(url, {
            body: await response.clone().text(),
            expiresAt: Date.now() + cacheForMs,
            headers: [...response.headers.entries()],
          });
        }
        return response;
      })
      .finally(() => pendingGets.delete(url));
    pendingGets.set(url, pending);
  }
  return pending.then((response) =>
    typeof response.clone === 'function' ? response.clone() : response,
  );
}
